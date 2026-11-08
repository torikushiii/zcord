use crate::discord::settings_proto::StickerAnimation;
use crate::media::{
    frames,
    images::{self, ImageSpec},
};
use anyhow::{Context, Result, ensure};
use gpui::{
    App, DefiniteLength, Element, ElementId, EntityId, Global, GlobalElementId, Hitbox,
    InspectorElementId, InteractiveElement, Interactivity, IntoElement, LayoutId, Length,
    ObjectFit, Pixels, RenderImage, StatefulInteractiveElement, StyleRefinement, Styled, Task,
    Window, px,
};
use gpui_tokio::Tokio;
use image::{
    AnimationDecoder, Delay, DynamicImage, Frame, Frames, ImageDecoder, ImageFormat, ImageReader,
    Limits, RgbaImage,
    codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder},
    imageops::{FilterType, resize},
    metadata::LoopCount,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    io::Cursor,
    num::NonZeroU32,
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

mod apng;
pub(crate) mod stream;

const MAX_IMAGE: usize = 32 * 1024 * 1024;
const MAX_TOTAL: usize = 256 * 1024 * 1024;
const MAX_FRAMES: usize = 600;
const MAX_DIMENSION: u32 = 4096;
const DOWNLOADS: usize = 8;
const DECODES: usize = 4;
/// Emoji and stickers only queue a tiny decode each, so they get their own lane.
const SMALL_DOWNLOADS: usize = 8;
const SMALL_DECODES: usize = 2;
const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const STALE_AFTER: Duration = Duration::from_secs(2);
/// An animation not shown for this long is dropped even under budget, so memory follows what
/// is on screen rather than filling the budget with what was. Coming back decodes it again.
const UNSEEN_DROP: Duration = Duration::from_secs(30);
/// A just-decoded image may still be waiting for its debounced repaint.
const FRESH_FOR: Duration = Duration::from_secs(1);
/// Must stay under `STALE_AFTER`, or jobs for visible tiles get cancelled between repaints.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
/// Elements only hold the image Arc during paint, so recent cache hits mark it visible.
const VISIBLE_GRACE: Duration = Duration::from_secs(5);
type Key = (ImageSpec, u32);

pub enum Load {
    Pending,
    Ready(Arc<RenderImage>),
    Failed,
}

#[derive(Clone, Copy)]
enum Lane {
    Gif,
    Small,
}

impl Lane {
    fn of(spec: &ImageSpec) -> Self {
        match spec {
            ImageSpec::Emoji { .. } | ImageSpec::Sticker { .. } => Lane::Small,
            ImageSpec::Animated { still } => Lane::of(still),
            _ => Lane::Gif,
        }
    }

    fn downloads(self) -> usize {
        match self {
            Lane::Gif => DOWNLOADS,
            Lane::Small => SMALL_DOWNLOADS,
        }
    }

    fn decodes(self) -> usize {
        match self {
            Lane::Gif => DECODES,
            Lane::Small => SMALL_DECODES,
        }
    }
}

fn decode_permits(lane: Lane) -> &'static tokio::sync::Semaphore {
    static GIF: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    static SMALL: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    match lane {
        Lane::Gif => GIF.get_or_init(|| tokio::sync::Semaphore::new(lane.decodes())),
        Lane::Small => SMALL.get_or_init(|| tokio::sync::Semaphore::new(lane.decodes())),
    }
}

fn wanted(last: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last) <= STALE_AFTER
}

fn still_wanted(stamp: &Mutex<Instant>) -> bool {
    wanted(
        *stamp
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        Instant::now(),
    )
}

enum DecodeOutcome {
    Decoded(Arc<RenderImage>, usize, Option<NonZeroU32>),
    Stream(
        Arc<RenderImage>,
        std::path::PathBuf,
        std::path::PathBuf,
        Option<NonZeroU32>,
    ),
    Cancelled,
}

enum Entry {
    Pending {
        /// Last `state`/`resolve` request for this key, used to drop abandoned work.
        requested: Instant,
        queued: bool,
        /// Refreshed on every request while the job is queued or in flight.
        wanted: Arc<Mutex<Instant>>,
    },
    Ready {
        image: Arc<RenderImage>,
        bytes: usize,
        plays: Option<NonZeroU32>,
        used: u64,
        ready_at: Instant,
        seen: Instant,
    },
    Failed,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<ImageSpec, HashMap<u32, Entry>>,
    /// Newest request at the front.
    queue: Vec<Key>,
    bytes: usize,
    clock: u64,
    running: [usize; 2],
    pump_pending: bool,
    refresh_pending: bool,
    heartbeat_running: bool,
    purge_scheduled: bool,
}

struct AnimatedCache(Rc<RefCell<Cache>>);
impl Global for AnimatedCache {}

pub fn init(cx: &mut App) {
    cx.set_global(AnimatedCache(Rc::new(RefCell::new(Cache::default()))));
    cx.set_global(AnimatedWakeups(Rc::new(RefCell::new(Wakeups::default()))));
    stream::init(cx);
}

/// A frame change this close is worth waiting for the compositor instead of a timer.
const FRAME_LEAD: Duration = Duration::from_millis(8);

pub struct AnimatedImage {
    image: Arc<RenderImage>,
    animate: bool,
    object_fit: Fit,
    started_at: Option<Instant>,
    plays: Option<NonZeroU32>,
    interactivity: Interactivity,
}

#[derive(Clone, Copy)]
enum Fit {
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
}

impl Fit {
    fn of(fit: ObjectFit) -> Self {
        match fit {
            ObjectFit::Fill => Fit::Fill,
            ObjectFit::Contain => Fit::Contain,
            ObjectFit::Cover => Fit::Cover,
            ObjectFit::ScaleDown => Fit::ScaleDown,
            ObjectFit::None => Fit::None,
        }
    }

    fn bounds(
        self,
        bounds: gpui::Bounds<Pixels>,
        size: gpui::Size<gpui::DevicePixels>,
    ) -> gpui::Bounds<Pixels> {
        let fit = match self {
            Fit::Fill => ObjectFit::Fill,
            Fit::Contain => ObjectFit::Contain,
            Fit::Cover => ObjectFit::Cover,
            Fit::ScaleDown => ObjectFit::ScaleDown,
            Fit::None => ObjectFit::None,
        };
        fit.get_bounds(bounds, size)
    }
}

/// An animated [`RenderImage`] that wakes only when its current frame is due.
///
/// `Img` requests an animation frame on every frame while its image has more than one
/// frame, which redraws the whole view at monitor refresh for a 30-100ms GIF. This
/// element picks the frame from elapsed time and schedules one timer per view for the
/// next change.
pub fn image(image: Arc<RenderImage>) -> AnimatedImage {
    AnimatedImage {
        image,
        animate: true,
        object_fit: Fit::Contain,
        started_at: None,
        plays: None,
        interactivity: Interactivity::new(),
    }
}

pub(crate) fn poster_image(frame: Arc<RenderImage>) -> AnimatedImage {
    let mut poster = image(frame);
    poster.animate = false;
    poster
}

impl AnimatedImage {
    pub fn object_fit(mut self, object_fit: ObjectFit) -> Self {
        self.object_fit = Fit::of(object_fit);
        self
    }

    /// Uses an external timeline; a finite play count holds the final frame when exhausted.
    pub(crate) fn play_from(mut self, started_at: Instant, plays: Option<NonZeroU32>) -> Self {
        self.started_at = Some(started_at);
        self.plays = plays;
        self
    }
}

impl Styled for AnimatedImage {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for AnimatedImage {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl StatefulInteractiveElement for AnimatedImage {}

impl IntoElement for AnimatedImage {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for AnimatedImage {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |mut style, window, cx| {
                let size = self.image.size(0);
                let image_size = gpui::size(px(size.width.0 as f32), px(size.height.0 as f32));
                // Match `Img`: intrinsic aspect only when a dimension is auto, otherwise
                // an explicit size wins (Cover crops, it must not resize the element).
                if matches!(style.size.width, Length::Auto)
                    || matches!(style.size.height, Length::Auto)
                {
                    let ratio = if image_size.height > px(0.) {
                        image_size.width / image_size.height
                    } else {
                        1.
                    };
                    style.aspect_ratio = Some(ratio);
                }
                if let Length::Auto = style.size.width {
                    style.size.width = match style.size.height {
                        Length::Definite(DefiniteLength::Absolute(height)) => {
                            let height = height.to_pixels(window.rem_size());
                            Length::Definite(
                                px(f32::from(image_size.width) * f32::from(height)
                                    / f32::from(image_size.height))
                                .into(),
                            )
                        }
                        _ => Length::Definite(image_size.width.into()),
                    };
                }
                if let Length::Auto = style.size.height {
                    style.size.height = match style.size.width {
                        Length::Definite(DefiniteLength::Absolute(width)) => {
                            let width = width.to_pixels(window.rem_size());
                            Length::Definite(
                                px(f32::from(image_size.height) * f32::from(width)
                                    / f32::from(image_size.width))
                                .into(),
                            )
                        }
                        _ => Length::Definite(image_size.height.into()),
                    };
                }
                window.request_layout(style, None::<LayoutId>, cx)
            },
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_, _, hitbox, _, _| hitbox,
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: gpui::Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let image = self.image.clone();
        let object_fit = self.object_fit;
        let started_at = self.started_at;
        let plays = self.plays;
        let animate = self.animate;
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |style, window, cx| {
                if bounds.intersect(&window.content_mask().bounds).is_empty() {
                    return;
                }
                let clock = started_at.map(|started_at| (started_at, plays));
                let streamed = stream::paint(&image, animate, clock, window, cx);
                let image = streamed.as_ref().unwrap_or(&image);
                let frame = match (started_at, global_id) {
                    _ if !animate => 0,
                    _ if streamed.is_some() => 0,
                    (Some(started_at), _) => frame_index_at(&image, started_at, plays, window, cx),
                    (None, Some(global_id)) => frame_index(&image, global_id, window, cx),
                    // Without an id there is nowhere to keep the clock; frame 0 is the
                    // only stable choice, matching `Img`'s stateless path.
                    (None, None) => 0,
                };
                if image.frame_count() == 0 {
                    return;
                }
                let frame = frame.min(image.frame_count() - 1);
                let fitted = object_fit.bounds(bounds, image.size(frame));
                let visible = bounds.intersect(&fitted);
                let corner_radii = style
                    .corner_radii
                    .to_pixels(window.rem_size())
                    .clamp_radii_for_quad_size(visible.size);
                window
                    .paint_image_fitted(visible, fitted, corner_radii, image.clone(), frame, false)
                    .ok();
            },
        );
    }
}

/// Canvas overlays use the same playback clock and per-view wakeups as animated elements.
pub(crate) fn frame_index(
    image: &Arc<RenderImage>,
    id: &GlobalElementId,
    window: &mut Window,
    cx: &mut App,
) -> usize {
    let (frame, wake) = window.with_element_state(id, |state: Option<AnimatedState>, window| {
        let mut state = state
            .filter(|state| state.image.id == image.id)
            .unwrap_or_else(|| AnimatedState {
                image: image.clone(),
                started: cx.background_executor().now(),
                paused_at: None,
                frame: 0,
            });
        let wake = state.advance(image, window, cx);
        ((state.frame, wake), state)
    });
    request_wakeup(wake, window, cx);
    frame
}

fn frame_index_at(
    image: &RenderImage,
    started_at: Instant,
    plays: Option<NonZeroU32>,
    window: &mut Window,
    cx: &mut App,
) -> usize {
    let reduced = cx.reduce_motion();
    let elapsed = if reduced {
        Duration::ZERO
    } else {
        cx.background_executor()
            .now()
            .saturating_duration_since(started_at)
    };
    let (frame, remaining) = frame_at(image, elapsed, plays);
    request_wakeup(
        wake(
            remaining,
            image.frame_count() > 1,
            reduced,
            window_active(window),
        ),
        window,
        cx,
    );
    frame
}

fn request_wakeup(wake: Wake, window: &mut Window, cx: &mut App) {
    match wake {
        Wake::AnimationFrame => window.request_animation_frame(),
        Wake::Timer(remaining) => {
            let view = window.current_view();
            schedule_wakeup(view, cx, remaining, window);
        }
        Wake::None => {}
    }
}

struct AnimatedState {
    image: Arc<RenderImage>,
    started: Instant,
    paused_at: Option<Instant>,
    frame: usize,
}

// GPUI's test platform always reports its window inactive, so render-count tests
// have to opt in to the active path explicitly.
#[cfg(test)]
thread_local! {
    static FORCE_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn window_active(window: &Window) -> bool {
    #[cfg(test)]
    if FORCE_ACTIVE.with(|active| active.get()) {
        return true;
    }
    window.is_window_active()
}

impl AnimatedState {
    fn advance(&mut self, image: &RenderImage, window: &Window, cx: &App) -> Wake {
        let now = cx.background_executor().now();
        let animated = image.frame_count() > 1;
        let reduced = cx.reduce_motion();
        let active = window_active(window);
        let elapsed = if reduced {
            self.started = now;
            self.paused_at = None;
            Duration::ZERO
        } else if !active {
            let paused = *self.paused_at.get_or_insert(now);
            paused.saturating_duration_since(self.started)
        } else {
            if let Some(paused) = self.paused_at.take() {
                self.started += now.saturating_duration_since(paused);
            }
            now.saturating_duration_since(self.started)
        };
        let (frame, remaining) = frame_at(image, elapsed, None);
        self.frame = frame;
        wake(remaining, animated, reduced, active)
    }
}

/// The frame visible at `elapsed` and how long until the next one.
fn frame_at(
    image: &RenderImage,
    elapsed: Duration,
    plays: Option<NonZeroU32>,
) -> (usize, Duration) {
    let count = image.frame_count();
    if count == 0 {
        return (0, Duration::ZERO);
    }
    if count == 1 {
        return (0, Duration::MAX);
    }
    let total: Duration = (0..count)
        .map(|index| Duration::from(image.delay(index)))
        .sum();
    let total = total.max(Duration::from_nanos(1));
    if plays.is_some_and(|plays| elapsed.as_nanos() / total.as_nanos() >= u128::from(plays.get())) {
        return (count - 1, Duration::MAX);
    }
    let mut rest = Duration::from_nanos((elapsed.as_nanos() % total.as_nanos()) as u64);
    for index in 0..count {
        let delay = Duration::from(image.delay(index));
        if rest < delay {
            return (index, delay - rest);
        }
        rest -= delay;
    }
    (0, Duration::from(image.delay(0)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wake {
    AnimationFrame,
    Timer(Duration),
    None,
}

fn wake(remaining: Duration, animated: bool, reduced: bool, active: bool) -> Wake {
    if !animated || reduced || !active || remaining == Duration::MAX {
        Wake::None
    } else if remaining <= FRAME_LEAD {
        Wake::AnimationFrame
    } else {
        Wake::Timer(remaining)
    }
}

struct Wakeup {
    deadline: Instant,
    task: Option<Task<()>>,
}

#[derive(Default)]
struct Wakeups {
    pending: HashMap<EntityId, Wakeup>,
    /// A fired task must not drop its own handle while it is still running; park it
    /// here and drop it from a later paint, where the executor is not inside it.
    retired: Vec<Task<()>>,
}

struct AnimatedWakeups(Rc<RefCell<Wakeups>>);
impl Global for AnimatedWakeups {}

impl Wakeups {
    fn claim(&mut self, view: EntityId, deadline: Instant) -> bool {
        self.retired.clear();
        if self
            .pending
            .get(&view)
            .is_some_and(|wakeup| wakeup.deadline <= deadline)
        {
            return false;
        }
        self.pending.insert(
            view,
            Wakeup {
                deadline,
                task: None,
            },
        );
        true
    }

    fn attach(&mut self, view: EntityId, deadline: Instant, task: Task<()>) {
        if let Some(wakeup) = self.pending.get_mut(&view)
            && wakeup.deadline == deadline
        {
            wakeup.task = Some(task);
        }
    }

    fn release(&mut self, view: EntityId, deadline: Instant) -> bool {
        let fired = self
            .pending
            .get(&view)
            .is_some_and(|wakeup| wakeup.deadline == deadline);
        if fired
            && let Some(wakeup) = self.pending.remove(&view)
            && let Some(task) = wakeup.task
        {
            self.retired.push(task);
        }
        fired
    }
}

fn schedule_wakeup(view: EntityId, cx: &mut App, remaining: Duration, window: &Window) {
    let Some(clocks) = cx.try_global::<AnimatedWakeups>().map(|c| c.0.clone()) else {
        return;
    };
    let deadline = cx.background_executor().now() + remaining;
    if !clocks.borrow_mut().claim(view, deadline) {
        return;
    }
    let task = {
        let clocks = clocks.clone();
        window.spawn(cx, async move |cx| {
            cx.background_executor().timer(remaining).await;
            if clocks.borrow_mut().release(view, deadline) {
                cx.update(move |_, cx| cx.notify(view)).ok();
            }
        })
    };
    clocks.borrow_mut().attach(view, deadline, task);
}

fn animations_enabled(spec: &ImageSpec) -> bool {
    match spec {
        ImageSpec::Emoji { .. } => crate::settings::emoji_animation(),
        ImageSpec::Sticker { .. } => {
            crate::settings::display().animate_stickers != StickerAnimation::Never
        }
        ImageSpec::Media { .. } | ImageSpec::Attachment { .. } | ImageSpec::LocalGif { .. } => {
            crate::settings::gif_autoplay() && crate::settings::window_focused()
        }
        ImageSpec::Animated { still } => animations_enabled(still),
        _ => true,
    }
}

pub fn resolve(spec: &ImageSpec, size: u32, cx: &App) -> Option<Arc<RenderImage>> {
    if !animations_enabled(spec) {
        return None;
    }
    match state(spec, size, cx) {
        Load::Ready(image) => Some(image),
        Load::Pending | Load::Failed => None,
    }
}

pub fn poster(spec: &ImageSpec, size: u32, cx: &App) -> Option<AnimatedImage> {
    if !stream::selected(spec) {
        return None;
    }
    match state(spec, size, cx) {
        Load::Ready(frame) => Some(poster_image(frame)),
        Load::Pending | Load::Failed => None,
    }
}

pub fn first_frame(spec: &ImageSpec, size: u32, cx: &App) -> Option<AnimatedImage> {
    match state(spec, size, cx) {
        Load::Ready(frames) => Some(poster_image(frames)),
        Load::Pending | Load::Failed => None,
    }
}

pub fn fallback(spec: &ImageSpec, size: u32, cx: &App) -> Option<ImageSpec> {
    fallback_spec(spec, matches!(state(spec, size, cx), Load::Failed))
}

fn fallback_spec(spec: &ImageSpec, failed: bool) -> Option<ImageSpec> {
    match spec {
        ImageSpec::Media {
            url, width, height, ..
        } => Some(ImageSpec::Media {
            url: url.clone(),
            width: *width,
            height: *height,
            animated: false,
        }),
        ImageSpec::Attachment { .. } if failed => Some(spec.clone()),
        _ => None,
    }
}

/// Intrinsic APNG play counts are retained for collectibles; `None` means unbounded.
pub(crate) fn play_count(spec: &ImageSpec, size: u32, cx: &App) -> Option<NonZeroU32> {
    let cache = cx.try_global::<AnimatedCache>()?.0.borrow();
    let size = cache.serving_size(spec, images::rounded_size(size));
    match cache.entries.get(spec)?.get(&size)? {
        Entry::Ready { plays, .. } => *plays,
        _ => None,
    }
}

pub fn state(spec: &ImageSpec, size: u32, cx: &App) -> Load {
    let Some(cache) = cx
        .try_global::<AnimatedCache>()
        .map(|cache| cache.0.clone())
    else {
        return Load::Failed;
    };
    {
        let mut state = cache.borrow_mut();
        let size = state.serving_size(spec, images::rounded_size(size));
        state.clock = state.clock.saturating_add(1);
        let used = state.clock;
        if let Some(entry) = state
            .entries
            .get_mut(spec)
            .and_then(|sizes| sizes.get_mut(&size))
        {
            match entry {
                Entry::Ready {
                    image,
                    used: stamp,
                    seen,
                    ..
                } => {
                    *stamp = used;
                    *seen = Instant::now();
                    let image = image.clone();
                    drop(state);
                    schedule_purge(cache, cx);
                    return Load::Ready(image);
                }
                Entry::Failed => return Load::Failed,
                Entry::Pending { .. } => {}
            }
        }
        state.request(spec, size, Instant::now());
        if state.pump_pending {
            return Load::Pending;
        }
        state.pump_pending = true;
    }
    cx.spawn(async move |cx| {
        cache.borrow_mut().pump_pending = false;
        cx.update(|cx| start_next(cache, cx));
    })
    .detach();
    Load::Pending
}

impl Cache {
    /// The size `spec` is served at for a request of `size`. A profile effect without a ready
    /// decode at that size takes the smallest ready larger one, so cards of different sizes
    /// share one copy instead of holding one each.
    fn serving_size(&self, spec: &ImageSpec, size: u32) -> u32 {
        let Some(sizes) = self.entries.get(spec) else {
            return size;
        };
        if !matches!(spec, ImageSpec::Collectible { .. })
            || matches!(sizes.get(&size), Some(Entry::Ready { .. }))
        {
            return size;
        }
        sizes
            .iter()
            .filter(|&(&ready, entry)| ready > size && matches!(entry, Entry::Ready { .. }))
            .map(|(&ready, _)| ready)
            .min()
            .unwrap_or(size)
    }

    /// Drops animations not shown for `UNSEEN_DROP`; also says whether any are left to check
    /// again later.
    fn purge_unseen(&mut self) -> (Vec<Arc<RenderImage>>, bool) {
        let mut purged = Vec::new();
        let mut remaining = false;
        let mut freed = 0;
        self.entries.retain(|spec, sizes| {
            sizes.retain(|_, entry| match entry {
                Entry::Ready {
                    image,
                    bytes,
                    ready_at,
                    seen,
                    ..
                } if Arc::strong_count(image) == 1 + usize::from(stream::selected(spec))
                    && ready_at.elapsed() >= FRESH_FOR
                    && seen.elapsed() >= UNSEEN_DROP =>
                {
                    freed += *bytes;
                    purged.push(image.clone());
                    false
                }
                Entry::Ready { .. } => {
                    remaining = true;
                    true
                }
                _ => true,
            });
            !sizes.is_empty()
        });
        self.bytes -= freed;
        (purged, remaining)
    }

    fn request(&mut self, spec: &ImageSpec, size: u32, now: Instant) {
        let mut found = false;
        let mut promote = false;
        if let Some(entry) = self
            .entries
            .get_mut(spec)
            .and_then(|sizes| sizes.get_mut(&size))
        {
            found = true;
            if let Entry::Pending {
                requested,
                queued,
                wanted,
            } = entry
            {
                *requested = now;
                *wanted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = now;
                promote = *queued;
            }
        }
        if found {
            if promote {
                self.promote(spec, size);
            }
            return;
        }
        let spec = spec.clone();
        self.entries.entry(spec.clone()).or_default().insert(
            size,
            Entry::Pending {
                requested: now,
                queued: true,
                wanted: Arc::new(Mutex::new(now)),
            },
        );
        self.queue.insert(0, (spec, size));
    }

    fn promote(&mut self, spec: &ImageSpec, size: u32) {
        if let Some(position) = self
            .queue
            .iter()
            .position(|(queued_spec, queued_size)| queued_spec == spec && *queued_size == size)
        {
            let key = self.queue.remove(position);
            self.queue.insert(0, key);
        }
    }

    fn next(&mut self) -> (Option<Key>, usize, Vec<Arc<RenderImage>>) {
        let mut dropped = 0;
        let mut index = 0;
        let selected = loop {
            if index >= self.queue.len() {
                break None;
            }
            let key = self.queue[index].clone();
            let lane = Lane::of(&key.0);
            let fresh = matches!(
                self.entries
                    .get(&key.0)
                    .and_then(|sizes| sizes.get(&key.1)),
                Some(Entry::Pending { requested, .. }) if requested.elapsed() <= STALE_AFTER
            );
            if fresh {
                if self.running[lane as usize] < lane.downloads() {
                    break Some((index, key, lane));
                }
                // Another lane may still have room; a full GIF lane must not block emoji.
                index += 1;
                continue;
            }
            self.queue.remove(index);
            let pending = matches!(
                self.entries.get(&key.0).and_then(|sizes| sizes.get(&key.1)),
                Some(Entry::Pending { .. })
            );
            if pending {
                let empty = if let Some(sizes) = self.entries.get_mut(&key.0) {
                    sizes.remove(&key.1);
                    sizes.is_empty()
                } else {
                    false
                };
                if empty {
                    self.entries.remove(&key.0);
                }
            }
            dropped += 1;
        };
        let Some((index, key, lane)) = selected else {
            return (None, dropped, Vec::new());
        };
        let evicted = self.evict();
        self.queue.remove(index);
        if let Some(Entry::Pending { queued, .. }) = self
            .entries
            .get_mut(&key.0)
            .and_then(|sizes| sizes.get_mut(&key.1))
        {
            *queued = false;
        }
        self.running[lane as usize] += 1;
        (Some(key), dropped, evicted)
    }

    fn wanted_stamp(&self, key: &Key) -> Option<Arc<Mutex<Instant>>> {
        match self.entries.get(&key.0)?.get(&key.1)? {
            Entry::Pending { wanted, .. } => Some(wanted.clone()),
            _ => None,
        }
    }

    fn needs_heartbeat(&self) -> bool {
        self.running.iter().any(|&running| running > 0) || !self.queue.is_empty()
    }

    fn evict(&mut self) -> Vec<Arc<RenderImage>> {
        let mut evicted = Vec::new();
        // The budget is soft: reserve one future decode, and if only fresh or pinned entries
        // are left, start anyway rather than stalling.
        while self.bytes + MAX_IMAGE > MAX_TOTAL {
            let candidate = self
                .entries
                .iter()
                .flat_map(|(spec, sizes)| {
                    sizes.iter().map(move |(size, entry)| (spec, size, entry))
                })
                .filter_map(|(spec, size, entry)| match entry {
                    Entry::Ready {
                        image,
                        used,
                        ready_at,
                        seen,
                        ..
                    } if Arc::strong_count(image) == 1 + usize::from(stream::selected(spec))
                        && ready_at.elapsed() >= FRESH_FOR
                        && seen.elapsed() >= VISIBLE_GRACE =>
                    {
                        Some((spec, size, *used))
                    }
                    _ => None,
                })
                .min_by_key(|(_, _, used)| *used)
                .map(|(spec, size, _)| (spec.clone(), *size));
            let Some((spec, size)) = candidate else {
                break;
            };
            let empty = if let Some(sizes) = self.entries.get_mut(&spec) {
                if let Some(Entry::Ready { image, bytes, .. }) = sizes.remove(&size) {
                    self.bytes -= bytes;
                    evicted.push(image);
                }
                sizes.is_empty()
            } else {
                false
            };
            if empty {
                self.entries.remove(&spec);
            }
        }
        evicted
    }
}

fn schedule_refresh(cache: Rc<RefCell<Cache>>, cx: &App) {
    {
        let mut state = cache.borrow_mut();
        if state.refresh_pending {
            return;
        }
        state.refresh_pending = true;
    }
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(16))
            .await;
        cache.borrow_mut().refresh_pending = false;
        cx.update(|cx| cx.refresh_windows());
    })
    .detach();
}

pub(crate) fn drop_images(images: Vec<Arc<RenderImage>>, cx: &App) {
    if images.is_empty() {
        return;
    }
    cx.spawn(async move |cx| {
        cx.update(|cx| {
            // gpui's sprite atlas keeps every frame until drop_image is called.
            for image in images {
                if !stream::forget(image.id, cx) {
                    cx.drop_image(image, None);
                }
            }
        });
    })
    .detach();
}

/// Checks for unseen animations every `UNSEEN_DROP` while any are cached.
fn schedule_purge(cache: Rc<RefCell<Cache>>, cx: &App) {
    {
        let mut state = cache.borrow_mut();
        if state.purge_scheduled {
            return;
        }
        state.purge_scheduled = true;
    }
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(UNSEEN_DROP).await;
            let (purged, remaining) = {
                let mut state = cache.borrow_mut();
                let (purged, remaining) = state.purge_unseen();
                state.purge_scheduled = remaining;
                (purged, remaining)
            };
            cx.update(|cx| drop_images(purged, cx));
            if !remaining {
                break;
            }
        }
    })
    .detach();
}

/// Repaint while work is in flight so visible still fallbacks keep refreshing their stamps.
fn start_heartbeat(cache: Rc<RefCell<Cache>>, cx: &App) {
    {
        let mut state = cache.borrow_mut();
        if state.heartbeat_running || !state.needs_heartbeat() {
            return;
        }
        state.heartbeat_running = true;
    }
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(HEARTBEAT_INTERVAL).await;
            let busy = {
                let mut state = cache.borrow_mut();
                if state.needs_heartbeat() {
                    true
                } else {
                    state.heartbeat_running = false;
                    false
                }
            };
            if !busy {
                break;
            }
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
}

fn start_next(cache: Rc<RefCell<Cache>>, cx: &App) {
    loop {
        let (key, dropped, evicted) = cache.borrow_mut().next();
        drop_images(evicted, cx);
        if dropped > 0 {
            // Dropped entries may still be visible; repaint so `resolve` re-requests them.
            schedule_refresh(cache.clone(), cx);
        }
        let Some(key) = key else {
            let evicted = {
                let mut state = cache.borrow_mut();
                if state.queue.is_empty() {
                    state.evict()
                } else {
                    Vec::new()
                }
            };
            drop_images(evicted, cx);
            return;
        };
        let lane = Lane::of(&key.0);
        let stamp = {
            let state = cache.borrow();
            state.wanted_stamp(&key)
        };
        let Some(stamp) = stamp else {
            cache.borrow_mut().running[lane as usize] -= 1;
            continue;
        };
        start_heartbeat(cache.clone(), cx);
        let service = images::download_service(cx);
        let request = key.clone();
        let job_stamp = stamp.clone();
        let task = Tokio::spawn_result(cx, async move {
            let service = service.context("download service unavailable")?;
            if !still_wanted(&job_stamp) {
                return Ok(DecodeOutcome::Cancelled);
            }
            let collectible = matches!(&request.0, ImageSpec::Collectible { .. });
            let streaming = stream::selected(&request.0);
            let cached = if matches!(lane, Lane::Gif) && !streaming {
                // A cached animation is as large in memory as a fresh decode, so it waits its
                // turn the same way.
                let _permit = decode_permits(lane)
                    .acquire()
                    .await
                    .context("animation decode permits closed")?;
                if !still_wanted(&job_stamp) {
                    return Ok(DecodeOutcome::Cancelled);
                }
                frames::load(&service.frames_path(&request.0, request.1)).await
            } else {
                None
            };
            if !still_wanted(&job_stamp) {
                return Ok(DecodeOutcome::Cancelled);
            }
            if let Some((image, bytes, plays)) = cached {
                return Ok(DecodeOutcome::Decoded(image, bytes, plays));
            }
            let source_spec = if streaming {
                // Some embeds omit `animated`; never fetch the still-converted cache file.
                stream::source_spec(&request.0)
            } else {
                request.0.clone()
            };
            let path = service
                .load(&source_spec, request.1)
                .await
                .context("animation download failed")?;
            if !still_wanted(&job_stamp) {
                return Ok(DecodeOutcome::Cancelled);
            }
            if streaming {
                let spool = service
                    .frames_path(&request.0, request.1)
                    .with_extension("spool");
                let image = stream::first(path.clone(), request.1, job_stamp.clone()).await?;
                let plays = if collectible {
                    source_play_count(&tokio::fs::read(&path).await?)?
                } else {
                    None
                };
                return Ok(match image {
                    Some(image) if still_wanted(&job_stamp) => {
                        DecodeOutcome::Stream(image, path, spool, plays)
                    }
                    _ => DecodeOutcome::Cancelled,
                });
            }
            let _permit = decode_permits(lane)
                .acquire()
                .await
                .context("animation decode permits closed")?;
            // The tile may have scrolled away while this job waited for a permit.
            if !still_wanted(&job_stamp) {
                return Ok(DecodeOutcome::Cancelled);
            }
            let frames_path = service.frames_path(&request.0, request.1);
            let (image, bytes, plays) = tokio::task::spawn_blocking(move || {
                let source = std::fs::read(path)?;
                let (image, bytes) =
                    decode_with_budget(&source, request.1, MAX_SOURCE_BYTES, MAX_IMAGE)?;
                if matches!(lane, Lane::Gif) {
                    match frames::save(&frames_path, &image, None) {
                        Ok(written) => service.record_frames_write(written),
                        Err(error) => log::debug!(
                            "Could not cache decoded frames at {}: {error}",
                            frames_path.display()
                        ),
                    }
                }
                Ok::<_, anyhow::Error>((image, bytes, None))
            })
            .await
            .context("decode worker failed")??;
            drop(_permit);
            Ok(DecodeOutcome::Decoded(image, bytes, plays))
        });
        let cache = cache.clone();
        cx.spawn(async move |cx| {
            let result = task.await;
            let result = cx.update(|cx| match result {
                Ok(DecodeOutcome::Stream(image, path, spool, plays)) => {
                    if !still_wanted(&stamp) {
                        return Ok(DecodeOutcome::Cancelled);
                    }
                    let bytes =
                        stream::register(&key.0, key.1, path, Some(spool), image.clone(), cx);
                    Ok(DecodeOutcome::Decoded(image, bytes, plays))
                }
                other => other,
            });
            let (success, cancelled) = {
                let mut state = cache.borrow_mut();
                state.running[lane as usize] -= 1;
                match result {
                    Ok(DecodeOutcome::Decoded(image, bytes, plays)) => {
                        state.bytes += bytes;
                        state.clock = state.clock.saturating_add(1);
                        let used = state.clock;
                        let (spec, size) = key;
                        state.entries.entry(spec).or_default().insert(
                            size,
                            Entry::Ready {
                                image,
                                bytes,
                                plays,
                                used,
                                ready_at: Instant::now(),
                                seen: Instant::now(),
                            },
                        );
                        (true, false)
                    }
                    Ok(DecodeOutcome::Cancelled) => {
                        let owned = matches!(
                            state
                                .entries
                                .get(&key.0)
                                .and_then(|sizes| sizes.get(&key.1)),
                            Some(Entry::Pending { wanted, .. }) if Arc::ptr_eq(wanted, &stamp)
                        );
                        if owned {
                            let empty = if let Some(sizes) = state.entries.get_mut(&key.0) {
                                sizes.remove(&key.1);
                                sizes.is_empty()
                            } else {
                                false
                            };
                            if empty {
                                state.entries.remove(&key.0);
                            }
                        }
                        (false, owned)
                    }
                    other => {
                        let reason = match &other {
                            Err(error) => format!("{error:#}"),
                            Ok(_) => "unexpected outcome".to_owned(),
                        };
                        log::warn!(
                            "Animation download or decode failed for {:?} (size {}): {reason}",
                            key.0,
                            key.1
                        );
                        let (spec, size) = key;
                        state
                            .entries
                            .entry(spec)
                            .or_default()
                            .insert(size, Entry::Failed);
                        (false, false)
                    }
                }
            };
            if success || cancelled {
                let cache = cache.clone();
                cx.update(|cx| schedule_refresh(cache, cx));
            }
            if success {
                // It may finish after whoever asked has gone, and never be asked for again.
                let cache = cache.clone();
                cx.update(|cx| schedule_purge(cache, cx));
            }
            cx.update(|cx| start_next(cache, cx));
        })
        .detach();
    }
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits
}

fn still_frames(decoder: impl ImageDecoder) -> Result<Frames<'static>> {
    let frame = Frame::new(DynamicImage::from_decoder(decoder)?.into_rgba8());
    Ok(Frames::new(Box::new(std::iter::once(Ok(frame)))))
}

fn source_play_count(bytes: &[u8]) -> Result<Option<NonZeroU32>> {
    if image::guess_format(bytes)? != ImageFormat::Png {
        return Ok(None);
    }
    let decoder = PngDecoder::with_limits(Cursor::new(bytes), limits())?;
    if !decoder.is_apng()? {
        return Ok(None);
    }
    Ok(match decoder.apng()?.loop_count() {
        LoopCount::Infinite => None,
        LoopCount::Finite(plays) => Some(plays),
    })
}

fn source_frames(bytes: &[u8]) -> Result<((u32, u32), Frames<'_>)> {
    let format = image::guess_format(bytes)?;
    let reader = Cursor::new(bytes);
    match format {
        ImageFormat::Png => apng::frames(reader),
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(reader)?;
            decoder.set_limits(limits())?;
            Ok((decoder.dimensions(), decoder.into_frames()))
        }
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(reader)?;
            decoder.set_limits(limits())?;
            let dimensions = decoder.dimensions();
            if decoder.has_animation() {
                Ok((dimensions, decoder.into_frames()))
            } else {
                Ok((dimensions, still_frames(decoder)?))
            }
        }
        _ => {
            let mut reader = ImageReader::with_format(reader, format);
            reader.limits(limits());
            let decoder = reader.into_decoder()?;
            let dimensions = decoder.dimensions();
            Ok((dimensions, still_frames(decoder)?))
        }
    }
}

fn output_size(width: u32, height: u32, size: u32, frames: usize) -> Result<(u32, u32)> {
    fitted_size(width, height, size, frames, MAX_IMAGE)
}

/// The largest size up to `size` whose `frames` fit in `max_image` bytes.
fn fitted_size(
    width: u32,
    height: u32,
    size: u32,
    frames: usize,
    max_image: usize,
) -> Result<(u32, u32)> {
    ensure!(
        width > 0 && height > 0 && width <= MAX_DIMENSION && height <= MAX_DIMENSION,
        "invalid input dimensions"
    );
    ensure!(size > 0, "empty target size");
    ensure!((1..=MAX_FRAMES).contains(&frames), "invalid frame count");
    let longest = width.max(height);
    let dimensions = |side: u32| {
        (
            (u64::from(width) * u64::from(side) / u64::from(longest)).max(1) as u32,
            (u64::from(height) * u64::from(side) / u64::from(longest)).max(1) as u32,
        )
    };
    let mut low = 1;
    let mut high = size.min(longest);
    while low < high {
        let side = low + (high - low).div_ceil(2);
        let (width, height) = dimensions(side);
        let bytes = u64::from(width) * u64::from(height) * 4 * frames as u64;
        if bytes <= max_image as u64 {
            low = side;
        } else {
            high = side - 1;
        }
    }
    Ok(dimensions(low))
}

fn valid_dimensions(dimensions: (u32, u32)) -> Result<()> {
    ensure!(
        dimensions.0 > 0
            && dimensions.1 > 0
            && dimensions.0 <= MAX_DIMENSION
            && dimensions.1 <= MAX_DIMENSION,
        "invalid input dimensions"
    );
    Ok(())
}

fn process_frame(frame: Frame, target: (u32, u32)) -> Frame {
    let delay = frame.delay();
    let (numerator, denominator) = delay.numer_denom_ms();
    let delay = if u64::from(numerator) < 20 * u64::from(denominator) {
        Delay::from_numer_denom_ms(100, 1)
    } else {
        delay
    };
    let mut pixels = frame.into_buffer();
    let factor = (pixels.width() / target.0).min(pixels.height() / target.1);
    if factor >= 2 {
        pixels = box_shrink(&pixels, factor);
    }
    if pixels.dimensions() != target {
        pixels = resize(&pixels, target.0, target.1, FilterType::Triangle);
    }
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Frame::from_parts(pixels, 0, 0, delay)
}

fn box_shrink(pixels: &RgbaImage, factor: u32) -> RgbaImage {
    let factor = factor as usize;
    let source_width = pixels.width() as usize;
    let width = source_width / factor;
    let height = pixels.height() as usize / factor;
    let raw = pixels.as_raw();
    let area = (factor * factor) as u32;
    let mut output = vec![0; width * height * 4];
    let mut sums = vec![[0u32; 4]; width];
    for y in 0..height {
        sums.fill([0; 4]);
        for offset in 0..factor {
            let row = (y * factor + offset) * source_width * 4;
            for (x, sum) in sums.iter_mut().enumerate() {
                let start = row + x * factor * 4;
                for pixel in raw[start..start + factor * 4].chunks_exact(4) {
                    sum[0] += u32::from(pixel[0]);
                    sum[1] += u32::from(pixel[1]);
                    sum[2] += u32::from(pixel[2]);
                    sum[3] += u32::from(pixel[3]);
                }
            }
        }
        for (x, sum) in sums.iter().enumerate() {
            let out = (y * width + x) * 4;
            for channel in 0..4 {
                output[out + channel] = (sum[channel] / area) as u8;
            }
        }
    }
    RgbaImage::from_raw(width as u32, height as u32, output).expect("shrunk dimensions fit")
}

#[cfg(test)]
fn decode(bytes: &[u8], size: u32) -> Result<(Arc<RenderImage>, usize)> {
    decode_with_budget(bytes, size, MAX_SOURCE_BYTES, MAX_IMAGE)
}

fn decode_with_budget(
    bytes: &[u8],
    size: u32,
    budget: u64,
    max_image: usize,
) -> Result<(Arc<RenderImage>, usize)> {
    let (dimensions, source) = source_frames(bytes)?;
    valid_dimensions(dimensions)?;
    // The target size depends on the real frame count, so hold decoded source frames only
    // until the single pass is done. Above the budget, fall back to counting and resizing
    // in two streaming passes instead of failing.
    let frame_bytes = u64::from(dimensions.0) * u64::from(dimensions.1) * 4;
    let mut source_frames = Vec::new();
    let mut over_budget = false;
    for frame in source {
        ensure!(
            source_frames.len() < MAX_FRAMES,
            "too many animation frames"
        );
        let held = (source_frames.len() as u64 + 1)
            .checked_mul(frame_bytes)
            .context("animation size overflow")?;
        if held > budget {
            over_budget = true;
            break;
        }
        let frame = frame?;
        ensure!(
            frame.buffer().dimensions() == dimensions,
            "frame size changed"
        );
        source_frames.push(frame);
    }
    if over_budget {
        // The decoder went with the loop; free the frames it gave before decoding twice more.
        drop(source_frames);
        return decode_streaming(bytes, size, max_image);
    }
    let count = source_frames.len();
    let target = fitted_size(dimensions.0, dimensions.1, size, count, max_image)?;
    let frames: Vec<_> = source_frames
        .into_iter()
        .map(|frame| process_frame(frame, target))
        .collect();
    let bytes = target.0 as usize * target.1 as usize * 4 * count;
    Ok((Arc::new(RenderImage::new(frames)), bytes))
}

fn decode_streaming(
    bytes: &[u8],
    size: u32,
    max_image: usize,
) -> Result<(Arc<RenderImage>, usize)> {
    let (dimensions, source) = source_frames(bytes)?;
    valid_dimensions(dimensions)?;
    let mut count = 0;
    for frame in source {
        ensure!(count < MAX_FRAMES, "too many animation frames");
        let frame = frame?;
        ensure!(
            frame.buffer().dimensions() == dimensions,
            "frame size changed"
        );
        count += 1;
    }
    let target = fitted_size(dimensions.0, dimensions.1, size, count, max_image)?;
    let (source_dimensions, source) = source_frames(bytes)?;
    ensure!(source_dimensions == dimensions, "frame size changed");
    let mut frames = Vec::with_capacity(count);
    for frame in source {
        ensure!(frames.len() < count, "frame count changed");
        let frame = frame?;
        ensure!(
            frame.buffer().dimensions() == dimensions,
            "frame size changed"
        );
        frames.push(process_frame(frame, target));
    }
    ensure!(frames.len() == count, "frame count changed");
    let bytes = target.0 as usize * target.1 as usize * 4 * count;
    Ok((Arc::new(RenderImage::new(frames)), bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Context, Entity, Render, StyleRefinement, TestAppContext, VisualTestContext, Window, div,
        prelude::*,
    };
    use image::{Rgba, RgbaImage};
    use png::{BitDepth, BlendOp, ColorType, DisposeOp, Encoder};
    use std::cell::Cell;

    #[test]
    fn fallback_selection_freezes_media_but_only_retries_failed_attachments() {
        for animated in [false, true] {
            let spec = ImageSpec::Media {
                url: "https://media.discordapp.net/test.gif".into(),
                width: 8192,
                height: 4096,
                animated,
            };
            let expected = ImageSpec::Media {
                url: "https://media.discordapp.net/test.gif".into(),
                width: 8192,
                height: 4096,
                animated: false,
            };
            for failed in [false, true] {
                assert_eq!(fallback_spec(&spec, failed), Some(expected.clone()));
            }
        }
        let attachment = ImageSpec::Attachment {
            id: 1,
            url: "https://cdn.discordapp.com/test.gif".into(),
            width: 8192,
            height: 4096,
        };
        assert_eq!(fallback_spec(&attachment, false), None);
        assert_eq!(fallback_spec(&attachment, true), Some(attachment));
        assert_eq!(
            fallback_spec(&ImageSpec::DefaultAvatar { index: 0 }, true),
            None
        );
    }

    // 4x2 canvas. Frame 0 is a 2x2 opaque red square at the origin and is disposed to
    // background. Frame 1 is a 2x2 alpha-blended subframe at x=2, with one blue pixel
    // and transparent padding. If disposal is skipped, frame 0's red square remains on
    // frame 1.
    const DISPOSAL_OFFSET_WEBP: &[u8] = &[
        0x52, 0x49, 0x46, 0x46, 0x86, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38,
        0x58, 0x0a, 0x00, 0x00, 0x00, 0x12, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x41, 0x4e, 0x49, 0x4d, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x41,
        0x4e, 0x4d, 0x46, 0x28, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x32, 0x00, 0x00, 0x03, 0x56, 0x50, 0x38, 0x4c, 0x0f, 0x00, 0x00,
        0x00, 0x2f, 0x01, 0x40, 0x00, 0x00, 0x07, 0x10, 0xfd, 0x8f, 0xfe, 0x07, 0x22, 0xa2, 0xff,
        0x01, 0x00, 0x41, 0x4e, 0x4d, 0x46, 0x2a, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x32, 0x00, 0x00, 0x00, 0x56, 0x50, 0x38, 0x4c,
        0x11, 0x00, 0x00, 0x00, 0x2f, 0x01, 0x40, 0x00, 0x10, 0x0f, 0x10, 0x31, 0xff, 0xf3, 0x1f,
        0x8c, 0x09, 0x88, 0xe8, 0x7f, 0x04, 0x00,
    ];

    fn apng(width: u32, height: u32, delays: &[u16], plays: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = Encoder::new(&mut bytes, width, height);
            encoder.set_color(ColorType::Rgba);
            encoder.set_depth(BitDepth::Eight);
            encoder.set_animated(delays.len() as u32, plays).unwrap();
            let mut writer = encoder.write_header().unwrap();
            let pixels = RgbaImage::from_pixel(width, height, Rgba([255, 0, 0, 255]));
            for &delay in delays {
                writer.set_frame_delay(delay, 1000).unwrap();
                writer.write_image_data(pixels.as_raw()).unwrap();
            }
            writer.finish().unwrap();
        }
        bytes
    }

    fn next(cache: &mut Cache) -> Option<Key> {
        let (key, dropped, evicted) = cache.next();
        assert_eq!(dropped, 0, "unexpected stale drop");
        assert!(evicted.is_empty());
        key
    }

    fn blank_image() -> Arc<RenderImage> {
        Arc::new(RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]))
    }

    fn delayed_image(delays: &[u64]) -> RenderImage {
        RenderImage::new(
            delays
                .iter()
                .map(|delay| {
                    Frame::from_parts(
                        RgbaImage::new(1, 1),
                        0,
                        0,
                        Delay::from_numer_denom_ms(*delay as u32, 1),
                    )
                })
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn frame_selection_follows_delays_and_wraps() {
        let image = delayed_image(&[50, 100, 120]);
        let ms = Duration::from_millis;
        for (elapsed, frame, remaining) in [
            (0, 0, 50),
            (49, 0, 1),
            (50, 1, 100),
            (149, 1, 1),
            (150, 2, 120),
            (269, 2, 1),
            (270, 0, 50),
            (305, 0, 15),
            (1000, 2, 80),
        ] {
            assert_eq!(
                frame_at(&image, ms(elapsed), None),
                (frame, ms(remaining)),
                "elapsed {elapsed}ms"
            );
        }
        assert_eq!(
            frame_at(&delayed_image(&[100]), ms(999), None),
            (0, Duration::MAX)
        );
        assert_eq!(
            frame_at(&RenderImage::new(Vec::new()), ms(1), None),
            (0, Duration::ZERO)
        );
    }

    #[test]
    fn one_shot_frames_hold_the_last_frame_instead_of_restarting() {
        let frames = Arc::new(delayed_image(&[50, 100, 120]));
        let ms = Duration::from_millis;
        let started_at = Instant::now();
        assert!(image(frames.clone()).started_at.is_none());
        assert_eq!(
            image(frames.clone())
                .play_from(started_at, Some(NonZeroU32::MIN))
                .started_at,
            Some(started_at)
        );
        for (elapsed, frame, remaining) in [
            (0, 0, ms(50)),
            (50, 1, ms(100)),
            (150, 2, ms(120)),
            (269, 2, ms(1)),
            (270, 2, Duration::MAX),
            (305, 2, Duration::MAX),
            (1000, 2, Duration::MAX),
        ] {
            assert_eq!(
                frame_at(&frames, ms(elapsed), Some(NonZeroU32::MIN)),
                (frame, remaining),
                "elapsed {elapsed}ms"
            );
        }
        assert_eq!(
            frame_at(&delayed_image(&[100]), ms(999), Some(NonZeroU32::MIN)),
            (0, Duration::MAX)
        );
        assert_eq!(
            frame_at(&RenderImage::new(Vec::new()), ms(1), Some(NonZeroU32::MIN)),
            (0, Duration::ZERO)
        );
    }

    #[test]
    fn scheduling_waits_for_the_deadline_unless_it_is_imminent() {
        let ms = Duration::from_millis;
        assert_eq!(wake(ms(0), true, false, true), Wake::AnimationFrame);
        assert_eq!(wake(ms(8), true, false, true), Wake::AnimationFrame);
        assert_eq!(wake(ms(9), true, false, true), Wake::Timer(ms(9)));
        assert_eq!(wake(ms(100), true, false, true), Wake::Timer(ms(100)));
        assert_eq!(wake(ms(100), false, false, true), Wake::None);
        assert_eq!(wake(ms(100), true, true, true), Wake::None);
        assert_eq!(wake(ms(100), true, false, false), Wake::None);
        assert_eq!(wake(Duration::MAX, true, false, true), Wake::None);
    }

    #[test]
    fn finite_play_counts_stop_only_after_the_last_play() {
        let image = delayed_image(&[50, 100]);
        let plays = NonZeroU32::new(2);
        let ms = Duration::from_millis;
        for (elapsed, frame, remaining) in [
            (0, 0, ms(50)),
            (50, 1, ms(100)),
            (150, 0, ms(50)),
            (299, 1, ms(1)),
            (300, 1, Duration::MAX),
            (1000, 1, Duration::MAX),
        ] {
            assert_eq!(frame_at(&image, ms(elapsed), plays), (frame, remaining));
        }
        assert_eq!(frame_at(&image, ms(300), None), (0, ms(50)));
    }

    #[test]
    fn one_wakeup_per_view_keeps_the_earliest_deadline() {
        let mut wakeups = Wakeups::default();
        let view = EntityId::from(1);
        let other = EntityId::from(2);
        let base = Instant::now();
        let at = |ms| base + Duration::from_millis(ms);
        assert!(wakeups.claim(view, at(100)));
        assert!(!wakeups.claim(view, at(100)));
        assert!(!wakeups.claim(view, at(200)));
        assert_eq!(wakeups.pending[&view].deadline, at(100));
        assert!(wakeups.claim(view, at(50)));
        assert_eq!(wakeups.pending[&view].deadline, at(50));
        assert!(wakeups.claim(other, at(75)));
        assert_eq!(wakeups.pending[&other].deadline, at(75));
        assert!(!wakeups.release(view, at(100)));
        assert!(wakeups.release(view, at(50)));
        assert!(!wakeups.pending.contains_key(&view));
        assert!(wakeups.pending.contains_key(&other));
        assert!(wakeups.claim(view, at(25)));
        assert_eq!(wakeups.pending[&view].deadline, at(25));
    }

    fn put_ready(
        cache: &mut Cache,
        key: &Key,
        image: Arc<RenderImage>,
        bytes: usize,
        used: u64,
        ready_at: Instant,
    ) {
        cache.entries.entry(key.0.clone()).or_default().insert(
            key.1,
            Entry::Ready {
                image,
                bytes,
                plays: None,
                used,
                ready_at,
                seen: ready_at,
            },
        );
    }

    fn has(cache: &Cache, key: &Key) -> bool {
        cache
            .entries
            .get(&key.0)
            .is_some_and(|sizes| sizes.contains_key(&key.1))
    }

    fn assert_same_frames(left: &RenderImage, right: &RenderImage) {
        assert_eq!(left.frame_count(), right.frame_count());
        for index in 0..left.frame_count() {
            assert_eq!(left.size(index).width.0, right.size(index).width.0);
            assert_eq!(left.size(index).height.0, right.size(index).height.0);
            assert_eq!(
                left.delay(index).numer_denom_ms(),
                right.delay(index).numer_denom_ms()
            );
            assert_eq!(left.as_bytes(index), right.as_bytes(index));
        }
    }

    #[test]
    fn apng_delays_are_preserved_and_short_delays_clamped() {
        let (image, bytes) = decode(&apng(2, 1, &[50, 0, 120], 0), 2).unwrap();
        assert_eq!(image.frame_count(), 3);
        assert_eq!(bytes, 24);
        assert_eq!(
            (0..3)
                .map(|index| image.delay(index).numer_denom_ms())
                .collect::<Vec<_>>(),
            [(50, 1), (100, 1), (120, 1)]
        );
        let (image, _) = decode(&apng(1, 1, &[19, 20], 0), 1).unwrap();
        assert_eq!(image.delay(0).numer_denom_ms(), (100, 1));
        assert_eq!(image.delay(1).numer_denom_ms(), (20, 1));
    }

    #[test]
    fn collectible_apng_preserves_the_source_play_count() {
        for plays in [0, 1, 2, 17, 0x7FFF_FFFF] {
            let source = apng(1, 1, &[80, 80], plays);
            assert_eq!(source_play_count(&source).unwrap(), NonZeroU32::new(plays));
        }
        // The PNG decoder ignores animation-control values above its 31-bit limit.
        assert_eq!(
            source_play_count(&apng(1, 1, &[80, 80], u32::MAX)).unwrap(),
            None
        );
        assert_eq!(source_play_count(DISPOSAL_OFFSET_WEBP).unwrap(), None);
        assert!(source_play_count(b"not an image").is_err());
    }

    #[tokio::test]
    async fn collectible_play_count_is_restored_alongside_cached_frames() {
        let source = apng(1, 1, &[83; 36], 1);
        let plays = source_play_count(&source).unwrap();
        let (decoded, _) = decode(&source, 1).unwrap();
        let path = std::env::temp_dir().join(format!(
            "zcord-effect-frames-{:016x}.frames",
            rand::random::<u64>()
        ));
        frames::save(&path, &decoded, plays).unwrap();
        drop(source);
        let cached = frames::load(&path).await;
        std::fs::remove_file(&path).unwrap();
        let (cached, _, cached_plays) = cached.unwrap();
        assert_eq!(cached_plays, plays);
        assert_same_frames(&decoded, &cached);
        for (image, plays) in [(decoded, plays), (cached, cached_plays)] {
            assert_eq!(image.frame_count(), 36);
            assert_eq!(frame_at(&image, Duration::from_millis(2880), plays).0, 34);
            for elapsed in [3000, 5760, 8640, 50_000] {
                assert_eq!(
                    frame_at(&image, Duration::from_millis(elapsed), plays),
                    (35, Duration::MAX)
                );
            }
        }
        let repeated = apng(1, 1, &[80; 36], 0);
        let (image, _) = decode(&repeated, 1).unwrap();
        assert_eq!(
            frame_at(
                &image,
                Duration::from_millis(2880),
                source_play_count(&repeated).unwrap()
            ),
            (0, Duration::from_millis(80))
        );
    }

    #[gpui::test]
    fn collectible_cache_hits_keep_the_play_count(cx: &mut TestAppContext) {
        cx.update(init);
        cx.update(|cx| {
            let spec = ImageSpec::Collectible {
                url: "https://cdn.discordapp.com/assets/content/test-effect".into(),
            };
            let source = apng(1, 1, &[80, 80], 1);
            let (image, bytes) = decode(&source, 64).unwrap();
            let cache = cx.global::<AnimatedCache>().0.clone();
            cache
                .borrow_mut()
                .entries
                .entry(spec.clone())
                .or_default()
                .insert(
                    64,
                    Entry::Ready {
                        image: image.clone(),
                        bytes,
                        plays: source_play_count(&source).unwrap(),
                        used: 0,
                        ready_at: Instant::now(),
                        seen: Instant::now(),
                    },
                );
            assert_eq!(play_count(&spec, 64, cx), Some(NonZeroU32::MIN));
            let Load::Ready(cached) = state(&spec, 64, cx) else {
                panic!("the existing cache entry should be ready");
            };
            assert!(Arc::ptr_eq(&image, &cached));
            assert_eq!(play_count(&spec, 64, cx), Some(NonZeroU32::MIN));
        });
    }

    #[test]
    fn red_rgba_becomes_bgra() {
        let (image, _) = decode(&apng(1, 1, &[50], 0), 1).unwrap();
        assert_eq!(image.as_bytes(0).unwrap(), &[0, 0, 255, 255]);
    }

    #[test]
    fn frames_downscale_without_upscaling() {
        let input = apng(64, 32, &[50, 100], 0);
        for (size, width, height) in [(16, 16, 8), (128, 64, 32)] {
            let (image, bytes) = decode(&input, size).unwrap();
            for index in 0..image.frame_count() {
                assert_eq!(image.size(index).width.0, width);
                assert_eq!(image.size(index).height.0, height);
            }
            assert_eq!(bytes, width as usize * height as usize * 4 * 2);
        }
    }

    #[test]
    fn box_shrink_averages_blocks_and_drops_leftovers() {
        let mut pixels = RgbaImage::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                let value = match (x / 2, y / 2) {
                    (0, 0) => [0, 0, 0, 0],
                    (1, 0) => [10, 11, 12, 13],
                    (0, 1) => [20, 21, 22, 23],
                    _ => [255, 255, 255, 255],
                };
                pixels.put_pixel(x, y, Rgba(value));
            }
        }
        for (x, y) in [(2, 2), (2, 3), (3, 2)] {
            pixels.put_pixel(x, y, Rgba([0, 0, 0, 0]));
        }
        let shrunk = box_shrink(&pixels, 2);
        assert_eq!(shrunk.dimensions(), (2, 2));
        assert_eq!(
            shrunk.as_raw(),
            &[0, 0, 0, 0, 10, 11, 12, 13, 20, 21, 22, 23, 63, 63, 63, 63]
        );

        let leftovers = RgbaImage::from_fn(5, 5, |x, y| {
            if x < 4 && y < 4 {
                Rgba([1, 2, 3, 4])
            } else {
                Rgba([200, 200, 200, 200])
            }
        });
        let shrunk = box_shrink(&leftovers, 2);
        assert_eq!(shrunk.dimensions(), (2, 2));
        assert_eq!(shrunk.as_raw(), &[1u8, 2, 3, 4].repeat(4));
    }

    #[test]
    fn process_frame_box_shrinks_large_downscales_to_the_exact_target() {
        let frame = Frame::from_parts(
            RgbaImage::from_pixel(64, 32, Rgba([10, 20, 30, 255])),
            0,
            0,
            Delay::from_numer_denom_ms(50, 1),
        );
        let processed = process_frame(frame, (8, 4));
        assert_eq!(processed.buffer().dimensions(), (8, 4));
        assert_eq!(processed.buffer().get_pixel(0, 0).0, [30, 20, 10, 255]);
    }

    #[test]
    fn apng_background_disposal_and_over_blending_are_composited() {
        for (dispose, expected) in [
            (DisposeOp::Background, [0, 0, 0, 0]),
            (DisposeOp::None, [0, 0, 255, 255]),
        ] {
            let mut bytes = Vec::new();
            {
                let mut encoder = Encoder::new(&mut bytes, 3, 1);
                encoder.set_color(ColorType::Rgba);
                encoder.set_depth(BitDepth::Eight);
                encoder.set_animated(2, 0).unwrap();
                encoder.set_frame_delay(50, 1000).unwrap();
                encoder.set_dispose_op(dispose).unwrap();
                let mut writer = encoder.write_header().unwrap();
                writer
                    .write_image_data(&[255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255])
                    .unwrap();
                writer.set_dispose_op(DisposeOp::None).unwrap();
                writer.set_blend_op(BlendOp::Over).unwrap();
                writer.set_frame_dimension(2, 1).unwrap();
                writer.set_frame_position(1, 0).unwrap();
                writer
                    .write_image_data(&[0, 0, 0, 0, 0, 0, 255, 255])
                    .unwrap();
                writer.finish().unwrap();
            }
            let (image, _) = decode(&bytes, 3).unwrap();
            assert_eq!(image.frame_count(), 2);
            let second = image.as_bytes(1).unwrap();
            assert_eq!(&second[..4], &expected);
            assert_eq!(&second[4..8], &expected);
            assert_eq!(&second[8..], &[255, 0, 0, 255]);
        }
    }

    #[test]
    fn apng_previous_disposal_restores_what_background_disposal_cleared() {
        let mut bytes = Vec::new();
        {
            let mut encoder = Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(ColorType::Rgba);
            encoder.set_depth(BitDepth::Eight);
            encoder.set_animated(3, 0).unwrap();
            encoder.set_frame_delay(50, 1000).unwrap();
            encoder.set_dispose_op(DisposeOp::Background).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0, 255]).unwrap();
            writer.set_dispose_op(DisposeOp::Previous).unwrap();
            writer.write_image_data(&[0, 255, 0, 255]).unwrap();
            writer.set_dispose_op(DisposeOp::None).unwrap();
            writer.set_blend_op(BlendOp::Over).unwrap();
            writer.write_image_data(&[0, 0, 0, 0]).unwrap();
            writer.finish().unwrap();
        }
        let (image, _) = decode(&bytes, 3).unwrap();
        assert_eq!(image.frame_count(), 3);
        assert_eq!(image.as_bytes(1).unwrap(), &[0, 255, 0, 255]);
        assert_eq!(image.as_bytes(2).unwrap(), &[0, 0, 0, 0]);
    }

    #[test]
    fn webp_disposes_first_frame_before_offset_alpha_blend() {
        let (image, bytes) = decode(DISPOSAL_OFFSET_WEBP, 16).unwrap();
        assert_eq!(image.frame_count(), 2);
        assert_eq!(bytes, 4 * 2 * 4 * 2);
        assert_eq!(image.delay(0).numer_denom_ms(), (50, 1));
        assert_eq!(image.delay(1).numer_denom_ms(), (50, 1));

        // Opaque red becomes BGRA after process_frame.
        assert_eq!(&image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);

        let second = image.as_bytes(1).unwrap();
        assert_eq!(&second[..8], &[0u8; 8]);
        // image-webp's fixed-point blend yields 254 for an opaque pixel over a
        // transparent one.
        let blue = &second[8..12];
        assert!(blue[0] >= 254, "blue channel {}", blue[0]);
        assert_eq!(&blue[1..], &[0, 0, 255]);
        assert_eq!(&second[12..16], &[0u8; 4]);
        assert_eq!(&second[16..], &[0u8; 16]);

        let (streamed, streamed_bytes) =
            decode_with_budget(DISPOSAL_OFFSET_WEBP, 16, 0, MAX_IMAGE).unwrap();
        assert_eq!(bytes, streamed_bytes);
        assert_same_frames(&image, &streamed);
    }

    #[test]
    fn plain_png_has_one_frame() {
        let mut bytes = Vec::new();
        {
            let mut encoder = Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(ColorType::Rgba);
            encoder.set_depth(BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0, 255]).unwrap();
            writer.finish().unwrap();
        }
        assert_eq!(source_play_count(&bytes).unwrap(), None);
        let (image, bytes) = decode(&bytes, 16).unwrap();
        assert_eq!(image.frame_count(), 1);
        assert_eq!(image.as_bytes(0).unwrap(), &[0, 0, 255, 255]);
        assert_eq!(bytes, 4);
    }

    #[test]
    fn gif_keeps_all_frames_and_delays() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for (pixel, delay) in [([255, 0, 0, 255], 50), ([0, 0, 255, 255], 10)] {
                encoder
                    .encode_frame(Frame::from_parts(
                        RgbaImage::from_pixel(2, 1, Rgba(pixel)),
                        0,
                        0,
                        Delay::from_numer_denom_ms(delay, 1),
                    ))
                    .unwrap();
            }
        }
        let (image, bytes) = decode(&bytes, 1).unwrap();
        assert_eq!(image.frame_count(), 2);
        assert_eq!(image.as_bytes(0).unwrap(), &[0, 0, 255, 255]);
        assert_eq!(image.as_bytes(1).unwrap(), &[255, 0, 0, 255]);
        assert_eq!(image.delay(0).numer_denom_ms(), (50, 1));
        assert_eq!(image.delay(1).numer_denom_ms(), (100, 1));
        assert_eq!(bytes, 8);
    }

    #[test]
    fn static_webp_and_jpeg_decode() {
        for format in [ImageFormat::WebP, ImageFormat::Jpeg] {
            let mut bytes = Cursor::new(Vec::new());
            let pixels = image::RgbImage::from_pixel(2, 1, image::Rgb([255, 0, 0]));
            DynamicImage::ImageRgb8(pixels)
                .write_to(&mut bytes, format)
                .unwrap();
            let (image, bytes) = decode(bytes.get_ref(), 1).unwrap();
            assert_eq!(image.frame_count(), 1);
            assert_eq!(image.size(0).width.0, 1);
            assert_eq!(image.size(0).height.0, 1);
            assert_eq!(image.delay(0).numer_denom_ms(), (100, 1));
            let pixel = image.as_bytes(0).unwrap();
            assert!(pixel[0] < 5 && pixel[1] < 5 && pixel[2] > 250);
            assert_eq!(pixel[3], 255);
            assert_eq!(bytes, 4);
        }
    }

    #[test]
    fn image_budget_shrinks_size_without_dropping_frames() {
        let (width, height) = output_size(2048, 1024, 2048, 600).unwrap();
        assert!(width < 2048);
        assert_eq!(height, width / 2);
        assert!(width as usize * height as usize * 4 * 600 <= MAX_IMAGE);
        assert!((width + 1) as usize * width.div_ceil(2) as usize * 4 * 600 > MAX_IMAGE);
        assert_eq!(output_size(64, 32, 128, 600).unwrap(), (64, 32));
    }

    #[test]
    fn invalid_dimensions_counts_and_garbage_are_rejected() {
        for bytes in [b"".as_slice(), b"garbage", b"\x89PNG\r\n\x1a\n"] {
            assert!(decode(bytes, 16).is_err());
        }
        assert!(output_size(4097, 1, 16, 1).is_err());
        assert!(output_size(1, 4097, 16, 1).is_err());
        assert!(output_size(0, 1, 16, 1).is_err());
        assert!(output_size(1, 1, 0, 1).is_err());
        assert!(output_size(1, 1, 16, 0).is_err());
        assert!(output_size(1, 1, 16, 601).is_err());
        assert!(decode(&apng(1, 1, &vec![50; 601], 0), 1).is_err());
        assert_eq!(
            decode(&apng(1, 1, &vec![50; 600], 0), 1)
                .unwrap()
                .0
                .frame_count(),
            600
        );
        assert!(decode(&apng(4097, 1, &[50], 0), 16).is_err());
    }

    #[test]
    fn decode_falls_back_to_streaming_with_identical_output() {
        let input = apng(64, 32, &[50, 100, 120], 0);
        let (single, single_bytes) = decode_with_budget(&input, 16, u64::MAX, MAX_IMAGE).unwrap();
        // Two 64x32 frames need 16KB, so one frame fits and the second triggers fallback.
        let budget = u64::from(64_u32 * 32 * 4);
        let (streamed, streamed_bytes) = decode_with_budget(&input, 16, budget, MAX_IMAGE).unwrap();
        assert_eq!(single_bytes, streamed_bytes);
        assert_same_frames(&single, &streamed);
    }

    #[test]
    fn streaming_fallback_keeps_frame_and_dimension_limits() {
        assert!(decode_with_budget(&apng(1, 1, &vec![50; 601], 0), 1, 0, MAX_IMAGE).is_err());
        assert_eq!(
            decode_with_budget(&apng(1, 1, &vec![50; 600], 0), 1, 0, MAX_IMAGE)
                .unwrap()
                .0
                .frame_count(),
            600
        );
        assert!(decode_with_budget(&apng(4097, 1, &[50], 0), 16, 0, MAX_IMAGE).is_err());
    }

    #[test]
    fn cache_uses_all_workers_and_evicts_only_old_unheld_lru_entries() {
        let mut cache = Cache::default();
        let now = Instant::now();
        for index in 0..(DOWNLOADS as u8 + 1) {
            cache.request(&ImageSpec::DefaultAvatar { index }, 16, now);
        }
        for _ in 0..DOWNLOADS {
            assert!(next(&mut cache).is_some());
        }
        assert!(next(&mut cache).is_none());
        assert_eq!(cache.running[Lane::Gif as usize], DOWNLOADS);

        // At max-1 workers a non-empty cache must still admit the last worker.
        cache.running[Lane::Gif as usize] = DOWNLOADS - 1;
        let mb = 1024 * 1024;
        let stale = now - VISIBLE_GRACE - Duration::from_secs(1);
        let held = blank_image();
        let held_key = (ImageSpec::DefaultAvatar { index: 10 }, 16);
        put_ready(&mut cache, &held_key, held.clone(), 64 * mb, 0, stale);
        let older_key = (ImageSpec::DefaultAvatar { index: 11 }, 16);
        put_ready(&mut cache, &older_key, blank_image(), 96 * mb, 1, stale);
        let fresh_key = (ImageSpec::DefaultAvatar { index: 12 }, 16);
        put_ready(
            &mut cache,
            &fresh_key,
            blank_image(),
            80 * mb,
            2,
            Instant::now(),
        );
        cache.bytes = 240 * mb;
        cache.request(&ImageSpec::DefaultAvatar { index: 13 }, 16, now);
        let (key, dropped, evicted) = cache.next();
        assert!(key.is_some());
        assert_eq!(dropped, 0);
        assert_eq!(evicted.len(), 1);
        assert_eq!(cache.running[Lane::Gif as usize], DOWNLOADS);
        assert!(!has(&cache, &older_key));
        assert!(has(&cache, &fresh_key));
        assert!(has(&cache, &held_key));
        assert!(cache.bytes + MAX_IMAGE <= MAX_TOTAL);
    }

    #[test]
    fn cache_starts_without_evicting_fresh_or_held_entries() {
        let mut cache = Cache::default();
        let now = Instant::now();
        let mb = 1024 * 1024;
        let held = blank_image();
        let held_key = (ImageSpec::DefaultAvatar { index: 20 }, 16);
        put_ready(
            &mut cache,
            &held_key,
            held.clone(),
            160 * mb,
            0,
            now - VISIBLE_GRACE - Duration::from_secs(1),
        );
        let fresh_key = (ImageSpec::DefaultAvatar { index: 21 }, 16);
        put_ready(
            &mut cache,
            &fresh_key,
            blank_image(),
            96 * mb,
            1,
            Instant::now(),
        );
        cache.bytes = 256 * mb;
        cache.request(&ImageSpec::DefaultAvatar { index: 22 }, 16, now);
        let (key, dropped, evicted) = cache.next();
        assert!(key.is_some());
        assert_eq!(dropped, 0);
        assert!(evicted.is_empty());
        assert_eq!(cache.running[Lane::Gif as usize], 1);
        assert!(has(&cache, &held_key));
        assert!(has(&cache, &fresh_key));
        assert_eq!(cache.bytes, 256 * mb);

        // Once released, the old held entry is evictable again.
        drop(held);
        cache.request(&ImageSpec::DefaultAvatar { index: 23 }, 16, now);
        let (key, _, evicted) = cache.next();
        assert!(key.is_some());
        assert_eq!(evicted.len(), 1);
        assert!(!has(&cache, &held_key));
        assert!(has(&cache, &fresh_key));
    }

    #[test]
    fn eviction_reports_the_image_for_dropping() {
        let mut cache = Cache::default();
        let now = Instant::now();
        let mb = 1024 * 1024;
        let fresh_key = (ImageSpec::DefaultAvatar { index: 30 }, 16);
        put_ready(
            &mut cache,
            &fresh_key,
            blank_image(),
            96 * mb,
            0,
            Instant::now(),
        );
        let evicted_key = (ImageSpec::DefaultAvatar { index: 31 }, 16);
        let image = blank_image();
        put_ready(
            &mut cache,
            &evicted_key,
            image.clone(),
            160 * mb,
            1,
            now - VISIBLE_GRACE - Duration::from_secs(1),
        );
        let image_ptr = Arc::as_ptr(&image);
        drop(image);
        cache.bytes = 256 * mb;
        cache.request(&ImageSpec::DefaultAvatar { index: 32 }, 16, now);
        let (key, dropped, evicted) = cache.next();
        assert!(key.is_some());
        assert_eq!(dropped, 0);
        assert!(!has(&cache, &evicted_key));
        assert!(has(&cache, &fresh_key));
        assert_eq!(evicted.len(), 1);
        assert_eq!(Arc::as_ptr(&evicted[0]), image_ptr);
        assert_eq!(Arc::strong_count(&evicted[0]), 1);
    }

    #[test]
    fn effects_share_a_larger_ready_decode_and_unseen_animations_go() {
        let mut cache = Cache::default();
        let effect = ImageSpec::Collectible {
            url: "https://cdn.discordapp.com/assets/content/effect".into(),
        };
        let old = Instant::now() - UNSEEN_DROP - Duration::from_secs(1);
        assert_eq!(cache.serving_size(&effect, 600), 600);
        // The stream player holds a registered effect's still image too.
        let registered = blank_image();
        put_ready(
            &mut cache,
            &(effect.clone(), 880),
            registered.clone(),
            100,
            0,
            old,
        );
        cache.bytes = 100;
        assert_eq!(cache.serving_size(&effect, 600), 880);
        assert_eq!(cache.serving_size(&effect, 1200), 1200);
        let avatar = ImageSpec::DefaultAvatar { index: 1 };
        put_ready(
            &mut cache,
            &(avatar.clone(), 880),
            blank_image(),
            10,
            1,
            old,
        );
        assert_eq!(cache.serving_size(&avatar, 600), 600);
        let shown = ImageSpec::DefaultAvatar { index: 2 };
        put_ready(&mut cache, &(shown.clone(), 880), blank_image(), 1, 2, old);
        if let Some(Entry::Ready { seen, .. }) = cache
            .entries
            .get_mut(&shown)
            .and_then(|sizes| sizes.get_mut(&880))
        {
            *seen = Instant::now();
        }
        cache.bytes = 111;

        let (purged, remaining) = cache.purge_unseen();
        assert_eq!(purged.len(), 2);
        assert!(remaining);
        assert!(!has(&cache, &(effect, 880)));
        assert!(!has(&cache, &(avatar, 880)));
        assert!(has(&cache, &(shown, 880)));
        assert_eq!(cache.bytes, 1);
    }

    #[test]
    fn visible_entries_survive_eviction_until_grace_expires() {
        let mut cache = Cache::default();
        let now = Instant::now();
        let mb = 1024 * 1024;
        let old = now - VISIBLE_GRACE - Duration::from_secs(1);
        let visible_key = (ImageSpec::DefaultAvatar { index: 40 }, 16);
        put_ready(&mut cache, &visible_key, blank_image(), 120 * mb, 0, old);
        if let Some(Entry::Ready { seen, .. }) = cache
            .entries
            .get_mut(&visible_key.0)
            .and_then(|sizes| sizes.get_mut(&visible_key.1))
        {
            *seen = Instant::now();
        }
        let unseen_key = (ImageSpec::DefaultAvatar { index: 41 }, 16);
        put_ready(&mut cache, &unseen_key, blank_image(), 120 * mb, 1, old);
        cache.bytes = 240 * mb;
        cache.request(&ImageSpec::DefaultAvatar { index: 42 }, 16, now);
        let (key, _, evicted) = cache.next();
        assert!(key.is_some());
        assert_eq!(evicted.len(), 1);
        assert!(has(&cache, &visible_key));
        assert!(!has(&cache, &unseen_key));

        if let Some(Entry::Ready { seen, .. }) = cache
            .entries
            .get_mut(&visible_key.0)
            .and_then(|sizes| sizes.get_mut(&visible_key.1))
        {
            *seen = old;
        }
        let filler_key = (ImageSpec::DefaultAvatar { index: 43 }, 16);
        put_ready(&mut cache, &filler_key, blank_image(), 120 * mb, 2, old);
        cache.bytes = 240 * mb;
        cache.request(&ImageSpec::DefaultAvatar { index: 44 }, 16, now);
        let (key, _, evicted) = cache.next();
        assert!(key.is_some());
        assert_eq!(evicted.len(), 1);
        assert!(!has(&cache, &visible_key));
        assert!(has(&cache, &filler_key));
    }

    #[test]
    fn newest_request_is_served_first_and_re_request_moves_to_front() {
        let mut cache = Cache::default();
        let now = Instant::now();
        let keys: Vec<_> = (0..3)
            .map(|index| (ImageSpec::DefaultAvatar { index }, 16))
            .collect();
        for key in &keys {
            cache.request(&key.0, key.1, now);
        }
        assert_eq!(next(&mut cache), Some(keys[2].clone()));
        assert_eq!(next(&mut cache), Some(keys[1].clone()));
        cache.running[Lane::Gif as usize] = 0;
        assert_eq!(next(&mut cache), Some(keys[0].clone()));

        let mut cache = Cache::default();
        for key in &keys {
            cache.request(&key.0, key.1, now);
        }
        cache.request(&keys[0].0, keys[0].1, now);
        assert_eq!(next(&mut cache), Some(keys[0].clone()));
    }

    #[test]
    fn stale_requests_are_dropped_without_becoming_failures() {
        let mut cache = Cache::default();
        let now = Instant::now();
        let stale = (ImageSpec::DefaultAvatar { index: 1 }, 16);
        let fresh = (ImageSpec::DefaultAvatar { index: 2 }, 16);
        cache.request(
            &stale.0,
            stale.1,
            now - STALE_AFTER - Duration::from_millis(1),
        );
        cache.request(&fresh.0, fresh.1, now);
        let (key, dropped, evicted) = cache.next();
        assert_eq!(key, Some(fresh));
        assert_eq!(dropped, 0);
        assert!(evicted.is_empty());
        cache.running[Lane::Gif as usize] = 0;
        let (key, dropped, evicted) = cache.next();
        assert_eq!(key, None);
        assert_eq!(dropped, 1);
        assert!(evicted.is_empty());
        assert!(!has(&cache, &stale));
        assert!(!matches!(
            cache
                .entries
                .get(&stale.0)
                .and_then(|sizes| sizes.get(&stale.1)),
            Some(Entry::Failed)
        ));

        let mut cache = Cache::default();
        cache.request(
            &stale.0,
            stale.1,
            now - STALE_AFTER - Duration::from_millis(1),
        );
        let (key, dropped, evicted) = cache.next();
        assert_eq!(key, None);
        assert_eq!(dropped, 1);
        assert!(evicted.is_empty());
        assert!(!has(&cache, &stale));
        assert!(!matches!(
            cache
                .entries
                .get(&stale.0)
                .and_then(|sizes| sizes.get(&stale.1)),
            Some(Entry::Failed)
        ));
    }

    #[test]
    fn requests_refresh_a_running_jobs_wanted_stamp() {
        let mut cache = Cache::default();
        let now = Instant::now();
        let key = (ImageSpec::DefaultAvatar { index: 1 }, 16);
        cache.request(&key.0, key.1, now);
        assert_eq!(next(&mut cache), Some(key.clone()));
        let stamp = cache.wanted_stamp(&key).unwrap();
        *stamp.lock().unwrap() = now - STALE_AFTER - Duration::from_millis(1);
        cache.request(&key.0, key.1, now);
        assert_eq!(*stamp.lock().unwrap(), now);
    }

    #[test]
    fn wanted_jobs_expire_after_the_stale_window() {
        let now = Instant::now();
        let old = now - STALE_AFTER - Duration::from_millis(1);
        let fresh = now - STALE_AFTER + Duration::from_millis(1);
        assert!(wanted(now, now));
        assert!(wanted(fresh, now));
        assert!(wanted(now - STALE_AFTER, now));
        assert!(!wanted(old, now));
    }

    #[test]
    fn queued_small_images_are_served_while_the_gif_lane_is_full() {
        let mut cache = Cache::default();
        let now = Instant::now();
        for index in 0..DOWNLOADS as u8 {
            cache.request(&ImageSpec::DefaultAvatar { index }, 16, now);
        }
        for _ in 0..DOWNLOADS {
            assert!(next(&mut cache).is_some());
        }
        assert_eq!(cache.running[Lane::Gif as usize], DOWNLOADS);
        assert!(next(&mut cache).is_none());

        let emoji = (
            ImageSpec::Emoji {
                id: 1,
                animated: true,
            },
            32,
        );
        cache.request(&emoji.0, emoji.1, now);
        // A fresh GIF request is ahead in the queue; its full lane must not block the emoji.
        cache.request(&ImageSpec::DefaultAvatar { index: 100 }, 16, now);
        assert_eq!(next(&mut cache), Some(emoji));
        assert_eq!(cache.running[Lane::Small as usize], 1);
        assert_eq!(cache.running[Lane::Gif as usize], DOWNLOADS);
    }

    #[test]
    fn heartbeat_runs_until_both_lanes_and_the_queue_are_idle() {
        let mut cache = Cache::default();
        assert!(!cache.needs_heartbeat());

        let key = (
            ImageSpec::Emoji {
                id: 2,
                animated: true,
            },
            32,
        );
        cache.request(&key.0, key.1, Instant::now());
        assert!(cache.needs_heartbeat());
        assert_eq!(next(&mut cache), Some(key));
        assert!(cache.needs_heartbeat());

        cache.running[Lane::Small as usize] = 0;
        assert!(!cache.needs_heartbeat());
        cache.running[Lane::Gif as usize] = 1;
        assert!(cache.needs_heartbeat());
        cache.running[Lane::Gif as usize] = 0;
        assert!(!cache.needs_heartbeat());
    }

    struct AnimationCounter {
        image: Arc<RenderImage>,
        renders: Rc<Cell<usize>>,
    }

    impl Render for AnimationCounter {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);
            div()
                .size(px(32.))
                .child(image(self.image.clone()).size(px(32.)).id("test-animation"))
        }
    }

    struct CanvasPlayback {
        image: Arc<RenderImage>,
        frame: Rc<Cell<usize>>,
    }

    impl Render for CanvasPlayback {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let image = self.image.clone();
            let frame = self.frame.clone();
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let index = window.with_global_id("canvas-playback".into(), |id, window| {
                        frame_index(&image, id, window, cx)
                    });
                    frame.set(index);
                    window
                        .paint_image(bounds, gpui::Corners::default(), image, index, false)
                        .ok();
                },
            )
            .size(px(32.))
        }
    }

    #[gpui::test]
    fn canvas_playback_advances_wraps_and_resets_when_its_image_changes(cx: &mut TestAppContext) {
        cx.update(init);
        FORCE_ACTIVE.set(true);
        let clock = cx.clone();
        let frame = Rc::new(Cell::new(usize::MAX));
        let (view, cx) = cx.add_window_view(|_, _| CanvasPlayback {
            image: Arc::new(delayed_image(&[50, 100])),
            frame: frame.clone(),
        });
        for (elapsed, expected) in [(0, 0), (50, 1), (99, 1), (1, 0), (50, 1)] {
            clock
                .background_executor
                .advance_clock(Duration::from_millis(elapsed));
            view.update(cx, |_, cx| cx.notify());
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            assert_eq!(frame.get(), expected);
        }
        view.update(cx, |view, cx| {
            view.image = blank_image();
            cx.notify();
        });
        for elapsed in [0, 500] {
            clock
                .background_executor
                .advance_clock(Duration::from_millis(elapsed));
            view.update(cx, |_, cx| cx.notify());
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            assert_eq!(frame.get(), 0);
        }
        FORCE_ACTIVE.set(false);
    }

    struct StaticPanel {
        renders: Rc<Cell<usize>>,
    }

    struct ClickableAnimation {
        image: Arc<RenderImage>,
        clicks: Rc<Cell<usize>>,
    }

    impl Render for ClickableAnimation {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            div()
                .id("animated-media-target")
                .size(px(32.))
                .on_click(move |_, _, _| clicks.set(clicks.get() + 1))
                .child(
                    image(self.image.clone())
                        .size(px(32.))
                        .id("clickable-animation"),
                )
        }
    }

    #[gpui::test]
    fn animated_media_keeps_its_parent_clickable_after_repainting(cx: &mut TestAppContext) {
        cx.update(init);
        let clicks = Rc::new(Cell::new(0));
        let clock = cx.clone();
        let (view, cx) = cx.add_window_view(|_, _| ClickableAnimation {
            image: Arc::new(delayed_image(&[100, 100])),
            clicks: clicks.clone(),
        });
        for expected in 1..=3 {
            clock
                .background_executor
                .advance_clock(Duration::from_millis(100));
            view.update(cx, |_, cx| cx.notify());
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            cx.simulate_click(gpui::point(px(16.), px(16.)), gpui::Modifiers::none());
            assert_eq!(clicks.get(), expected);
        }
        let position = gpui::point(px(16.), px(16.));
        cx.simulate_mouse_down(position, gpui::MouseButton::Left, gpui::Modifiers::none());
        clock
            .background_executor
            .advance_clock(Duration::from_millis(100));
        view.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_mouse_up(position, gpui::MouseButton::Left, gpui::Modifiers::none());
        assert_eq!(clicks.get(), 4);
    }

    impl Render for StaticPanel {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);
            div().size(px(32.))
        }
    }

    struct AnimationHost {
        panel: Entity<AnimationCounter>,
        sibling: Entity<StaticPanel>,
    }

    impl Render for AnimationHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(
                    self.panel
                        .clone()
                        .cached(StyleRefinement::default().size(px(32.))),
                )
                .child(
                    self.sibling
                        .clone()
                        .cached(StyleRefinement::default().size(px(32.))),
                )
        }
    }

    #[gpui::test]
    fn hundred_millisecond_frames_render_about_ten_times_per_second(cx: &mut TestAppContext) {
        cx.update(init);
        let image = Arc::new(delayed_image(&[100, 100]));
        let renders = Rc::new(Cell::new(0));
        let counted = renders.clone();
        let sibling_renders = Rc::new(Cell::new(0));
        let counted_sibling = sibling_renders.clone();
        FORCE_ACTIVE.set(true);
        let clock = cx.clone();
        let (_host, cx) = cx.add_window_view(move |_, cx| {
            let panel = cx.new(|_| AnimationCounter {
                image,
                renders: counted,
            });
            let sibling = cx.new(|_| StaticPanel {
                renders: counted_sibling,
            });
            AnimationHost { panel, sibling }
        });
        let draw = |cx: &mut VisualTestContext| {
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        };
        draw(cx);
        let initial = renders.get();
        let sibling_initial = sibling_renders.get();
        assert_eq!(initial, 1);

        // 165 draws per simulated second; only the animation's own wakeups may
        // dirty the cached panel.
        let step = Duration::from_nanos(1_000_000_000 / 165);
        for _ in 0..165 {
            clock.background_executor.advance_clock(step);
            draw(cx);
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
            });
        }
        let counted = renders.get() - initial;
        println!(
            "100ms animation renders per simulated second: {counted}; \
             sibling panel renders: {}",
            sibling_renders.get() - sibling_initial
        );
        assert_eq!(sibling_renders.get(), sibling_initial);
        assert!(
            (8..=12).contains(&counted),
            "a 100ms animation should repaint its view ~10 times per second, got {counted}"
        );
        FORCE_ACTIVE.set(false);
    }
}
