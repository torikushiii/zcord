use super::*;
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Weak,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
    time::SystemTime,
};

const WORKERS: usize = 4;
// One more than shown + ready: a replaced frame stays pinned until the next scene swap, and
// without a spare slot decoding stalls for that swap, capping playback near 20fps at 60Hz.
const PLAYBACK_FRAMES: usize = 3;
const IDLE: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(16);
const SWEEP: Duration = Duration::from_millis(100);
const DECODE_SLICE: usize = 8;
const CATCH_UP_BUDGET: Duration = Duration::from_millis(100);
const SPOOL_HEADER: u64 = 12;
const SPOOL_LIMIT: u64 = 256 * 1024 * 1024;
/// Catch-up slices in a row whose frame is already past, held back for a playback on a
/// caller's clock; after that one shows anyway, so a decoder that can't keep up still moves.
const MAX_STALE_SKIPS: u8 = 16;

pub(super) fn selected(spec: &ImageSpec) -> bool {
    matches!(
        spec,
        ImageSpec::Media { .. }
            | ImageSpec::Attachment { .. }
            | ImageSpec::LocalGif { .. }
            | ImageSpec::Collectible { .. }
    )
}

/// Tells playbacks apart in the decode workers; two may share one still image.
fn next_key() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn source_spec(spec: &ImageSpec) -> ImageSpec {
    match spec {
        ImageSpec::Media {
            url, width, height, ..
        } => ImageSpec::Media {
            url: url.clone(),
            width: *width,
            height: *height,
            animated: true,
        },
        _ => spec.clone(),
    }
}

struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn reserve(count: &Arc<AtomicUsize>) -> Option<Self> {
        count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < PLAYBACK_FRAMES).then_some(held + 1)
            })
            .ok()
            .map(|_| Self(count.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct Prepared {
    image: Arc<RenderImage>,
    start: Duration,
    end: Duration,
    slot: Slot,
}

struct Retired {
    image: Arc<RenderImage>,
    _slot: Option<Slot>,
}

type Retirements = Rc<RefCell<Vec<Retired>>>;

struct FrameImage {
    image: Arc<RenderImage>,
    slot: Option<Slot>,
    retired: Retirements,
}

impl Drop for FrameImage {
    fn drop(&mut self) {
        self.retired.borrow_mut().push(Retired {
            image: self.image.clone(),
            _slot: self.slot.take(),
        });
    }
}

fn drain(retired: &Retirements, cx: &mut App) {
    release_with(retired, |image| cx.drop_image(image, None));
}

fn release_with(retired: &Retirements, mut drop_image: impl FnMut(Arc<RenderImage>)) {
    for frame in retired.take() {
        drop_image(frame.image);
        // Keep the slot occupied until every window's atlas has released the image.
        drop(frame._slot);
    }
}

struct Demand {
    displays: usize,
    active_displays: usize,
    absent_since: Instant,
    paused_since: Option<Instant>,
    requested: Instant,
    generation: u64,
    queued: bool,
    target: Duration,
    ready: Option<Prepared>,
    terminal: bool,
    slots: Arc<AtomicUsize>,
    /// Set for playbacks on a caller's clock: frames already past are held back while
    /// decoding catches up, so layers sharing that clock don't each show a different moment.
    follows_clock: bool,
    stale_skips: u8,
}

impl Demand {
    fn new(now: Instant) -> Self {
        Self {
            displays: 0,
            active_displays: 0,
            absent_since: now,
            paused_since: Some(now),
            requested: now,
            generation: 0,
            queued: false,
            target: Duration::ZERO,
            ready: None,
            terminal: false,
            slots: Arc::new(AtomicUsize::new(0)),
            follows_clock: false,
            stale_skips: 0,
        }
    }

    fn idle(&self, now: Instant) -> bool {
        self.displays == 0 && now.saturating_duration_since(self.absent_since) >= IDLE
    }

    fn wanted(&self, now: Instant) -> bool {
        self.active_displays > 0 && wanted(self.requested, now)
    }

    fn reset(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.queued = false;
        self.ready = None;
        self.terminal = false;
        self.target = Duration::ZERO;
        self.stale_skips = 0;
    }
}

struct DisplayLease {
    shared: Arc<Mutex<Demand>>,
    active: bool,
}

impl DisplayLease {
    fn new(shared: &Arc<Mutex<Demand>>, active: bool) -> Self {
        let mut demand = shared.lock().unwrap_or_else(|p| p.into_inner());
        demand.displays += 1;
        if active {
            demand.active_displays += 1;
            demand.paused_since = None;
        }
        Self {
            shared: shared.clone(),
            active,
        }
    }
}

impl Drop for DisplayLease {
    fn drop(&mut self) {
        let mut shared = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        if self.active {
            shared.active_displays -= 1;
            if shared.active_displays == 0 {
                shared.paused_since = Some(now);
            }
        }
        shared.displays -= 1;
        if shared.displays == 0 {
            shared.absent_since = now;
        }
    }
}

/// Where a playback may spool its frames, and the cache that counts the file once written.
#[derive(Clone)]
struct SpoolTarget {
    file: PathBuf,
    service: Option<Arc<images::Service>>,
}

struct Playback {
    key: u64,
    spec: ImageSpec,
    kind: SourceKind,
    path: PathBuf,
    spool: Option<SpoolTarget>,
    size: u32,
    still: Rc<FrameImage>,
    current: Option<Rc<FrameImage>>,
    end: Duration,
    started: Option<Instant>,
    /// A start time set by the caller, as for profile effect layers that play in step;
    /// playback then follows it instead of pausing while unseen.
    clock: Option<Instant>,
    plays: Option<NonZeroU32>,
    shared: Arc<Mutex<Demand>>,
}

impl Playback {
    fn clocked(&self, clock: Instant, plays: Option<NonZeroU32>) -> Self {
        Self {
            key: next_key(),
            spec: self.spec.clone(),
            kind: self.kind.clone(),
            path: self.path.clone(),
            spool: self.spool.clone(),
            size: self.size,
            still: self.still.clone(),
            current: None,
            end: Duration::from(self.still.image.delay(0)),
            started: None,
            clock: Some(clock),
            plays,
            shared: Arc::new(Mutex::new(Demand {
                follows_clock: true,
                ..Demand::new(Instant::now())
            })),
        }
    }

    fn resume(&mut self, now: Instant) -> Duration {
        if let Some(clock) = self.clock {
            return now.saturating_duration_since(clock);
        }
        let paused_since = {
            let shared = self.shared.lock().unwrap_or_else(|p| p.into_inner());
            shared.paused_since
        };
        if let (Some(paused_since), Some(started)) = (paused_since, self.started.as_mut()) {
            *started += now.saturating_duration_since(paused_since);
        }
        now.saturating_duration_since(*self.started.get_or_insert(now))
    }

    fn expire(&mut self, now: Instant) -> bool {
        let mut shared = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        if (self.started.is_none() && self.current.is_none()) || !shared.idle(now) {
            return false;
        }
        shared.reset();
        self.current = None;
        self.started = None;
        self.end = Duration::from(self.still.image.delay(0));
        true
    }
}

#[derive(Default)]
struct Streams {
    entries: HashMap<gpui::ImageId, Rc<RefCell<Playback>>>,
    /// Playbacks of a registered image on a caller's clock, dropped once unseen.
    clocked: HashMap<(gpui::ImageId, Instant), Rc<RefCell<Playback>>>,
    retired: Retirements,
}

impl Streams {
    fn playbacks(&self) -> impl Iterator<Item = &Rc<RefCell<Playback>>> {
        self.entries.values().chain(self.clocked.values())
    }
}

struct StreamGlobal {
    state: Rc<RefCell<Streams>>,
    _sweep: Task<()>,
    _closed: gpui::Subscription,
    _quit: gpui::Subscription,
}

impl Global for StreamGlobal {}

pub(super) fn init(cx: &mut App) {
    let state = Rc::new(RefCell::new(Streams::default()));
    let weak = Rc::downgrade(&state);
    let task = cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(SWEEP).await;
            let Some(state) = weak.upgrade() else { break };
            cx.update(|cx| {
                let now = Instant::now();
                state.borrow_mut().clocked.retain(|_, playback| {
                    !playback
                        .borrow()
                        .shared
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .idle(now)
                });
                let state = state.borrow();
                drain(&state.retired, cx);
                let mut changed = false;
                for playback in state.playbacks() {
                    changed |= playback.borrow_mut().expire(now);
                }
                if changed {
                    cx.refresh_windows();
                }
            });
        }
    });
    let weak = Rc::downgrade(&state);
    let closed = cx.on_window_closed(move |cx, _| {
        let weak = weak.clone();
        cx.defer(move |cx| {
            let Some(state) = weak.upgrade() else { return };
            let state = state.borrow();
            for playback in state.playbacks() {
                playback.borrow_mut().expire(Instant::now() + IDLE);
            }
            drain(&state.retired, cx);
            cx.refresh_windows();
        });
    });
    let quit = {
        let weak = Rc::downgrade(&state);
        cx.on_app_quit(move |_| {
            let mut keys = Vec::new();
            if let Some(state) = weak.upgrade() {
                for playback in state.borrow().playbacks() {
                    let playback = playback.borrow();
                    if matches!(
                        playback.spec,
                        ImageSpec::Video { .. } | ImageSpec::LocalVideo { .. }
                    ) {
                        playback
                            .shared
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .reset();
                        keys.push(playback.key);
                    }
                }
            }
            let mut replies = Vec::new();
            if !keys.is_empty() {
                for worker in &pool().workers {
                    let (reply, receiver) = tokio::sync::oneshot::channel();
                    if worker
                        .send(Command::Release {
                            keys: keys.clone(),
                            reply,
                        })
                        .is_ok()
                    {
                        replies.push(receiver);
                    }
                }
            }
            async move {
                for reply in replies {
                    let _ = reply.await;
                }
            }
        })
    };
    cx.set_global(StreamGlobal {
        state,
        _sweep: task,
        _closed: closed,
        _quit: quit,
    });
}

pub(crate) fn register(
    spec: &ImageSpec,
    size: u32,
    path: PathBuf,
    spool: Option<PathBuf>,
    image: Arc<RenderImage>,
    cx: &App,
) -> usize {
    let state = &cx.global::<StreamGlobal>().state;
    let mut state = state.borrow_mut();
    let dimensions = image.size(0);
    let bytes = dimensions.width.0 as usize * dimensions.height.0 as usize * 4;
    let end = Duration::from(image.delay(0));
    let id = image.id;
    let still = Rc::new(FrameImage {
        image,
        slot: None,
        retired: state.retired.clone(),
    });
    let spool = spool.map(|file| SpoolTarget {
        file,
        service: images::download_service(cx),
    });
    state.entries.insert(
        id,
        Rc::new(RefCell::new(Playback {
            key: next_key(),
            spec: spec.clone(),
            kind: SourceKind::of(spec),
            path,
            spool,
            size,
            still,
            current: None,
            end,
            started: None,
            clock: None,
            plays: None,
            shared: Arc::new(Mutex::new(Demand::new(Instant::now()))),
        })),
    );
    bytes * (1 + PLAYBACK_FRAMES)
}

pub(super) fn forget(id: gpui::ImageId, cx: &App) -> bool {
    let Some(global) = cx.try_global::<StreamGlobal>() else {
        return false;
    };
    let mut state = global.state.borrow_mut();
    state.clocked.retain(|(image, _), _| *image != id);
    state.entries.remove(&id).is_some()
}

/// The frame of `still`'s animation to show now. With a `clock`, it plays from that start
/// for the given number of plays (`None` loops forever) instead of on its own clock.
pub(super) fn paint(
    still: &Arc<RenderImage>,
    animate: bool,
    clock: Option<(Instant, Option<NonZeroU32>)>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<RenderImage>> {
    let global = cx.try_global::<StreamGlobal>()?;
    let (playback, retired) = {
        let mut state = global.state.borrow_mut();
        let registered = state.entries.get(&still.id)?.clone();
        let playback = match clock {
            None => registered,
            Some((start, plays)) => state
                .clocked
                .entry((still.id, start))
                .or_insert_with(|| Rc::new(RefCell::new(registered.borrow().clocked(start, plays))))
                .clone(),
        };
        (playback, state.retired.clone())
    };
    let mut playback = playback.borrow_mut();
    let now = Instant::now();
    playback.expire(now);
    let enabled = animate && animations_enabled(&playback.spec) && !cx.reduce_motion();
    let active = enabled && window_active(window);
    let elapsed = active.then(|| playback.resume(now));
    let lease = Some(DisplayLease::new(&playback.shared, active));
    let mut next_wake = Wake::None;
    if let Some(elapsed) = elapsed {
        let prepared = {
            let mut shared = playback.shared.lock().unwrap_or_else(|p| p.into_inner());
            if shared
                .ready
                .as_ref()
                .is_some_and(|frame| frame.start <= elapsed)
            {
                shared.ready.take()
            } else {
                None
            }
        };
        if let Some(prepared) = prepared {
            playback.end = prepared.end;
            playback.current = Some(Rc::new(FrameImage {
                image: prepared.image,
                slot: Some(prepared.slot),
                retired: retired.clone(),
            }));
        }
        let mut shared = playback.shared.lock().unwrap_or_else(|p| p.into_inner());
        shared.requested = now;
        shared.target = elapsed.max(playback.end);
        if !shared.queued
            && !shared.terminal
            && shared.ready.is_none()
            && shared.slots.load(Ordering::Acquire) < PLAYBACK_FRAMES
        {
            shared.queued = true;
            if pool()
                .send(
                    playback.key as usize,
                    Command::Advance {
                        key: playback.key,
                        generation: shared.generation,
                        plays: playback.plays,
                        path: playback.path.clone(),
                        size: playback.size,
                        kind: playback.kind.clone(),
                        spool: playback.spool.clone(),
                        shared: playback.shared.clone(),
                    },
                )
                .is_err()
            {
                shared.queued = false;
                shared.terminal = true;
            }
        }
        if !shared.terminal || shared.ready.is_some() {
            let remaining = playback.end.saturating_sub(elapsed);
            let remaining = if shared.ready.is_some() {
                remaining
            } else {
                POLL
            };
            next_wake = wake(remaining, true, false, true);
        }
    }
    let frame = if enabled {
        playback.current.as_ref().unwrap_or(&playback.still).clone()
    } else {
        playback.still.clone()
    };
    let image = frame.image.clone();
    let anchor = still.clone();
    // GPUI copies presentation callbacks with reused scene primitives and clears them
    // only after replacing that scene. Arcs alone do not protect atlas tiles.
    pin_scene(frame, lease, anchor, window);
    cx.defer(move |cx| drain(&retired, cx));
    request_wakeup(next_wake, window, cx);
    Some(image)
}

fn pin_scene(
    frame: Rc<FrameImage>,
    lease: Option<DisplayLease>,
    anchor: Arc<RenderImage>,
    window: &mut Window,
) {
    window.on_present(move || {
        let _ = (&frame, &lease, &anchor);
    });
}

enum Command {
    Release {
        keys: Vec<u64>,
        reply: tokio::sync::oneshot::Sender<()>,
    },
    First {
        path: PathBuf,
        size: u32,
        kind: SourceKind,
        stamp: Arc<Mutex<Instant>>,
        reply: tokio::sync::oneshot::Sender<Result<Option<Arc<RenderImage>>>>,
    },
    Advance {
        key: u64,
        generation: u64,
        plays: Option<NonZeroU32>,
        path: PathBuf,
        size: u32,
        kind: SourceKind,
        spool: Option<SpoolTarget>,
        shared: Arc<Mutex<Demand>>,
    },
}

struct Pool {
    workers: Vec<mpsc::Sender<Command>>,
    next: AtomicUsize,
}

impl Pool {
    fn send(&self, key: usize, command: Command) -> Result<()> {
        self.workers[key % self.workers.len()]
            .send(command)
            .map_err(|_| anyhow::anyhow!("animation worker unavailable"))
    }
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let mut workers = Vec::new();
        for index in 0..WORKERS {
            let (sender, receiver) = mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name(format!("media-stream-{index}"))
                .spawn(move || worker(receiver));
            if spawned.is_err() {
                log::warn!("Could not start animation worker {index}");
            }
            workers.push(sender);
        }
        Pool {
            workers,
            next: AtomicUsize::new(0),
        }
    })
}

pub(super) async fn first(
    path: PathBuf,
    size: u32,
    stamp: Arc<Mutex<Instant>>,
) -> Result<Option<Arc<RenderImage>>> {
    first_source(path, size, stamp, SourceKind::Image).await
}

#[cfg(windows)]
pub(crate) async fn first_video(
    path: PathBuf,
    size: u32,
    stamp: Arc<Mutex<Instant>>,
) -> Result<Option<Arc<RenderImage>>> {
    first_source(path, size, stamp, SourceKind::Video).await
}

async fn first_source(
    path: PathBuf,
    size: u32,
    stamp: Arc<Mutex<Instant>>,
    kind: SourceKind,
) -> Result<Option<Arc<RenderImage>>> {
    let (reply, receiver) = tokio::sync::oneshot::channel();
    let pool = pool();
    pool.send(
        pool.next.fetch_add(1, Ordering::Relaxed),
        Command::First {
            path,
            size,
            kind,
            stamp,
            reply,
        },
    )?;
    receiver.await.context("animation worker stopped")?
}

struct Session {
    core: Core,
    generation: u64,
    shared: Weak<Mutex<Demand>>,
}

fn worker(receiver: mpsc::Receiver<Command>) {
    let mut sessions: HashMap<u64, Session> = HashMap::new();
    loop {
        sweep_sessions(&mut sessions, Instant::now());
        let command = match receiver.recv_timeout(SWEEP) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match command {
            Command::Release { keys, reply } => {
                for key in keys {
                    sessions.remove(&key);
                }
                let _ = reply.send(());
            }
            Command::First {
                path,
                size,
                kind,
                stamp,
                reply,
            } => {
                let result = if still_wanted(&stamp) {
                    Core::open_kind(path, size, kind, None).and_then(|mut core| {
                        let frame = core.step(Duration::ZERO, || still_wanted(&stamp))?;
                        if !still_wanted(&stamp) {
                            return Ok(None);
                        }
                        Ok(Some(frame.context("animation has no valid first frame")?.0))
                    })
                } else {
                    Ok(None)
                };
                let _ = reply.send(result);
            }
            Command::Advance {
                key,
                generation,
                plays,
                path,
                size,
                kind,
                spool,
                shared,
            } => {
                advance(
                    &mut sessions,
                    key,
                    generation,
                    plays,
                    path,
                    size,
                    kind,
                    spool,
                    &shared,
                );
            }
        }
    }
}

fn sweep_sessions(sessions: &mut HashMap<u64, Session>, now: Instant) {
    sessions.retain(|_, session| {
        session.shared.upgrade().is_some_and(|shared| {
            let shared = shared.lock().unwrap_or_else(|p| p.into_inner());
            shared.generation == session.generation && !shared.idle(now)
        })
    });
}

fn advance(
    sessions: &mut HashMap<u64, Session>,
    id: u64,
    generation: u64,
    plays: Option<NonZeroU32>,
    path: PathBuf,
    size: u32,
    kind: SourceKind,
    spool: Option<SpoolTarget>,
    shared: &Arc<Mutex<Demand>>,
) {
    let (target, slot) = {
        let mut demand = shared.lock().unwrap_or_else(|p| p.into_inner());
        if generation != demand.generation {
            return;
        }
        if !demand.queued
            || demand.terminal
            || !demand.wanted(Instant::now())
            || demand.ready.is_some()
        {
            demand.queued = false;
            return;
        }
        let Some(slot) = Slot::reserve(&demand.slots) else {
            demand.queued = false;
            return;
        };
        (demand.target, slot)
    };
    let result = (|| {
        if sessions
            .get(&id)
            .is_some_and(|session| session.generation != generation)
        {
            sessions.remove(&id);
        }
        if !sessions.contains_key(&id) {
            sessions.insert(
                id,
                Session {
                    core: Core::open_kind(
                        path.clone(),
                        size,
                        kind,
                        spool.as_ref().map(|spool| spool.file.clone()),
                    )?
                    .with_plays(plays),
                    generation,
                    shared: Arc::downgrade(shared),
                },
            );
        }
        let session = sessions.get_mut(&id).context("missing animation session")?;
        let frame = session.core.step(target, || {
            let demand = shared.lock().unwrap_or_else(|p| p.into_inner());
            demand.generation == generation && demand.wanted(Instant::now())
        })?;
        let spool_file = session.core.wants_spool();
        Ok::<_, anyhow::Error>((frame, session.core.terminal, spool_file))
    })();
    let mut demand = shared.lock().unwrap_or_else(|p| p.into_inner());
    if generation != demand.generation {
        sessions.remove(&id);
        return;
    }
    demand.queued = false;
    match result {
        Ok((frame, terminal, spool_file)) => {
            if let Some(file) = spool_file
                && spool_pool()
                    .send(SpoolJob {
                        source: path,
                        size,
                        file,
                        service: spool.and_then(|spool| spool.service),
                        shared: Arc::downgrade(shared),
                    })
                    .is_err()
            {
                log::debug!("Could not queue GIF spooling");
            }
            demand.terminal = terminal;
            if terminal {
                sessions.remove(&id);
            }
            let stale = !terminal
                && demand.follows_clock
                && demand.stale_skips < MAX_STALE_SKIPS
                && frame.as_ref().is_some_and(|(_, _, end)| *end <= target);
            if stale {
                demand.stale_skips += 1;
            } else if demand.wanted(Instant::now()) {
                demand.stale_skips = 0;
                demand.ready = frame.map(|(image, start, end)| Prepared {
                    image,
                    start,
                    end,
                    slot,
                });
            }
        }
        Err(_) => {
            demand.terminal = true;
            sessions.remove(&id);
        }
    }
}

#[derive(Clone)]
enum SourceKind {
    Image,
    #[cfg(windows)]
    Video,
    #[cfg(test)]
    Fake(Arc<dyn Fn() -> Result<Box<dyn FrameSource>> + Send + Sync>),
}

impl SourceKind {
    fn of(spec: &ImageSpec) -> Self {
        #[cfg(windows)]
        if matches!(spec, ImageSpec::Video { .. } | ImageSpec::LocalVideo { .. }) {
            return Self::Video;
        }
        let _ = spec;
        Self::Image
    }
}

pub(crate) enum SourceRead {
    Frame(Frame),
    End,
}

pub(crate) trait FrameSource {
    fn read(&mut self) -> Result<SourceRead>;
    fn restart(&mut self) -> Result<()>;
    fn max_frames(&self) -> usize;
    fn target(&self) -> Option<(u32, u32)> {
        None
    }
    fn delay(&self, frame: &Frame) -> Duration {
        Duration::from(frame.delay())
    }
    fn render(&self, frame: Frame) -> Frame {
        frame
    }
}

struct ImageFrames {
    path: PathBuf,
    frames: Frames<'static>,
    dimensions: (u32, u32),
    target: (u32, u32),
}

impl FrameSource for ImageFrames {
    fn read(&mut self) -> Result<SourceRead> {
        match self.frames.next() {
            Some(frame) => {
                let frame = frame?;
                ensure!(
                    frame.buffer().dimensions() == self.dimensions,
                    "frame size changed"
                );
                Ok(SourceRead::Frame(frame))
            }
            None => Ok(SourceRead::End),
        }
    }

    fn restart(&mut self) -> Result<()> {
        let (dimensions, frames) = source(&self.path)?;
        ensure!(
            dimensions == self.dimensions,
            "animation dimensions changed"
        );
        self.frames = frames;
        Ok(())
    }

    fn max_frames(&self) -> usize {
        MAX_FRAMES
    }

    fn target(&self) -> Option<(u32, u32)> {
        Some(self.target)
    }

    fn delay(&self, frame: &Frame) -> Duration {
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        if u64::from(numerator) < 20 * u64::from(denominator) {
            Duration::from_millis(100)
        } else {
            Duration::from(frame.delay())
        }
    }

    fn render(&self, frame: Frame) -> Frame {
        process_frame(frame, self.target)
    }
}

struct SpoolFrames {
    path: PathBuf,
    reader: BufReader<File>,
    width: u32,
    height: u32,
}

impl SpoolFrames {
    fn open(path: &Path, expected: (u32, u32)) -> Result<Self> {
        let file = File::options().read(true).write(true).open(path)?;
        file.set_modified(SystemTime::now())?;
        let mut reader = BufReader::new(file);
        let mut header = [0; SPOOL_HEADER as usize];
        reader.read_exact(&mut header)?;
        ensure!(&header[..4] == b"ZCSP", "invalid spool file");
        let width = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let height = u32::from_le_bytes(header[8..12].try_into().unwrap());
        ensure!((width, height) == expected, "spool dimensions changed");
        Ok(Self {
            path: path.to_path_buf(),
            reader,
            width,
            height,
        })
    }
}

impl FrameSource for SpoolFrames {
    fn read(&mut self) -> Result<SourceRead> {
        let mut delay = [0; 4];
        let mut filled = 0;
        while filled < delay.len() {
            match self.reader.read(&mut delay[filled..]) {
                Ok(0) if filled == 0 => return Ok(SourceRead::End),
                Ok(0) => anyhow::bail!("truncated spool"),
                Ok(read) => filled += read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
        let delay = u32::from_le_bytes(delay);
        let mut pixels = vec![0; self.width as usize * self.height as usize * 4];
        self.reader
            .read_exact(&mut pixels)
            .with_context(|| format!("truncated spool {}", self.path.display()))?;
        let pixels =
            RgbaImage::from_raw(self.width, self.height, pixels).context("invalid spool frame")?;
        Ok(SourceRead::Frame(Frame::from_parts(
            pixels,
            0,
            0,
            Delay::from_numer_denom_ms(delay, 1),
        )))
    }

    fn restart(&mut self) -> Result<()> {
        self.reader.seek(SeekFrom::Start(SPOOL_HEADER))?;
        Ok(())
    }

    fn max_frames(&self) -> usize {
        MAX_FRAMES
    }
}

/// Writes the spool and returns its size.
fn write_spool(
    source: &Path,
    size: u32,
    file: &Path,
    keep_going: impl Fn() -> bool,
) -> Result<u64> {
    let temp = file.with_extension("spool.tmp");
    let result = write_spool_frames(source, size, &temp, keep_going).and_then(|written| {
        std::fs::rename(&temp, file).context("could not publish spool")?;
        Ok(written)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn write_spool_frames(
    source: &Path,
    size: u32,
    file: &Path,
    keep_going: impl Fn() -> bool,
) -> Result<u64> {
    let (dimensions, frames) = self::source(source)?;
    let target = output_size(dimensions.0, dimensions.1, size, 1 + PLAYBACK_FRAMES)?;
    let frame_bytes = u64::from(target.0) * u64::from(target.1) * 4;
    let mut writer = BufWriter::new(File::create(file)?);
    writer.write_all(b"ZCSP")?;
    writer.write_all(&target.0.to_le_bytes())?;
    writer.write_all(&target.1.to_le_bytes())?;
    let mut total = SPOOL_HEADER;
    let mut count = 0;
    for frame in frames {
        ensure!(keep_going(), "spool cancelled");
        ensure!(count < MAX_FRAMES, "too many animation frames");
        ensure!(total + 4 + frame_bytes <= SPOOL_LIMIT, "spool too large");
        let frame = process_frame(frame?, target);
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = (numerator / denominator).max(1);
        writer.write_all(&delay.to_le_bytes())?;
        writer.write_all(frame.buffer().as_raw())?;
        total += 4 + frame_bytes;
        count += 1;
    }
    writer.flush()?;
    Ok(total)
}

struct SpoolJob {
    source: PathBuf,
    size: u32,
    file: PathBuf,
    service: Option<Arc<images::Service>>,
    shared: Weak<Mutex<Demand>>,
}

fn spool_pool() -> &'static mpsc::Sender<SpoolJob> {
    static POOL: OnceLock<mpsc::Sender<SpoolJob>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<SpoolJob>();
        let spawned = std::thread::Builder::new()
            .name("gif-spool".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let shared = job.shared;
                    let keep_going = move || {
                        shared.upgrade().is_some_and(|shared| {
                            !shared
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .idle(Instant::now())
                        })
                    };
                    match write_spool(&job.source, job.size, &job.file, keep_going) {
                        // Counted like decoded frames, so the frame cache stays in budget.
                        Ok(written) => {
                            if let Some(service) = job.service {
                                service.record_frames_write(written);
                            }
                        }
                        Err(error) => log::debug!("Could not spool GIF frames: {error}"),
                    }
                }
            });
        if spawned.is_err() {
            log::warn!("Could not start GIF spool worker");
        }
        sender
    })
}

struct Core {
    source: Box<dyn FrameSource>,
    count: usize,
    position: Duration,
    base: Duration,
    /// How many times to play before holding the last frame; `None` loops forever.
    plays: Option<NonZeroU32>,
    cycles: u64,
    terminal: bool,
    spool: Option<PathBuf>,
    spool_requested: bool,
    spent: Duration,
    #[cfg(test)]
    decoded: usize,
}

fn source(path: &Path) -> Result<((u32, u32), Frames<'static>)> {
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.len() <= MAX_SOURCE_BYTES,
        "animation source too large"
    );
    let reader = ImageReader::new(BufReader::new(file)).with_guessed_format()?;
    let format = reader.format().context("unknown animation format")?;
    let reader = reader.into_inner();
    let (dimensions, frames) = match format {
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(reader)?;
            decoder.set_limits(limits())?;
            (decoder.dimensions(), decoder.into_frames())
        }
        ImageFormat::Png => apng::frames(reader)?,
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(reader)?;
            decoder.set_limits(limits())?;
            let dimensions = decoder.dimensions();
            let frames = if decoder.has_animation() {
                decoder.into_frames()
            } else {
                still_frames(decoder)?
            };
            (dimensions, frames)
        }
        _ => {
            let mut reader = ImageReader::with_format(reader, format);
            reader.limits(limits());
            let decoder = reader.into_decoder()?;
            (decoder.dimensions(), still_frames(decoder)?)
        }
    };
    valid_dimensions(dimensions)?;
    Ok((dimensions, frames))
}

impl Core {
    fn open(path: PathBuf, size: u32) -> Result<Self> {
        let (dimensions, frames) = source(&path)?;
        let target = output_size(dimensions.0, dimensions.1, size, 1 + PLAYBACK_FRAMES)?;
        Ok(Self::from_source(Box::new(ImageFrames {
            path,
            frames,
            dimensions,
            target,
        })))
    }

    fn open_kind(
        path: PathBuf,
        size: u32,
        kind: SourceKind,
        spool: Option<PathBuf>,
    ) -> Result<Self> {
        match kind {
            SourceKind::Image => {
                let mut core = Self::open(path, size)?;
                let Some(file) = spool else {
                    return Ok(core);
                };
                if file.exists()
                    && let Some(target) = core.source.target()
                {
                    match SpoolFrames::open(&file, target) {
                        Ok(source) => return Ok(Self::from_source(Box::new(source))),
                        Err(_) => {
                            let _ = std::fs::remove_file(&file);
                        }
                    }
                }
                core.spool = Some(file);
                Ok(core)
            }
            #[cfg(windows)]
            SourceKind::Video => {
                let reader = crate::media::video::Reader::open(&path, size)?;
                let rate = reader.rate();
                Ok(Self::from_source(Box::new(
                    crate::media::video::animation::VideoFrames::new(reader, rate),
                )))
            }
            #[cfg(test)]
            SourceKind::Fake(open) => Ok(Self::from_source(open()?)),
        }
    }

    fn from_source(source: Box<dyn FrameSource>) -> Self {
        Self {
            source,
            count: 0,
            position: Duration::ZERO,
            base: Duration::ZERO,
            plays: None,
            cycles: 0,
            terminal: false,
            spool: None,
            spool_requested: false,
            spent: Duration::ZERO,
            #[cfg(test)]
            decoded: 0,
        }
    }

    fn with_plays(mut self, plays: Option<NonZeroU32>) -> Self {
        self.plays = plays;
        self
    }

    fn wants_spool(&mut self) -> Option<PathBuf> {
        if self.spool_requested || self.count < 4 || self.spent <= self.position {
            return None;
        }
        let file = self.spool.clone()?;
        self.spool_requested = true;
        Some(file)
    }

    fn step(
        &mut self,
        target: Duration,
        wanted: impl Fn() -> bool,
    ) -> Result<Option<(Arc<RenderImage>, Duration, Duration)>> {
        if self.terminal || !wanted() {
            return Ok(None);
        }
        let started = Instant::now();
        let mut latest = None;
        // Whether `latest` has been through its source's `render` already.
        let mut rendered = false;
        for _ in 0..DECODE_SLICE {
            if !wanted() {
                break;
            }
            let reading = Instant::now();
            let read = self.source.read();
            if self.spool.is_some() {
                self.spent += reading.elapsed();
            }
            let frame = match read {
                Ok(SourceRead::Frame(frame)) if self.count < self.source.max_frames() => frame,
                Ok(SourceRead::End) if self.count > 1 => {
                    let cycle = self.position;
                    let loops = target.saturating_sub(self.base).as_nanos() / cycle.as_nanos();
                    self.cycles = self
                        .cycles
                        .saturating_add(u64::try_from(loops.max(1)).unwrap_or(u64::MAX));
                    if self
                        .plays
                        .is_some_and(|plays| self.cycles >= u64::from(plays.get()))
                    {
                        // Played out: the last frame shown stays.
                        self.terminal = true;
                        break;
                    }
                    self.base += Duration::from_nanos(
                        (cycle.as_nanos() * loops.max(1)).min(u128::from(u64::MAX)) as u64,
                    );
                    let mut swapped = false;
                    if let Some(file) = self.spool.as_ref()
                        && file.exists()
                        && let Some(expected) = self.source.target()
                        && let Ok(spool) = SpoolFrames::open(file, expected)
                    {
                        // A frame read from the old source needs that source's resize and
                        // colour order; the spool's frames are stored done.
                        if !rendered {
                            latest = latest
                                .map(|(frame, start, end)| (self.source.render(frame), start, end));
                            rendered = true;
                        }
                        self.source = Box::new(spool);
                        swapped = true;
                    }
                    if swapped {
                        self.spool = None;
                    } else if self.source.restart().is_err() {
                        self.terminal = true;
                        break;
                    }
                    self.count = 0;
                    self.position = Duration::ZERO;
                    self.spent = Duration::ZERO;
                    continue;
                }
                _ => {
                    self.terminal = true;
                    break;
                }
            };
            self.count += 1;
            #[cfg(test)]
            {
                self.decoded += 1;
            }
            let delay = self.source.delay(&frame).max(Duration::from_nanos(1));
            let start = self.base + self.position;
            self.position += delay;
            let end = self.base + self.position;
            latest = Some((frame, start, end));
            rendered = false;
            if end > target || started.elapsed() >= CATCH_UP_BUDGET {
                break;
            }
        }
        let Some((frame, start, end)) = latest else {
            return Ok(None);
        };
        let rendering = Instant::now();
        let frame = if rendered {
            frame
        } else {
            self.source.render(frame)
        };
        if self.spool.is_some() {
            self.spent += rendering.elapsed();
        }
        Ok(Some((Arc::new(RenderImage::new(vec![frame])), start, end)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Entity, Render, TestAppContext, div, prelude::*};
    use image::{
        Rgba, RgbaImage,
        codecs::gif::{GifEncoder, Repeat},
    };

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "zcord-stream-{}-{}.gif",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::write(&path, bytes).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn gif(count: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).unwrap();
            for index in 0..count {
                let delay = [10, 20, 30, 100][index % 4];
                encoder
                    .encode_frame(Frame::from_parts(
                        RgbaImage::from_pixel(8, 4, Rgba([index as u8, 20, 200, 255])),
                        0,
                        0,
                        Delay::from_numer_denom_ms(delay, 1),
                    ))
                    .unwrap();
            }
        }
        bytes
    }

    fn gif_colored(count: usize, color: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            encoder.set_repeat(Repeat::Infinite).unwrap();
            for _ in 0..count {
                encoder
                    .encode_frame(Frame::from_parts(
                        RgbaImage::from_pixel(8, 4, Rgba(color)),
                        0,
                        0,
                        Delay::from_numer_denom_ms(100, 1),
                    ))
                    .unwrap();
            }
        }
        bytes
    }

    struct TempFile(PathBuf);

    impl TempFile {
        fn new(extension: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "zcord-spool-{}-{}.{extension}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            Self(path)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("spool.tmp"));
        }
    }

    fn spec() -> ImageSpec {
        ImageSpec::Media {
            url: "https://media.tenor.com/test.gif".into(),
            width: 8,
            height: 4,
            animated: true,
        }
    }

    fn still() -> Arc<RenderImage> {
        Arc::new(RenderImage::new(vec![process_frame(
            Frame::new(RgbaImage::new(8, 4)),
            (8, 4),
        )]))
    }

    fn playback(path: PathBuf, retired: &Retirements) -> Playback {
        Playback {
            key: next_key(),
            spec: spec(),
            kind: SourceKind::Image,
            path,
            spool: None,
            size: 8,
            still: Rc::new(FrameImage {
                image: still(),
                slot: None,
                retired: retired.clone(),
            }),
            current: None,
            end: Duration::from_millis(100),
            started: Some(Instant::now()),
            clock: None,
            plays: None,
            shared: Arc::new(Mutex::new(Demand::new(Instant::now()))),
        }
    }

    fn request(playback: &Playback, sessions: &mut HashMap<u64, Session>, target: Duration) {
        let generation = {
            let mut shared = playback.shared.lock().unwrap();
            shared.queued = true;
            shared.target = target;
            shared.requested = Instant::now();
            shared.generation
        };
        advance(
            sessions,
            playback.key,
            generation,
            playback.plays,
            playback.path.clone(),
            playback.size,
            playback.kind.clone(),
            playback.spool.clone(),
            &playback.shared,
        );
    }

    fn adopt(playback: &mut Playback, retired: &Retirements) {
        let prepared = playback.shared.lock().unwrap().ready.take().unwrap();
        playback.end = prepared.end;
        playback.current = Some(Rc::new(FrameImage {
            image: prepared.image,
            slot: Some(prepared.slot),
            retired: retired.clone(),
        }));
    }

    #[derive(Default)]
    struct VideoStats {
        opened: AtomicUsize,
        closed: AtomicUsize,
        reads: AtomicUsize,
        copied: AtomicUsize,
        loops: AtomicUsize,
        opened_on: Mutex<Option<std::thread::ThreadId>>,
        closed_on: Mutex<Option<std::thread::ThreadId>>,
    }

    struct FakeVideo {
        index: usize,
        empty: bool,
        count: usize,
        error_at: Option<usize>,
        fail_restart: bool,
        ticks: i64,
        stats: Arc<VideoStats>,
    }

    impl Drop for FakeVideo {
        fn drop(&mut self) {
            self.stats.closed.fetch_add(1, Ordering::Relaxed);
            *self.stats.closed_on.lock().unwrap() = Some(std::thread::current().id());
        }
    }

    impl crate::media::video::animation::VideoReader for FakeVideo {
        fn read(&mut self, pixels: bool) -> Result<crate::media::video::animation::VideoRead> {
            use crate::media::video::animation::VideoRead;
            self.stats.reads.fetch_add(1, Ordering::Relaxed);
            if self.empty {
                self.empty = false;
                return Ok(VideoRead::Empty);
            }
            ensure!(Some(self.index) != self.error_at, "fake video decode error");
            if self.index >= self.count {
                return Ok(VideoRead::End);
            }
            let image = pixels.then(|| {
                self.stats.copied.fetch_add(1, Ordering::Relaxed);
                RgbaImage::from_pixel(8, 4, Rgba([self.index as u8, 11, 22, 255]))
            });
            let timestamp = self.index as i64 * self.ticks;
            self.index += 1;
            Ok(VideoRead::Sample {
                timestamp,
                duration: Some(self.ticks),
                pixels: image,
            })
        }

        fn restart(&mut self) -> Result<()> {
            ensure!(!self.fail_restart, "fake video reopen error");
            self.stats.loops.fetch_add(1, Ordering::Relaxed);
            self.index = 0;
            self.empty = true;
            Ok(())
        }
    }

    fn fake_video(
        count: usize,
        error_at: Option<usize>,
        rate: Option<(u32, u32)>,
        fail_restart: bool,
    ) -> (SourceKind, Arc<VideoStats>) {
        let stats = Arc::new(VideoStats::default());
        let captured = stats.clone();
        let kind = SourceKind::Fake(Arc::new(move || {
            captured.opened.fetch_add(1, Ordering::Relaxed);
            *captured.opened_on.lock().unwrap() = Some(std::thread::current().id());
            let reader = FakeVideo {
                index: 0,
                empty: true,
                count,
                error_at,
                fail_restart,
                ticks: rate.map_or(333_333, |(n, d)| {
                    (10_000_000u64 * u64::from(d) / u64::from(n)).max(1) as i64
                }),
                stats: captured.clone(),
            };
            Ok(Box::new(crate::media::video::animation::VideoFrames::new(
                reader, rate,
            )))
        }));
        (kind, stats)
    }

    #[test]
    fn video_source_loops_with_stretched_delays_and_skips_ahead_without_gif_delay_rules() {
        let (kind, stats) = fake_video(6, None, Some((30, 1)), false);
        let mut core = Core::open_kind(PathBuf::new(), 8, kind, None).unwrap();
        let interval = Duration::from_nanos(66_666_600);
        let mut target = Duration::ZERO;
        for index in [0, 2, 4, 0, 2, 4] {
            let (frame, start, end) = core.step(target, || true).unwrap().unwrap();
            assert_eq!(start, target);
            assert_eq!(end - start, interval);
            assert_eq!(Duration::from(frame.delay(0)), interval);
            assert_eq!(&frame.as_bytes(0).unwrap()[..4], &[index, 11, 22, 255]);
            target = end;
        }
        assert_eq!(stats.copied.load(Ordering::Relaxed), 6);
        assert_eq!(stats.loops.load(Ordering::Relaxed), 1);
        let far = Duration::from_secs(3600);
        let mut caught_up = false;
        for _ in 0..8 {
            let before = core.decoded;
            if let Some((_, start, end)) = core.step(far, || true).unwrap() {
                assert_eq!(end - start, interval);
                caught_up = end > far;
            }
            assert!(core.decoded - before <= DECODE_SLICE);
            if caught_up {
                break;
            }
        }
        assert!(caught_up);
    }

    #[test]
    fn video_uses_shared_frame_slots_and_never_reads_ahead_of_the_bound() {
        let (kind, stats) = fake_video(1000, None, Some((30, 1)), false);
        let retired = Rc::default();
        let mut playback = playback(PathBuf::new(), &retired);
        playback.kind = kind;
        playback.end = Duration::ZERO;
        let _lease = DisplayLease::new(&playback.shared, true);
        let slots = playback.shared.lock().unwrap().slots.clone();
        let mut sessions = HashMap::new();
        let mut released = std::collections::HashSet::new();
        let mut handed = std::collections::HashSet::new();
        for _ in 0..650 {
            request(&playback, &mut sessions, playback.end);
            assert!(playback.shared.lock().unwrap().ready.is_some());
            let reads = stats.reads.load(Ordering::Relaxed);
            request(&playback, &mut sessions, playback.end);
            assert_eq!(stats.reads.load(Ordering::Relaxed), reads);
            let old_scene = playback.current.clone();
            adopt(&mut playback, &retired);
            handed.insert(playback.current.as_ref().unwrap().image.id);
            assert!(slots.load(Ordering::Acquire) <= PLAYBACK_FRAMES);
            if old_scene.is_some() {
                request(&playback, &mut sessions, playback.end);
                assert!(playback.shared.lock().unwrap().ready.is_some());
                let reads = stats.reads.load(Ordering::Relaxed);
                request(&playback, &mut sessions, playback.end);
                assert_eq!(stats.reads.load(Ordering::Relaxed), reads);
            }
            drop(old_scene);
            release_with(&retired, |image| assert!(released.insert(image.id)));
        }
        playback.shared.lock().unwrap().ready = None;
        playback.current = None;
        release_with(&retired, |image| assert!(released.insert(image.id)));
        assert_eq!(slots.load(Ordering::Acquire), 0);
        assert_eq!(released, handed);
        assert!(stats.loops.load(Ordering::Relaxed) >= 2);
    }

    #[test]
    fn video_end_error_and_failed_reopen_keep_the_last_good_frame() {
        for (count, error, reopen_error, expected) in [
            (1, None, false, 0),
            (6, Some(3), false, 2),
            (6, None, true, 4),
        ] {
            let (kind, stats) = fake_video(count, error, Some((30, 1)), reopen_error);
            let retired = Rc::default();
            let mut playback = playback(PathBuf::new(), &retired);
            playback.kind = kind;
            playback.end = Duration::ZERO;
            let _lease = DisplayLease::new(&playback.shared, true);
            let mut sessions = HashMap::new();
            for _ in 0..8 {
                request(&playback, &mut sessions, playback.end);
                if playback.shared.lock().unwrap().ready.is_some() {
                    adopt(&mut playback, &retired);
                    release_with(&retired, |_| {});
                }
                if playback.shared.lock().unwrap().terminal {
                    break;
                }
            }
            assert!(playback.shared.lock().unwrap().terminal);
            let last = playback.current.as_ref().unwrap().image.id;
            assert_eq!(
                playback
                    .current
                    .as_ref()
                    .unwrap()
                    .image
                    .as_bytes(0)
                    .unwrap()[0],
                expected
            );
            request(&playback, &mut sessions, playback.end);
            assert_eq!(playback.current.as_ref().unwrap().image.id, last);
            assert!(sessions.is_empty());
            assert_eq!(stats.closed.load(Ordering::Relaxed), 1);
        }
        let (kind, _) = fake_video(0, None, Some((30, 1)), false);
        let mut core = Core::open_kind(PathBuf::new(), 8, kind, None).unwrap();
        assert!(core.step(Duration::ZERO, || true).unwrap().is_none());
        assert!(core.terminal);
    }

    #[test]
    fn video_idle_drop_reopens_the_source_and_inactive_scenes_remain_frozen() {
        let (kind, stats) = fake_video(100, None, Some((30, 1)), false);
        let retired = Rc::default();
        let mut playback = playback(PathBuf::new(), &retired);
        playback.kind = kind;
        playback.end = Duration::ZERO;
        let still_id = playback.still.image.id;
        let active = DisplayLease::new(&playback.shared, true);
        let mut sessions = HashMap::new();
        request(&playback, &mut sessions, playback.end);
        adopt(&mut playback, &retired);
        let frame_id = playback.current.as_ref().unwrap().image.id;
        let inactive = DisplayLease::new(&playback.shared, false);
        drop(active);
        let reads = stats.reads.load(Ordering::Relaxed);
        request(&playback, &mut sessions, playback.end);
        assert_eq!(stats.reads.load(Ordering::Relaxed), reads);
        assert!(!playback.expire(Instant::now() + IDLE * 10));
        assert_eq!(playback.current.as_ref().unwrap().image.id, frame_id);
        drop(inactive);
        let absent = playback.shared.lock().unwrap().absent_since;
        assert!(playback.expire(absent + IDLE));
        sweep_sessions(&mut sessions, absent + IDLE);
        release_with(&retired, |_| {});
        assert_eq!(stats.closed.load(Ordering::Relaxed), 1);
        assert!(playback.current.is_none());
        assert_eq!(playback.still.image.id, still_id);
        let _active = DisplayLease::new(&playback.shared, true);
        request(&playback, &mut sessions, playback.end);
        adopt(&mut playback, &retired);
        assert_eq!(stats.opened.load(Ordering::Relaxed), 2);
        assert_eq!(
            playback
                .current
                .as_ref()
                .unwrap()
                .image
                .as_bytes(0)
                .unwrap()[0],
            2
        );
    }

    #[test]
    fn video_unknown_rate_uses_timestamps_to_cap_conversion_and_uploads() {
        let (kind, stats) = fake_video(6, None, None, false);
        let mut core = Core::open_kind(PathBuf::new(), 8, kind, None).unwrap();
        let (frame, start, end) = core.step(Duration::ZERO, || true).unwrap().unwrap();
        assert_eq!(start, Duration::ZERO);
        assert_eq!(end, Duration::from_nanos(99_999_900));
        assert_eq!(frame.as_bytes(0).unwrap()[0], 0);
        assert_eq!(stats.reads.load(Ordering::Relaxed), 4);
        assert_eq!(stats.copied.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn video_fps_cap_retains_the_existing_rate_based_skip_intervals() {
        for (rate, samples, nanos) in [
            ((30, 1), 2, 66_666_600),
            ((60, 1), 4, 66_666_400),
            ((24, 1), 2, 83_333_200),
            ((16, 1), 2, 125_000_000),
            ((30_000, 1001), 2, 66_733_200),
        ] {
            let (kind, stats) = fake_video(100, None, Some(rate), false);
            let mut core = Core::open_kind(PathBuf::new(), 8, kind, None).unwrap();
            let (_, start, end) = core.step(Duration::ZERO, || true).unwrap().unwrap();
            assert_eq!(end - start, Duration::from_nanos(nanos));
            assert_eq!(stats.reads.load(Ordering::Relaxed), samples + 1);
            assert_eq!(stats.copied.load(Ordering::Relaxed), 1);
        }
    }

    #[test]
    fn video_shutdown_acknowledges_decoder_destruction_on_its_own_worker() {
        let (kind, stats) = fake_video(100, None, Some((30, 1)), false);
        let id = still().id;
        let shared = Arc::new(Mutex::new(Demand::new(Instant::now())));
        let _lease = DisplayLease::new(&shared, true);
        shared.lock().unwrap().queued = true;
        pool()
            .send(
                id.0,
                Command::Advance {
                    key: id.0 as u64,
                    generation: 0,
                    plays: None,
                    path: PathBuf::new(),
                    size: 8,
                    kind,
                    spool: None,
                    shared: shared.clone(),
                },
            )
            .unwrap();
        let (reply, receiver) = tokio::sync::oneshot::channel();
        pool()
            .send(
                id.0,
                Command::Release {
                    keys: vec![id.0 as u64],
                    reply,
                },
            )
            .unwrap();
        receiver.blocking_recv().unwrap();
        assert_eq!(stats.opened.load(Ordering::Relaxed), 1);
        assert_eq!(stats.closed.load(Ordering::Relaxed), 1);
        assert_eq!(
            *stats.opened_on.lock().unwrap(),
            *stats.closed_on.lock().unwrap()
        );
        assert_ne!(
            *stats.closed_on.lock().unwrap(),
            Some(std::thread::current().id())
        );
    }

    #[test]
    fn streamed_frames_match_eager_order_pixels_and_browser_delays() {
        let bytes = gif(100);
        let fixture = Fixture::new(&bytes);
        let eager = decode(&bytes, 8).unwrap().0;
        let mut core = Core::open(fixture.0.clone(), 8).unwrap();
        let mut target = Duration::ZERO;
        for index in 0..100 {
            let (frame, start, end) = core.step(target, || true).unwrap().unwrap();
            assert_eq!(frame.frame_count(), 1);
            assert_eq!(frame.size(0), eager.size(index));
            assert_eq!(frame.as_bytes(0), eager.as_bytes(index));
            assert_eq!(frame.delay(0), eager.delay(index));
            assert_eq!(start, target);
            assert_eq!(end - start, Duration::from(eager.delay(index)));
            target = end;
        }
        let (frame, start, _) = core.step(target, || true).unwrap().unwrap();
        assert_eq!(start, target);
        assert_eq!(frame.as_bytes(0), eager.as_bytes(0));
    }

    #[test]
    fn a_finite_play_count_holds_the_last_frame() {
        let bytes = gif(4);
        let fixture = Fixture::new(&bytes);
        let eager = decode(&bytes, 8).unwrap().0;
        let mut core = Core::open(fixture.0.clone(), 8)
            .unwrap()
            .with_plays(NonZeroU32::new(1));
        let far = Duration::from_secs(10);
        let mut last = None;
        while !core.terminal {
            if let Some((frame, _, _)) = core.step(far, || true).unwrap() {
                last = Some(frame);
            }
        }
        assert_eq!(last.unwrap().as_bytes(0), eager.as_bytes(3));
        assert!(core.step(far, || true).unwrap().is_none());
    }

    #[test]
    fn a_clocked_playback_holds_back_frames_already_past_while_catching_up() {
        let fixture = Fixture::new(&gif(100));
        let retired = Retirements::default();
        let registered = playback(fixture.0.clone(), &retired);
        let both = [
            registered.clocked(Instant::now(), None),
            playback(fixture.0.clone(), &retired),
        ];
        // Far enough in that one slice of eight frames can't reach it.
        let target = Duration::from_secs(5);
        for playback in &both {
            let _lease = DisplayLease::new(&playback.shared, true);
            let mut sessions = HashMap::new();
            request(playback, &mut sessions, target);
            let shared = playback.shared.lock().unwrap();
            if shared.follows_clock {
                assert!(shared.ready.is_none());
                assert_eq!(shared.stale_skips, 1);
            } else {
                assert!(shared.ready.as_ref().unwrap().end <= target);
            }
        }
        let clocked = &both[0];
        let _lease = DisplayLease::new(&clocked.shared, true);
        let mut sessions = HashMap::new();
        for _ in 0..MAX_STALE_SKIPS {
            if clocked.shared.lock().unwrap().ready.is_some() {
                break;
            }
            request(clocked, &mut sessions, target);
        }
        let shared = clocked.shared.lock().unwrap();
        assert!(shared.ready.as_ref().unwrap().end > target);
        assert_eq!(shared.stale_skips, 0);
    }

    #[test]
    fn each_caller_clock_gets_its_own_playback() {
        let fixture = Fixture::new(&gif(4));
        let retired = Retirements::default();
        let registered = playback(fixture.0.clone(), &retired);
        let start = Instant::now();
        let first = registered.clocked(start, NonZeroU32::new(1));
        let second = registered.clocked(start + Duration::from_secs(1), None);
        assert_ne!(first.key, second.key);
        assert_ne!(first.key, registered.key);
        assert!(!Arc::ptr_eq(&first.shared, &second.shared));
        assert_eq!(first.plays, NonZeroU32::new(1));
        let mut first = first;
        assert_eq!(
            first.resume(start + Duration::from_millis(250)),
            Duration::from_millis(250)
        );
    }

    struct SlowFrames {
        reads: usize,
    }

    impl FrameSource for SlowFrames {
        fn read(&mut self) -> Result<SourceRead> {
            std::thread::sleep(Duration::from_millis(5));
            self.reads += 1;
            Ok(SourceRead::Frame(Frame::from_parts(
                RgbaImage::from_pixel(8, 4, Rgba([self.reads as u8, 0, 0, 255])),
                0,
                0,
                Delay::from_numer_denom_ms(1, 1),
            )))
        }

        fn restart(&mut self) -> Result<()> {
            Ok(())
        }

        fn max_frames(&self) -> usize {
            MAX_FRAMES
        }
    }

    #[test]
    fn slow_sources_skip_frames_within_a_single_step() {
        let mut core = Core::open_kind(
            PathBuf::new(),
            8,
            SourceKind::Fake(Arc::new(|| Ok(Box::new(SlowFrames { reads: 0 })))),
            None,
        )
        .unwrap();
        let (_, start, end) = core
            .step(Duration::from_secs(60), || true)
            .unwrap()
            .unwrap();
        assert!(core.decoded > 1 && core.decoded <= DECODE_SLICE);
        assert!(start > Duration::ZERO);
        assert!(end > start);
    }

    #[test]
    fn spooled_frames_round_trip_pixels_delays_and_restart() {
        let bytes = gif(7);
        let fixture = Fixture::new(&bytes);
        let spool = TempFile::new("spool");
        write_spool(&fixture.0, 8, &spool.0, || true).unwrap();
        let eager = decode(&bytes, 8).unwrap().0;
        let mut source = SpoolFrames::open(&spool.0, (8, 4)).unwrap();
        for index in 0..eager.frame_count() {
            let SourceRead::Frame(frame) = source.read().unwrap() else {
                panic!("spool ended after {index} frames");
            };
            assert_eq!(
                frame.buffer().as_raw().as_slice(),
                eager.as_bytes(index).unwrap()
            );
            assert_eq!(
                frame.delay().numer_denom_ms(),
                eager.delay(index).numer_denom_ms()
            );
        }
        assert!(matches!(source.read().unwrap(), SourceRead::End));
        source.restart().unwrap();
        let SourceRead::Frame(frame) = source.read().unwrap() else {
            panic!("spool did not restart");
        };
        assert_eq!(
            frame.buffer().as_raw().as_slice(),
            eager.as_bytes(0).unwrap()
        );
    }

    #[test]
    fn cancelled_spooling_leaves_no_files_behind() {
        let fixture = Fixture::new(&gif(5));
        let spool = TempFile::new("spool");
        assert!(write_spool(&fixture.0, 8, &spool.0, || false).is_err());
        assert!(!spool.0.exists());
        assert!(!spool.0.with_extension("spool.tmp").exists());
    }

    #[test]
    fn corrupt_spool_files_are_deleted_and_the_source_still_plays() {
        let bytes = gif(3);
        let fixture = Fixture::new(&bytes);
        let spool = TempFile::new("spool");
        std::fs::write(&spool.0, b"not a spool").unwrap();
        let mut core = Core::open_kind(
            fixture.0.clone(),
            8,
            SourceKind::Image,
            Some(spool.0.clone()),
        )
        .unwrap();
        assert!(!spool.0.exists());
        let (frame, _, _) = core.step(Duration::ZERO, || true).unwrap().unwrap();
        assert_eq!(frame.as_bytes(0), decode(&bytes, 8).unwrap().0.as_bytes(0));
    }

    #[test]
    fn each_loop_measures_decode_time_from_zero() {
        let fixture = Fixture::new(&gif(2));
        let mut core = Core::open(fixture.0.clone(), 8).unwrap();
        core.spent = Duration::from_secs(3600);
        core.step(Duration::from_secs(1), || true).unwrap();
        assert!(core.spent < Duration::from_secs(3600));
    }

    #[test]
    fn wants_spool_waits_for_the_first_pass_to_fall_behind() {
        let fixture = Fixture::new(&gif(10));
        let mut core = Core::open(fixture.0.clone(), 8).unwrap();
        let spool = TempFile::new("spool");
        core.spool = Some(spool.0.clone());
        core.count = 3;
        core.spent = Duration::from_millis(2);
        assert!(core.wants_spool().is_none());
        core.count = 4;
        core.position = Duration::from_millis(1);
        core.spent = Duration::from_millis(1);
        assert!(core.wants_spool().is_none());
        core.spent = Duration::from_millis(2);
        assert_eq!(core.wants_spool(), Some(spool.0.clone()));
        assert!(core.wants_spool().is_none());
    }

    #[test]
    fn end_of_source_swaps_to_a_ready_spool() {
        let first = gif(3);
        let fixture = Fixture::new(&first);
        let second = gif_colored(3, [7, 8, 9, 255]);
        let other = Fixture::new(&second);
        let spool = TempFile::new("spool");
        write_spool(&other.0, 8, &spool.0, || true).unwrap();
        let expected = decode(&second, 8).unwrap().0;
        let mut core = Core::open(fixture.0.clone(), 8).unwrap();
        core.spool = Some(spool.0.clone());
        for _ in 0..3 {
            let target = core.base + core.position;
            core.step(target, || true).unwrap().unwrap();
        }
        assert_eq!(core.count, 3);
        let target = core.base + core.position;
        let (frame, start, end) = core.step(target, || true).unwrap().unwrap();
        assert!(core.spool.is_none());
        assert!(!core.terminal);
        assert_eq!(start, target);
        assert_eq!(end - start, Duration::from(expected.delay(0)));
        assert_eq!(frame.as_bytes(0), expected.as_bytes(0));
    }

    #[test]
    fn a_frame_read_before_a_spool_swap_is_still_processed() {
        // Seven frames, then the end of the source, fill one decode slice exactly, so the
        // slice's last frame was read from the source the swap replaces.
        let source = gif(DECODE_SLICE - 1);
        let fixture = Fixture::new(&source);
        let other = Fixture::new(&gif_colored(3, [7, 8, 9, 255]));
        let spool = TempFile::new("spool");
        write_spool(&other.0, 8, &spool.0, || true).unwrap();
        let expected = decode(&source, 8).unwrap().0;
        let mut core = Core::open(fixture.0.clone(), 8).unwrap();
        core.spool = Some(spool.0.clone());
        let (frame, _, _) = core
            .step(Duration::from_secs(10), || true)
            .unwrap()
            .unwrap();
        assert!(core.spool.is_none());
        assert_eq!(frame.as_bytes(0), expected.as_bytes(DECODE_SLICE - 2));
    }

    #[test]
    fn first_frame_jobs_use_the_bounded_pool_and_cancel_before_opening_stale_sources() {
        let bytes = gif(3);
        let fixture = Fixture::new(&bytes[..bytes.len() - 10]);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let image = runtime
            .block_on(first(
                fixture.0.clone(),
                8,
                Arc::new(Mutex::new(Instant::now())),
            ))
            .unwrap()
            .unwrap();
        assert_eq!(image.frame_count(), 1);
        assert_eq!(image.as_bytes(0), decode(&bytes, 8).unwrap().0.as_bytes(0));
        assert_eq!(pool().workers.len(), WORKERS);
        let missing = fixture.0.with_extension("missing");
        let cancelled = runtime
            .block_on(first(
                missing,
                8,
                Arc::new(Mutex::new(Instant::now() - STALE_AFTER - IDLE)),
            ))
            .unwrap();
        assert!(cancelled.is_none());
    }

    #[test]
    fn many_frame_playback_holds_at_most_still_plus_three_frames_including_scene_pins() {
        let fixture = Fixture::new(&gif(500));
        let retired = Rc::default();
        let mut playback = playback(fixture.0.clone(), &retired);
        let lease = DisplayLease::new(&playback.shared, true);
        let mut sessions = HashMap::new();
        let slots = playback.shared.lock().unwrap().slots.clone();
        let mut released = std::collections::HashSet::new();
        let mut handed = std::collections::HashSet::new();
        for _ in 0..1000 {
            request(&playback, &mut sessions, playback.end);
            assert!(playback.shared.lock().unwrap().ready.is_some());
            let old_scene = playback.current.clone();
            adopt(&mut playback, &retired);
            handed.insert(playback.current.as_ref().unwrap().image.id);
            assert!(slots.load(Ordering::Acquire) <= PLAYBACK_FRAMES);
            if old_scene.is_some() {
                request(&playback, &mut sessions, playback.end);
                assert!(playback.shared.lock().unwrap().ready.is_some());
                assert_eq!(slots.load(Ordering::Acquire), PLAYBACK_FRAMES);
            }
            drop(old_scene);
            release_with(&retired, |image| assert!(released.insert(image.id)));
        }
        drop(lease);
        playback.shared.lock().unwrap().ready = None;
        playback.current = None;
        release_with(&retired, |image| assert!(released.insert(image.id)));
        assert_eq!(slots.load(Ordering::Acquire), 0);
        assert_eq!(handed, released);
    }

    #[test]
    fn decoder_only_runs_on_demand_and_never_runs_ahead_of_its_ready_slot() {
        let fixture = Fixture::new(&gif(100));
        let retired = Rc::default();
        let playback = playback(fixture.0.clone(), &retired);
        let mut sessions = HashMap::new();
        request(&playback, &mut sessions, playback.end);
        assert!(sessions.is_empty());
        let lease = DisplayLease::new(&playback.shared, true);
        request(&playback, &mut sessions, playback.end);
        let count = sessions[&playback.key].core.decoded;
        assert_eq!(count, 2);
        for _ in 0..100 {
            request(&playback, &mut sessions, Duration::from_secs(10));
        }
        assert_eq!(sessions[&playback.key].core.decoded, count);
        playback.shared.lock().unwrap().ready = None;
        drop(lease);
        for _ in 0..100 {
            request(&playback, &mut sessions, Duration::from_secs(10));
        }
        assert_eq!(sessions[&playback.key].core.decoded, count);
    }

    #[test]
    fn idle_discards_decoder_and_playback_frames_keeps_still_and_reopens_source() {
        let fixture = Fixture::new(&gif(100));
        let retired = Rc::default();
        let mut playback = playback(fixture.0.clone(), &retired);
        let still_id = playback.still.image.id;
        let slots = playback.shared.lock().unwrap().slots.clone();
        let lease = DisplayLease::new(&playback.shared, true);
        let mut sessions = HashMap::new();
        request(&playback, &mut sessions, playback.end);
        adopt(&mut playback, &retired);
        request(&playback, &mut sessions, playback.end);
        assert_eq!(slots.load(Ordering::Acquire), 2);
        drop(lease);
        let absent = playback.shared.lock().unwrap().absent_since;
        assert!(!playback.expire(absent + IDLE - Duration::from_nanos(1)));
        assert!(playback.expire(absent + IDLE));
        sweep_sessions(&mut sessions, absent + IDLE);
        assert!(sessions.is_empty());
        assert!(playback.current.is_none());
        assert!(playback.shared.lock().unwrap().ready.is_none());
        assert_eq!(playback.still.image.id, still_id);
        assert!(playback.started.is_none());
        release_with(&retired, |_| {});
        assert_eq!(slots.load(Ordering::Acquire), 0);
        let _lease = DisplayLease::new(&playback.shared, true);
        request(&playback, &mut sessions, playback.end);
        adopt(&mut playback, &retired);
        assert!(sessions.contains_key(&playback.key));
        assert_eq!(
            playback.current.as_ref().unwrap().image.as_bytes(0),
            decode(&gif(100), 8).unwrap().0.as_bytes(1)
        );
    }

    #[test]
    fn truncated_gif_keeps_last_good_frame_and_stops_without_panicking() {
        let bytes = gif(3);
        let truncated = &bytes[..bytes.len() - 10];
        assert!(decode(truncated, 8).is_err());
        let fixture = Fixture::new(truncated);
        let retired = Rc::default();
        let mut playback = playback(fixture.0.clone(), &retired);
        let _lease = DisplayLease::new(&playback.shared, true);
        let mut sessions = HashMap::new();
        request(&playback, &mut sessions, Duration::ZERO);
        adopt(&mut playback, &retired);
        request(&playback, &mut sessions, playback.end);
        adopt(&mut playback, &retired);
        release_with(&retired, |_| {});
        let last_good = playback.current.as_ref().unwrap().image.clone();
        request(&playback, &mut sessions, playback.end);
        assert!(playback.shared.lock().unwrap().terminal);
        assert!(playback.shared.lock().unwrap().ready.is_none());
        assert_eq!(playback.current.as_ref().unwrap().image.id, last_good.id);
        assert_eq!(
            last_good.as_bytes(0),
            decode(&bytes, 8).unwrap().0.as_bytes(1)
        );
        assert!(sessions.is_empty());
    }

    #[test]
    fn stalled_playback_skips_full_cycles_and_bounds_each_decode_slice() {
        let fixture = Fixture::new(&gif(20));
        let mut core = Core::open(fixture.0.clone(), 8).unwrap();
        let target = Duration::from_secs(3600);
        let mut latest_end = Duration::ZERO;
        for _ in 0..20 {
            let before = core.decoded;
            if let Some((_, _, end)) = core.step(target, || true).unwrap() {
                latest_end = end;
            }
            assert!(core.decoded - before <= DECODE_SLICE);
            if latest_end > target {
                break;
            }
        }
        assert!(latest_end > target);
        let before = core.decoded;
        assert!(core.step(target, || false).unwrap().is_none());
        assert_eq!(core.decoded, before);
    }

    #[test]
    fn short_absences_pause_the_clock_and_stale_generations_cannot_publish() {
        let fixture = Fixture::new(&gif(100));
        let retired = Rc::default();
        let mut playback = playback(fixture.0.clone(), &retired);
        let base = playback.started.unwrap();
        playback.shared.lock().unwrap().paused_since = Some(base + Duration::from_millis(20));
        assert_eq!(
            playback.resume(base + Duration::from_millis(500)),
            Duration::from_millis(20)
        );
        let _lease = DisplayLease::new(&playback.shared, true);
        assert_eq!(
            playback.resume(base + Duration::from_millis(700)),
            Duration::from_millis(220)
        );
        let mut sessions = HashMap::new();
        let generation = playback.shared.lock().unwrap().generation;
        playback.shared.lock().unwrap().reset();
        advance(
            &mut sessions,
            playback.key,
            generation,
            playback.plays,
            playback.path.clone(),
            playback.size,
            playback.kind.clone(),
            None,
            &playback.shared,
        );
        assert!(sessions.is_empty());
        assert!(playback.shared.lock().unwrap().ready.is_none());
    }

    #[test]
    fn inactive_scenes_keep_the_current_frame_without_decoding_or_advancing_the_clock() {
        let fixture = Fixture::new(&gif(100));
        let retired = Rc::default();
        let mut playback = playback(fixture.0.clone(), &retired);
        let mut sessions = HashMap::new();
        let active = DisplayLease::new(&playback.shared, true);
        request(&playback, &mut sessions, playback.end);
        adopt(&mut playback, &retired);
        let current = playback.current.as_ref().unwrap().image.id;
        let decoded = sessions[&playback.key].core.decoded;
        let inactive = DisplayLease::new(&playback.shared, false);
        drop(active);
        let paused = playback.shared.lock().unwrap().paused_since.unwrap();
        let elapsed = paused.saturating_duration_since(playback.started.unwrap());
        assert!(!playback.expire(paused + IDLE * 10));
        sweep_sessions(&mut sessions, paused + IDLE * 10);
        request(&playback, &mut sessions, playback.end);
        assert_eq!(sessions[&playback.key].core.decoded, decoded);
        assert_eq!(playback.current.as_ref().unwrap().image.id, current);
        assert!(playback.shared.lock().unwrap().ready.is_none());
        let returned = paused + Duration::from_secs(10);
        assert_eq!(playback.resume(returned), elapsed);
        let _active = DisplayLease::new(&playback.shared, true);
        drop(inactive);
        assert_eq!(playback.resume(returned + POLL), elapsed + POLL);
        assert_eq!(playback.current.as_ref().unwrap().image.id, current);
    }

    #[test]
    fn source_dimensions_frame_count_and_file_size_remain_limited() {
        let fixture = Fixture::new(&gif(MAX_FRAMES + 1));
        let mut core = Core::open(fixture.0.clone(), 8).unwrap();
        for _ in 0..MAX_FRAMES + 1 {
            core.step(core.base + core.position, || true).unwrap();
        }
        assert!(core.terminal);
        assert_eq!(core.decoded, MAX_FRAMES);
        assert!(Core::open(fixture.0.clone(), 0).is_err());
        let mut bad_header = gif(1);
        bad_header[6..8].copy_from_slice(&(MAX_DIMENSION as u16 + 1).to_le_bytes());
        let bad = Fixture::new(&bad_header);
        assert!(Core::open(bad.0.clone(), 8).is_err());
        File::options()
            .write(true)
            .open(&bad.0)
            .unwrap()
            .set_len(MAX_SOURCE_BYTES + 1)
            .unwrap();
        assert!(Core::open(bad.0.clone(), 8).is_err());
    }

    #[test]
    fn large_media_only_uses_streaming() {
        assert!(selected(&spec()));
        assert!(selected(&ImageSpec::Attachment {
            id: 1,
            url: "https://cdn.discordapp.com/test.gif".into(),
            width: 8,
            height: 4,
        }));
        for spec in [
            ImageSpec::Emoji {
                id: 1,
                animated: true,
            },
            ImageSpec::Avatar {
                user_id: 1,
                hash: "a_test".into(),
            },
            ImageSpec::Nameplate {
                asset: "test".into(),
                animated: true,
            },
        ] {
            assert!(!selected(&spec));
        }
        assert!(selected(&ImageSpec::Collectible { url: "test".into() }));
        let still = ImageSpec::Media {
            url: "https://media.discordapp.net/test.gif".into(),
            width: 8,
            height: 4,
            animated: false,
        };
        assert!(matches!(
            source_spec(&still),
            ImageSpec::Media { animated: true, .. }
        ));
        assert!(matches!(
            still,
            ImageSpec::Media {
                animated: false,
                ..
            }
        ));
    }

    #[test]
    fn local_files_only_stream_gifs() {
        assert!(selected(&ImageSpec::LocalGif {
            path: PathBuf::from("staged.gif"),
        }));
        assert!(!selected(&ImageSpec::LocalVideo {
            path: PathBuf::from("staged.mp4"),
        }));
    }

    struct LeaseView {
        frame: Option<Rc<FrameImage>>,
    }

    struct StreamProbe {
        still: Arc<RenderImage>,
        animate: bool,
        picked: Rc<std::cell::Cell<Option<gpui::ImageId>>>,
    }

    impl Render for StreamProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let still = self.still.clone();
            let picked = self.picked.clone();
            let animate = self.animate;
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let image = paint(&still, animate, None, window, cx).unwrap();
                    picked.set(Some(image.id));
                    window
                        .paint_image(bounds, gpui::Corners::default(), image, 0, false)
                        .unwrap();
                },
            )
            .size(px(32.))
        }
    }

    #[gpui::test]
    fn reduced_motion_inactivity_and_production_close_cleanup(cx: &mut TestAppContext) {
        cx.update(super::super::init);
        FORCE_ACTIVE.set(true);
        let fixture = Fixture::new(&gif(3));
        let still = still();
        let still_id = still.id;
        let playback = cx.update(|cx| {
            register(&spec(), 8, fixture.0.clone(), None, still.clone(), cx);
            cx.global::<StreamGlobal>().state.borrow().entries[&still_id].clone()
        });
        let slots = playback.borrow().shared.lock().unwrap().slots.clone();
        let frame_id = {
            let mut playback = playback.borrow_mut();
            let frame = Rc::new(FrameImage {
                image: self::still(),
                slot: Slot::reserve(&slots),
                retired: playback.still.retired.clone(),
            });
            let id = frame.image.id;
            playback.current = Some(frame);
            playback.started = Some(Instant::now());
            playback.shared.lock().unwrap().terminal = true;
            id
        };
        let picked = Rc::new(std::cell::Cell::new(None));
        let (view, cx) = cx.add_window_view(|_, _| StreamProbe {
            still,
            animate: true,
            picked: picked.clone(),
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert_eq!(picked.get(), Some(frame_id));
        assert_eq!(playback.borrow().shared.lock().unwrap().displays, 1);
        cx.update(|window, cx| {
            cx.set_reduce_motion(true);
            window.refresh();
            window.draw(cx).clear();
        });
        assert_eq!(picked.get(), Some(still_id));
        assert_eq!(playback.borrow().shared.lock().unwrap().displays, 1);
        assert_eq!(playback.borrow().shared.lock().unwrap().active_displays, 0);
        FORCE_ACTIVE.set(false);
        cx.update(|window, cx| {
            cx.set_reduce_motion(false);
            window.refresh();
        });
        view.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| window.draw(cx).clear());
        assert_eq!(picked.get(), Some(frame_id));
        assert_eq!(playback.borrow().shared.lock().unwrap().displays, 1);
        assert_eq!(playback.borrow().shared.lock().unwrap().active_displays, 0);
        assert!(!playback.borrow_mut().expire(Instant::now() + IDLE * 2));
        assert_eq!(
            playback.borrow().current.as_ref().unwrap().image.id,
            frame_id
        );
        FORCE_ACTIVE.set(true);
        view.update(cx, |view, cx| {
            view.animate = false;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear());
        assert_eq!(picked.get(), Some(still_id));
        assert_eq!(playback.borrow().shared.lock().unwrap().displays, 1);
        assert_eq!(playback.borrow().shared.lock().unwrap().active_displays, 0);
        FORCE_ACTIVE.set(false);
        cx.update(|window, _| window.remove_window());
        assert!(playback.borrow().current.is_none());
        assert!(playback.borrow().started.is_none());
        assert_eq!(slots.load(Ordering::Acquire), 0);
        assert!(cx.cx.read(|cx| {
            cx.global::<StreamGlobal>()
                .state
                .borrow()
                .retired
                .borrow()
                .is_empty()
        }));
    }

    impl Render for LeaseView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let frame = self.frame.clone();
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    if let Some(frame) = frame {
                        window
                            .paint_image(
                                bounds,
                                gpui::Corners::default(),
                                frame.image.clone(),
                                0,
                                false,
                            )
                            .unwrap();
                        pin_scene(frame.clone(), None, frame.image.clone(), window);
                    }
                },
            )
            .size(px(32.))
        }
    }

    struct LeaseHost(Entity<LeaseView>);

    impl Render for LeaseHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                self.0
                    .clone()
                    .cached(StyleRefinement::default().size(px(32.))),
            )
        }
    }

    #[gpui::test]
    fn scene_reuse_replacement_and_window_close_release_each_image_exactly_once(
        cx: &mut TestAppContext,
    ) {
        let retired: Retirements = Rc::default();
        let slots = Arc::new(AtomicUsize::new(0));
        let first = Rc::new(FrameImage {
            image: still(),
            slot: Slot::reserve(&slots),
            retired: retired.clone(),
        });
        let first_id = first.image.id;
        let weak = Rc::downgrade(&first);
        let (host, cx) =
            cx.add_window_view(|_, cx| LeaseHost(cx.new(|_| LeaseView { frame: Some(first) })));
        cx.update(|window, cx| window.draw(cx).clear());
        let leaf = host.read_with(cx, |host, _| host.0.clone());
        leaf.update(cx, |view, _| view.frame = None);
        host.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(
            weak.upgrade().is_some(),
            "cached scene must retain its image"
        );
        assert!(retired.borrow().is_empty());
        assert_eq!(slots.load(Ordering::Acquire), 1);
        leaf.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| window.draw(cx).clear());
        assert!(weak.upgrade().is_none());
        let mut released = Vec::new();
        cx.update(|_, cx| {
            release_with(&retired, |image| {
                released.push(image.id);
                cx.drop_image(image, None);
            })
        });
        assert_eq!(released, [first_id]);
        assert_eq!(slots.load(Ordering::Acquire), 0);

        let last = Rc::new(FrameImage {
            image: still(),
            slot: Slot::reserve(&slots),
            retired: retired.clone(),
        });
        let last_id = last.image.id;
        leaf.update(cx, |view, cx| {
            view.frame = Some(last);
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear());
        leaf.update(cx, |view, _| view.frame = None);
        cx.update(|window, _| window.remove_window());
        cx.cx.update(|cx| {
            release_with(&retired, |image| {
                released.push(image.id);
                cx.drop_image(image, None);
            })
        });
        release_with(&retired, |image| released.push(image.id));
        assert_eq!(released, [first_id, last_id]);
        assert_eq!(slots.load(Ordering::Acquire), 0);
    }
}
