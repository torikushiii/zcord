mod cache;
mod decoding;
mod element;
mod wakeups;

#[cfg(windows)]
pub(crate) use cache::drop_images;
use cache::schedule_purge;
#[cfg(test)]
use decoding::decode;
use decoding::{limits, output_size, process_frame, start_next, still_frames, valid_dimensions};
pub use element::{AnimatedImage, image};
pub(crate) use element::{frame_index, poster_image};
use element::{request_wakeup, window_active};
use wakeups::{AnimatedWakeups, Wake, Wakeups, animations_enabled, wake};

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

#[cfg(test)]
thread_local! {
    static FORCE_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn init(cx: &mut App) {
    cx.set_global(AnimatedCache(Rc::new(RefCell::new(Cache::default()))));
    cx.set_global(AnimatedWakeups(Rc::new(RefCell::new(Wakeups::default()))));
    stream::init(cx);
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

#[cfg(test)]
mod tests;
