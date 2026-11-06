use crate::media::animated::AnimatedImage;
use crate::media::images::ImageSpec;
use gpui::{App, RenderImage};
use std::sync::Arc;

#[cfg(windows)]
pub(crate) use platform::Reader;
#[cfg(windows)]
pub(crate) use platform::init;

pub fn resolve(spec: &ImageSpec, size: u32, cx: &App) -> Option<Arc<RenderImage>> {
    #[cfg(windows)]
    {
        platform::resolve(spec, size, cx)
    }
    #[cfg(not(windows))]
    {
        let _ = (spec, size, cx);
        None
    }
}

pub fn poster(spec: &ImageSpec, size: u32, cx: &App) -> Option<AnimatedImage> {
    #[cfg(windows)]
    {
        platform::resolve(spec, size, cx).map(crate::media::animated::poster_image)
    }
    #[cfg(not(windows))]
    {
        let _ = (spec, size, cx);
        None
    }
}

#[cfg(any(windows, test))]
pub(crate) mod animation {
    use image::Delay;
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };

    pub const MAX_CLIP: usize = 32 * 1024 * 1024;
    pub const MAX_FRAMES: usize = 300;
    pub const MAX_FPS: u32 = 15;
    pub const PROMO_MAX_FPS: u32 = 60;

    pub fn max_fps(spec: &crate::media::images::ImageSpec) -> u32 {
        let crate::media::images::ImageSpec::Video { url, .. } = spec else {
            return MAX_FPS;
        };
        let promo = reqwest::Url::parse(url).is_ok_and(|url| {
            url.host_str() == Some("cdn.discordapp.com")
                && (url.path().starts_with("/quests/")
                    || url.path().starts_with("/assets/content/"))
        });
        if promo { PROMO_MAX_FPS } else { MAX_FPS }
    }
    pub const STALE_AFTER: Duration = Duration::from_secs(2);
    pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);

    pub fn wanted(last: Instant, now: Instant) -> bool {
        now.saturating_duration_since(last) <= STALE_AFTER
    }

    pub fn still_wanted(stamp: &Mutex<Instant>) -> bool {
        wanted(
            *stamp
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            Instant::now(),
        )
    }

    pub fn heartbeat_needed(running: usize, queued: bool) -> bool {
        running > 0 || queued
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct CropRect {
        pub x: u32,
        pub y: u32,
        pub width: u32,
        pub height: u32,
    }

    pub fn crop_rect(
        width: u32,
        height: u32,
        aperture: Option<(i32, i32, i32, i32)>,
    ) -> Option<CropRect> {
        if width == 0 || height == 0 {
            return None;
        }
        let Some((x, y, crop_width, crop_height)) = aperture else {
            return Some(CropRect {
                x: 0,
                y: 0,
                width,
                height,
            });
        };
        if crop_width <= 0 || crop_height <= 0 {
            return None;
        }
        let left = i64::from(x).clamp(0, i64::from(width));
        let top = i64::from(y).clamp(0, i64::from(height));
        let right = (i64::from(x) + i64::from(crop_width)).clamp(0, i64::from(width));
        let bottom = (i64::from(y) + i64::from(crop_height)).clamp(0, i64::from(height));
        if right <= left || bottom <= top {
            return None;
        }
        Some(CropRect {
            x: left as u32,
            y: top as u32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        })
    }

    pub enum VideoRead {
        Sample {
            timestamp: i64,
            duration: Option<i64>,
            pixels: Option<image::RgbaImage>,
        },
        Empty,
        End,
    }

    pub trait VideoReader {
        fn read(&mut self, pixels: bool) -> anyhow::Result<VideoRead>;
        fn restart(&mut self) -> anyhow::Result<()>;
    }

    pub struct VideoFrames<R> {
        reader: R,
        max_fps: u32,
        stride: Option<usize>,
        nominal: i64,
        previous: Option<i64>,
        sum: u128,
        observations: u128,
        reads: usize,
        frames: usize,
        ended: bool,
        error: Option<anyhow::Error>,
    }

    impl<R: VideoReader> VideoFrames<R> {
        pub fn new(reader: R, rate: Option<(u32, u32)>, max_fps: u32) -> Self {
            let rate = rate.filter(|&(n, d)| n > 0 && d > 0);
            Self {
                reader,
                max_fps,
                stride: rate.map(|(n, d)| {
                    u64::from(n)
                        .div_ceil(u64::from(d) * u64::from(max_fps))
                        .max(1) as usize
                }),
                nominal: rate.map_or(1_000_000, |(n, d)| {
                    (10_000_000u64 * u64::from(d) / u64::from(n)).max(1) as i64
                }),
                previous: None,
                sum: 0,
                observations: 0,
                reads: 0,
                frames: 0,
                ended: false,
                error: None,
            }
        }

        fn frame(&mut self) -> anyhow::Result<crate::media::animated::stream::SourceRead> {
            use crate::media::animated::stream::SourceRead;
            use anyhow::Context as _;

            if let Some(error) = self.error.take() {
                return Err(error);
            }
            if self.ended || self.frames >= MAX_FRAMES {
                return Ok(SourceRead::End);
            }
            let mut pixels = None;
            let mut first = 0;
            let mut end = 0;
            let mut samples = 0;
            while self.reads < MAX_FRAMES * 20 {
                self.reads += 1;
                match self.reader.read(pixels.is_none()) {
                    Ok(VideoRead::Sample {
                        timestamp,
                        duration,
                        pixels: next,
                    }) => {
                        if let Some(delta) = self
                            .previous
                            .and_then(|previous| timestamp.checked_sub(previous))
                            .filter(|delta| *delta > 0)
                        {
                            self.sum += delta as u128;
                            self.observations += 1;
                        }
                        self.previous = Some(timestamp);
                        if pixels.is_none() {
                            pixels = Some(next.context("video sample has no pixels")?);
                            first = timestamp;
                        }
                        let fallback = self
                            .sum
                            .checked_div(self.observations)
                            .map_or(self.nominal, |average| average.min(i64::MAX as u128) as i64);
                        end = timestamp.saturating_add(
                            duration
                                .filter(|duration| *duration > 0)
                                .unwrap_or(fallback),
                        );
                        samples += 1;
                        let complete = self.stride.map_or_else(
                            || {
                                end.saturating_sub(first)
                                    >= 10_000_000u64.div_ceil(u64::from(self.max_fps)) as i64
                            },
                            |stride| samples >= stride,
                        );
                        if complete {
                            break;
                        }
                    }
                    Ok(VideoRead::Empty) => {}
                    Ok(VideoRead::End) => {
                        self.ended = true;
                        break;
                    }
                    Err(error) => {
                        self.error = Some(error);
                        break;
                    }
                }
            }
            if self.reads >= MAX_FRAMES * 20 {
                self.ended = true;
            }
            match pixels {
                Some(pixels) => {
                    self.frames += 1;
                    let ticks = end.saturating_sub(first).max(200_000) as u64;
                    let nanos = (u128::from(ticks) * 100).min(u128::from(u64::MAX)) as u64;
                    Ok(SourceRead::Frame(image::Frame::from_parts(
                        pixels,
                        0,
                        0,
                        Delay::from_saturating_duration(Duration::from_nanos(nanos)),
                    )))
                }
                None => match self.error.take() {
                    Some(error) => Err(error),
                    None => Ok(SourceRead::End),
                },
            }
        }
    }

    impl<R: VideoReader> crate::media::animated::stream::FrameSource for VideoFrames<R> {
        fn read(&mut self) -> anyhow::Result<crate::media::animated::stream::SourceRead> {
            self.frame()
        }

        fn restart(&mut self) -> anyhow::Result<()> {
            self.reader.restart()?;
            self.previous = None;
            self.sum = 0;
            self.observations = 0;
            self.reads = 0;
            self.frames = 0;
            self.ended = false;
            self.error = None;
            Ok(())
        }

        fn max_frames(&self) -> usize {
            MAX_FRAMES
        }
    }

    pub fn output_size(width: u32, height: u32, size: u32, frames: usize) -> Option<(u32, u32)> {
        if width == 0 || height == 0 || size == 0 {
            return None;
        }
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
            let bytes =
                u128::from(width) * u128::from(height) * 4 * frames.clamp(1, MAX_FRAMES) as u128;
            if bytes <= MAX_CLIP as u128 {
                low = side;
            } else {
                high = side - 1;
            }
        }
        Some(dimensions(low))
    }
}

#[cfg(windows)]
mod platform {
    use super::{animation::*, *};
    use crate::media::{animated::stream, images};
    use anyhow::{Context, Result, ensure};
    use gpui::Global;
    use gpui_tokio::Tokio;
    use image::{
        RgbaImage,
        imageops::{FilterType, resize},
    };
    use std::{
        cell::RefCell,
        collections::{HashMap, VecDeque},
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
        rc::Rc,
        sync::{Mutex, OnceLock},
        time::{Duration, Instant},
    };
    use windows::{
        Win32::{
            Media::MediaFoundation::*,
            System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        },
        core::{Interface, PCWSTR, w},
    };

    const MAX_TOTAL: usize = 256 * 1024 * 1024;
    const VISIBLE_GRACE: Duration = Duration::from_secs(5);
    const DECODES: usize = 2;
    type Key = (ImageSpec, u32);

    enum Entry {
        Pending {
            requested: Instant,
            wanted: Arc<Mutex<Instant>>,
        },
        Ready {
            image: Arc<RenderImage>,
            bytes: usize,
            used: u64,
            seen: Instant,
        },
        Failed,
    }

    #[derive(Default)]
    struct Cache {
        entries: HashMap<Key, Entry>,
        queue: VecDeque<Key>,
        bytes: usize,
        clock: u64,
        running: usize,
        refresh_pending: bool,
        heartbeat_running: bool,
    }

    impl Cache {
        fn needs_heartbeat(&self) -> bool {
            heartbeat_needed(self.running, !self.queue.is_empty())
        }
    }

    enum DecodeOutcome {
        Decoded(Arc<RenderImage>, PathBuf),
        Cancelled,
    }

    struct VideoCache(Rc<RefCell<Cache>>);
    impl Global for VideoCache {}

    pub fn init(cx: &mut App) {
        cx.set_global(VideoCache(Rc::new(RefCell::new(Cache::default()))));
    }

    pub fn resolve(spec: &ImageSpec, size: u32, cx: &App) -> Option<Arc<RenderImage>> {
        if !matches!(spec, ImageSpec::Video { .. } | ImageSpec::LocalVideo { .. }) {
            return None;
        }
        let cache = cx.try_global::<VideoCache>()?.0.clone();
        let key = (spec.clone(), images::rounded_size(size));
        let now = Instant::now();
        {
            let mut state = cache.borrow_mut();
            state.clock = state.clock.saturating_add(1);
            let used = state.clock;
            let pending = match state.entries.get_mut(&key) {
                Some(Entry::Ready {
                    image,
                    used: stamp,
                    seen,
                    ..
                }) => {
                    *stamp = used;
                    *seen = Instant::now();
                    return Some(image.clone());
                }
                Some(Entry::Failed) => return None,
                Some(Entry::Pending { requested, wanted }) => {
                    *requested = now;
                    *wanted
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = now;
                    true
                }
                None => false,
            };
            if pending {
                if let Some(position) = state.queue.iter().position(|queued| queued == &key) {
                    state.queue.remove(position);
                    state.queue.push_front(key);
                }
            } else {
                state.entries.insert(
                    key.clone(),
                    Entry::Pending {
                        requested: now,
                        wanted: Arc::new(Mutex::new(now)),
                    },
                );
                state.queue.push_front(key);
            }
        }
        start_next(cache, cx);
        None
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
        start_heartbeat(cache.clone(), cx);
        let (job, evicted, dropped) = {
            let mut state = cache.borrow_mut();
            let mut dropped = 0;
            loop {
                let Some(key) = state.queue.front().cloned() else {
                    break;
                };
                let stale = match state.entries.get(&key) {
                    Some(Entry::Pending { requested, .. }) => requested.elapsed() > STALE_AFTER,
                    _ => true,
                };
                if !stale {
                    break;
                }
                state.queue.pop_front();
                if matches!(state.entries.get(&key), Some(Entry::Pending { .. })) {
                    state.entries.remove(&key);
                }
                dropped += 1;
            }
            if state.running >= DECODES {
                (None, Vec::new(), dropped)
            } else {
                match state.queue.front().cloned() {
                    None => (None, Vec::new(), dropped),
                    Some(key) => {
                        let mut evicted = Vec::new();
                        let mut blocked = false;
                        while state.bytes > MAX_TOTAL - MAX_CLIP {
                            let oldest = state
                                .entries
                                .iter()
                                .filter_map(|(key, entry)| match entry {
                                    Entry::Ready {
                                        image, used, seen, ..
                                    } if Arc::strong_count(image) == 2
                                        && seen.elapsed() >= VISIBLE_GRACE =>
                                    {
                                        Some((key.clone(), *used))
                                    }
                                    _ => None,
                                })
                                .min_by_key(|(_, used)| *used);
                            let Some((oldest, _)) = oldest else {
                                blocked = true;
                                break;
                            };
                            if let Some(Entry::Ready { image, bytes, .. }) =
                                state.entries.remove(&oldest)
                            {
                                state.bytes -= bytes;
                                evicted.push(image);
                            }
                        }
                        if blocked {
                            (None, evicted, dropped)
                        } else {
                            let stamp = match state.entries.get(&key) {
                                Some(Entry::Pending { wanted, .. }) => Some(wanted.clone()),
                                _ => None,
                            };
                            if let Some(stamp) = stamp {
                                state.queue.pop_front();
                                state.running += 1;
                                (Some((key, stamp)), evicted, dropped)
                            } else {
                                (None, evicted, dropped)
                            }
                        }
                    }
                }
            }
        };
        crate::media::animated::drop_images(evicted, cx);
        if dropped > 0 {
            schedule_refresh(cache.clone(), cx);
        }
        let Some((key, stamp)) = job else {
            return;
        };
        let service = images::download_service(cx);
        let request = key.clone();
        let job_stamp = stamp.clone();
        let task = Tokio::spawn_result(cx, async move {
            let service = service.context("download service unavailable")?;
            if !still_wanted(&job_stamp) {
                return Ok(DecodeOutcome::Cancelled);
            }
            let path = service
                .load(&request.0, request.1)
                .await
                .context("video download failed")?;
            if !still_wanted(&job_stamp) {
                return Ok(DecodeOutcome::Cancelled);
            }
            let max_fps = animation::max_fps(&request.0);
            let image =
                stream::first_video(path.clone(), request.1, max_fps, job_stamp.clone()).await?;
            Ok(match image {
                Some(image) if still_wanted(&job_stamp) => DecodeOutcome::Decoded(image, path),
                _ => DecodeOutcome::Cancelled,
            })
        });
        cx.spawn(async move |cx| {
            let result = task.await;
            let result = cx.update(|cx| match result {
                Ok(DecodeOutcome::Decoded(image, path)) if still_wanted(&stamp) => {
                    let bytes = stream::register(&key.0, key.1, path, None, image.clone(), cx);
                    Ok(Some((image, bytes)))
                }
                Ok(_) => Ok(None),
                Err(error) => Err(error),
            });
            let refresh = {
                let mut state = cache.borrow_mut();
                state.running -= 1;
                match result {
                    Ok(Some((image, bytes))) => {
                        state.bytes += bytes;
                        state.clock = state.clock.saturating_add(1);
                        let used = state.clock;
                        state.entries.insert(
                            key,
                            Entry::Ready {
                                image,
                                bytes,
                                used,
                                seen: Instant::now(),
                            },
                        );
                        true
                    }
                    Ok(None) => {
                        let owned = matches!(
                            state.entries.get(&key),
                            Some(Entry::Pending { wanted, .. }) if Arc::ptr_eq(wanted, &stamp)
                        );
                        if owned {
                            state.entries.remove(&key);
                        }
                        owned
                    }
                    _ => {
                        log::warn!("Video download or decode failed (size {})", key.1);
                        state.entries.insert(key, Entry::Failed);
                        false
                    }
                }
            };
            cx.update(|cx| {
                start_next(cache.clone(), cx);
                if refresh {
                    schedule_refresh(cache.clone(), cx);
                }
            });
        })
        .detach();
    }

    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    struct Foundation;
    impl Drop for Foundation {
        fn drop(&mut self) {
            let _ = unsafe { MFShutdown() };
        }
    }

    fn decode_permit() -> Result<tokio::sync::SemaphorePermit<'static>> {
        static PERMITS: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
        futures::executor::block_on(
            PERMITS
                .get_or_init(|| tokio::sync::Semaphore::new(DECODES))
                .acquire(),
        )
        .context("video decode permits closed")
    }

    pub(crate) struct Reader {
        reader: IMFSourceReader,
        stream: u32,
        layout: FrameLayout,
        target: (u32, u32),
        rate: Option<(u32, u32)>,
        path: PathBuf,
        size: u32,
        ended: bool,
        // Fields drop in declaration order: release MF objects before MFShutdown/COM.
        _foundation: Foundation,
        _com: Com,
        _thread: std::marker::PhantomData<Rc<()>>,
    }

    impl Reader {
        pub(crate) fn open(path: &Path, size: u32) -> Result<Self> {
            let _permit = decode_permit()?;
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
            let com = Com;
            unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)? };
            let foundation = Foundation;
            let mut attributes = None;
            unsafe { MFCreateAttributes(&mut attributes, 1)? };
            let attributes = attributes.context("missing source reader attributes")?;
            unsafe { attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)? };
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let reader = unsafe { MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attributes) }
                .or_else(|_| unsafe {
                    let stream = MFCreateFile(
                        MF_ACCESSMODE_READ,
                        MF_OPENMODE_FAIL_IF_NOT_EXIST,
                        MF_FILEFLAGS_NONE,
                        PCWSTR(wide.as_ptr()),
                    )?;
                    stream
                        .cast::<IMFAttributes>()?
                        .SetString(&MF_BYTESTREAM_CONTENT_TYPE, w!("video/mp4"))?;
                    MFCreateSourceReaderFromByteStream(&stream, &attributes)
                })?;
            let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
            unsafe {
                reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
                reader.SetStreamSelection(stream, true)?;
            }
            let native = unsafe { reader.GetCurrentMediaType(stream)? };
            let rate = unsafe { native.GetUINT64(&MF_MT_FRAME_RATE) }
                .ok()
                .map(unpack);
            let output = unsafe { MFCreateMediaType()? };
            unsafe {
                output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
                reader.SetCurrentMediaType(stream, None, &output)?;
            }
            let layout = current_layout(&reader, stream)?;
            let target = output_size(layout.crop.width, layout.crop.height, size, 4)
                .context("invalid output dimensions")?;
            Ok(Self {
                reader,
                stream,
                layout,
                target,
                rate,
                path: path.to_owned(),
                size,
                ended: false,
                _foundation: foundation,
                _com: com,
                _thread: std::marker::PhantomData,
            })
        }

        pub(crate) fn rate(&self) -> Option<(u32, u32)> {
            self.rate
        }
    }

    impl VideoReader for Reader {
        fn read(&mut self, pixels: bool) -> Result<VideoRead> {
            if self.ended {
                return Ok(VideoRead::End);
            }
            let _permit = decode_permit()?;
            let mut flags = 0;
            let mut timestamp = 0;
            let mut sample = None;
            unsafe {
                self.reader.ReadSample(
                    self.stream,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut timestamp),
                    Some(&mut sample),
                )?
            };
            ensure!(
                flags & MF_SOURCE_READERF_ERROR.0 as u32 == 0,
                "source reader error"
            );
            if flags
                & (MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32
                    | MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED.0 as u32)
                != 0
            {
                self.layout = current_layout(&self.reader, self.stream)?;
            }
            self.ended = flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0;
            let Some(sample) = sample else {
                return Ok(if self.ended {
                    VideoRead::End
                } else {
                    VideoRead::Empty
                });
            };
            let duration = unsafe { sample.GetSampleDuration() }.ok();
            let image = if pixels {
                let buffer = unsafe { sample.ConvertToContiguousBuffer()? };
                let mut image = copy_frame(&buffer, &self.layout)?;
                if self.target != (self.layout.crop.width, self.layout.crop.height) {
                    image = resize(&image, self.target.0, self.target.1, FilterType::Triangle);
                }
                for pixel in image.pixels_mut() {
                    pixel.0[3] = 255;
                }
                Some(image)
            } else {
                None
            };
            Ok(VideoRead::Sample {
                timestamp,
                duration,
                pixels: image,
            })
        }

        fn restart(&mut self) -> Result<()> {
            let mut next = Self::open(&self.path, self.size)?;
            next.target = self.target;
            *self = next;
            Ok(())
        }
    }

    fn unpack(value: u64) -> (u32, u32) {
        ((value >> 32) as u32, value as u32)
    }

    struct FrameLayout {
        width: u32,
        height: u32,
        stride: i32,
        crop: CropRect,
    }

    fn current_layout(reader: &IMFSourceReader, stream: u32) -> Result<FrameLayout> {
        let media_type = unsafe { reader.GetCurrentMediaType(stream)? };
        let (width, height) = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE).map(unpack)? };
        ensure!(width > 0 && height > 0, "empty video dimensions");
        ensure!(
            u128::from(width) * u128::from(height) * 4 <= 128 * 1024 * 1024,
            "native video frame too large"
        );
        let stride = unsafe { media_type.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
            .map(|stride| stride as i32)
            .or_else(|_| unsafe {
                MFGetStrideForBitmapInfoHeader(MFVideoFormat_RGB32.data1, width)
            })?;
        let crop = [MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_GEOMETRIC_APERTURE]
            .into_iter()
            .find_map(|key| {
                let mut bytes = [0u8; std::mem::size_of::<MFVideoArea>()];
                if unsafe { media_type.GetBlobSize(&key) }.ok()? != bytes.len() as u32 {
                    return None;
                }
                let mut length = 0;
                unsafe { media_type.GetBlob(&key, &mut bytes, Some(&mut length)) }.ok()?;
                if length != bytes.len() as u32 {
                    return None;
                }
                let area =
                    unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<MFVideoArea>()) };
                crop_rect(
                    width,
                    height,
                    Some((
                        i32::from(area.OffsetX.value),
                        i32::from(area.OffsetY.value),
                        area.Area.cx,
                        area.Area.cy,
                    )),
                )
            })
            .or_else(|| crop_rect(width, height, None))
            .context("invalid video aperture")?;
        Ok(FrameLayout {
            width,
            height,
            stride,
            crop,
        })
    }

    fn copy_frame(buffer: &IMFMediaBuffer, layout: &FrameLayout) -> Result<RgbaImage> {
        let crop = layout.crop;
        let mut image = RgbaImage::new(crop.width, crop.height);
        let row_bytes = crop.width.checked_mul(4).context("row overflow")?;
        let surface_row_bytes = layout.width.checked_mul(4).context("row overflow")?;
        if let Ok(buffer) = buffer.cast::<IMF2DBuffer>() {
            let mut scanline = std::ptr::null_mut();
            let mut pitch = 0;
            unsafe { buffer.Lock2D(&mut scanline, &mut pitch)? };
            let result: Result<()> = (|| {
                ensure!(
                    !scanline.is_null() && pitch.unsigned_abs() >= surface_row_bytes,
                    "invalid 2D buffer layout"
                );
                let offset =
                    isize::try_from(i64::from(crop.y) * i64::from(pitch) + i64::from(crop.x) * 4)
                        .context("crop offset overflow")?;
                unsafe {
                    MFCopyImage(
                        image.as_mut_ptr(),
                        row_bytes as i32,
                        scanline.offset(offset),
                        pitch,
                        row_bytes,
                        crop.height,
                    )
                }?;
                Ok(())
            })();
            let unlocked = unsafe { buffer.Unlock2D() };
            result?;
            unlocked?;
        } else {
            let mut data = std::ptr::null_mut();
            let mut length = 0;
            unsafe { buffer.Lock(&mut data, None, Some(&mut length))? };
            let pitch = u64::from(layout.stride.unsigned_abs());
            let required = pitch * u64::from(layout.height - 1) + u64::from(surface_row_bytes);
            let result: Result<()> = (|| {
                ensure!(
                    !data.is_null()
                        && pitch >= u64::from(surface_row_bytes)
                        && required <= u64::from(length),
                    "invalid linear buffer layout"
                );
                let top = if layout.stride < 0 {
                    unsafe { data.add((pitch * u64::from(layout.height - 1)) as usize) }
                } else {
                    data
                };
                let offset = isize::try_from(
                    i64::from(crop.y) * i64::from(layout.stride) + i64::from(crop.x) * 4,
                )
                .context("crop offset overflow")?;
                unsafe {
                    MFCopyImage(
                        image.as_mut_ptr(),
                        row_bytes as i32,
                        top.offset(offset),
                        layout.stride,
                        row_bytes,
                        crop.height,
                    )
                }?;
                Ok(())
            })();
            let unlocked = unsafe { buffer.Unlock() };
            result?;
            unlocked?;
        }
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::animation::*;
    use std::{
        sync::Mutex,
        time::{Duration, Instant},
    };

    #[test]
    fn display_aperture_is_clamped_to_the_surface() {
        for (surface, aperture, expected) in [
            ((512, 288), None, Some((0, 0, 512, 288))),
            ((512, 288), Some((0, 0, 498, 280)), Some((0, 0, 498, 280))),
            ((512, 288), Some((7, 4, 498, 280)), Some((7, 4, 498, 280))),
            (
                (512, 288),
                Some((500, 280, 100, 100)),
                Some((500, 280, 12, 8)),
            ),
            ((512, 288), Some((-10, -20, 100, 100)), Some((0, 0, 90, 80))),
            (
                (512, 288),
                Some((10, 20, i32::MAX, i32::MAX)),
                Some((10, 20, 502, 268)),
            ),
            ((512, 288), Some((512, 0, 1, 1)), None),
            ((512, 288), Some((0, 288, 1, 1)), None),
            ((512, 288), Some((-10, 0, 10, 1)), None),
            (
                (512, 288),
                Some((i32::MAX, i32::MAX, i32::MAX, i32::MAX)),
                None,
            ),
            ((512, 288), Some((0, 0, 0, 280)), None),
            ((512, 288), Some((0, 0, 498, -1)), None),
            ((0, 288), None, None),
            ((512, 0), None, None),
            ((u32::MAX, u32::MAX), None, Some((0, 0, u32::MAX, u32::MAX))),
        ] {
            assert_eq!(
                crop_rect(surface.0, surface.1, aperture).map(|rect| (
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height
                )),
                expected
            );
        }
    }
    #[test]
    fn video_jobs_are_wanted_only_within_the_stale_window() {
        let now = Instant::now();
        assert!(wanted(now, now));
        assert!(wanted(now - STALE_AFTER + Duration::from_millis(1), now));
        assert!(wanted(now - STALE_AFTER, now));
        assert!(!wanted(now - STALE_AFTER - Duration::from_millis(1), now));
        let stamp = Mutex::new(now - STALE_AFTER - Duration::from_millis(1));
        assert!(!still_wanted(&stamp));
        *stamp.lock().unwrap() = Instant::now();
        assert!(still_wanted(&stamp));
    }

    #[test]
    fn heartbeat_is_needed_while_running_or_queued() {
        assert!(!heartbeat_needed(0, false));
        assert!(heartbeat_needed(1, false));
        assert!(heartbeat_needed(0, true));
        assert!(heartbeat_needed(2, true));
        assert!(HEARTBEAT_INTERVAL < STALE_AFTER);
    }
    #[test]
    fn output_scaling_obeys_size_and_pixel_budget() {
        assert_eq!(output_size(400, 200, 128, 10), Some((128, 64)));
        assert_eq!(output_size(200, 400, 128, 10), Some((64, 128)));
        assert_eq!(output_size(40, 20, 128, 10), Some((40, 20)));
        assert_eq!(output_size(0, 200, 128, 10), None);
        assert_eq!(output_size(200, 0, 128, 10), None);
        assert_eq!(output_size(200, 100, 0, 10), None);
        assert_eq!(output_size(4096, 4096, 4096, 1), Some((2896, 2896)));
        assert_eq!(output_size(4096, 4096, 4096, MAX_FRAMES), Some((167, 167)));
        for (width, height) in [
            (4096, 4096),
            (1920, 1080),
            (1, u32::MAX),
            (u32::MAX, 1),
            (u32::MAX, u32::MAX),
        ] {
            for frames in [0, 1, 30, MAX_FRAMES, usize::MAX] {
                let (w, h) = output_size(width, height, 4096, frames).unwrap();
                assert!(w <= width && h <= height && w.max(h) <= 4096);
                assert!(w as usize * h as usize * 4 * frames.clamp(1, MAX_FRAMES) <= MAX_CLIP);
            }
        }
    }
}
