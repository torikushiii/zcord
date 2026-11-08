mod decoder;
mod sources;
mod worker;

use sources::SourceKind;
#[cfg(any(windows, test))]
pub(crate) use sources::{FrameSource, SourceRead};
pub(super) use worker::first;
#[cfg(windows)]
pub(crate) use worker::first_video;
use worker::{Command, pool};

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
const PLAYBACK_FRAMES: usize = 3;
const IDLE: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(16);
const SWEEP: Duration = Duration::from_millis(100);
const DECODE_SLICE: usize = 8;

pub(super) fn selected(spec: &ImageSpec) -> bool {
    matches!(
        spec,
        ImageSpec::Media { .. }
            | ImageSpec::Attachment { .. }
            | ImageSpec::LocalGif { .. }
            | ImageSpec::Collectible { .. }
    )
}

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

#[cfg(test)]
mod tests;
