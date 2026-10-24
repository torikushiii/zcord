use crate::images::{self, ImageSpec};
use anyhow::{Context, Result, ensure};
use gpui::{App, Global, ImageSource, Img, RenderImage, img};
use gpui_tokio::Tokio;
use image::{
    AnimationDecoder, Delay, DynamicImage, Frame, Frames, ImageDecoder, ImageFormat, ImageReader,
    Limits,
    codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder},
    imageops::{FilterType, resize},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    io::Cursor,
    rc::Rc,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

const MAX_IMAGE: usize = 96 * 1024 * 1024;
const MAX_TOTAL: usize = 256 * 1024 * 1024;
const MAX_FRAMES: usize = 600;
const MAX_DIMENSION: u32 = 4096;
const MAX_RUNNING_CAP: usize = 6;
const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const STALE_AFTER: Duration = Duration::from_secs(2);
/// A just-decoded image may still be waiting for its debounced repaint.
const FRESH_FOR: Duration = Duration::from_secs(1);
type Key = (ImageSpec, u32);

pub enum Load {
    Pending,
    Ready(Arc<RenderImage>),
    Failed,
}

fn max_running() -> usize {
    static RUNNING: OnceLock<usize> = OnceLock::new();
    *RUNNING.get_or_init(|| {
        std::thread::available_parallelism()
            .map_or(2, |parallelism| parallelism.get().saturating_sub(1))
            .clamp(2, MAX_RUNNING_CAP)
    })
}

enum Entry {
    Pending {
        /// Last `state`/`resolve` request for this key, used to drop abandoned work.
        requested: Instant,
        queued: bool,
    },
    Ready {
        image: Arc<RenderImage>,
        bytes: usize,
        used: u64,
        ready_at: Instant,
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
    running: usize,
    pump_pending: bool,
    refresh_pending: bool,
}

struct AnimatedCache(Rc<RefCell<Cache>>);
impl Global for AnimatedCache {}

pub fn init(cx: &mut App) {
    cx.set_global(AnimatedCache(Rc::new(RefCell::new(Cache::default()))));
}

pub fn image(image: Arc<RenderImage>) -> Img {
    img(ImageSource::Custom(Arc::new(move |window, _| {
        window.with_global_id("animated-image-pin".into(), |id, window| {
            // GPUI drops image sources after drawing; element state keeps visible frames pinned.
            window.with_element_state(id, |_, _| ((), image.clone()));
        });
        Some(Ok(image.clone()))
    })))
}

pub fn resolve(spec: &ImageSpec, size: u32, cx: &App) -> Option<Arc<RenderImage>> {
    match state(spec, size, cx) {
        Load::Ready(image) => Some(image),
        Load::Pending | Load::Failed => None,
    }
}

pub fn state(spec: &ImageSpec, size: u32, cx: &App) -> Load {
    let Some(cache) = cx
        .try_global::<AnimatedCache>()
        .map(|cache| cache.0.clone())
    else {
        return Load::Failed;
    };
    let size = images::rounded_size(size);
    {
        let mut state = cache.borrow_mut();
        state.clock = state.clock.saturating_add(1);
        let used = state.clock;
        if let Some(entry) = state
            .entries
            .get_mut(spec)
            .and_then(|sizes| sizes.get_mut(&size))
        {
            match entry {
                Entry::Ready {
                    image, used: stamp, ..
                } => {
                    *stamp = used;
                    return Load::Ready(image.clone());
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
    fn request(&mut self, spec: &ImageSpec, size: u32, now: Instant) {
        let mut found = false;
        let mut promote = false;
        if let Some(entry) = self
            .entries
            .get_mut(spec)
            .and_then(|sizes| sizes.get_mut(&size))
        {
            found = true;
            if let Entry::Pending { requested, queued } = entry {
                *requested = now;
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

    fn next(&mut self) -> (Option<Key>, usize) {
        if self.running >= max_running() {
            return (None, 0);
        }
        let mut dropped = 0;
        loop {
            let Some(key) = self.queue.first().cloned() else {
                return (None, dropped);
            };
            let stale = match self.entries.get(&key.0).and_then(|sizes| sizes.get(&key.1)) {
                Some(Entry::Pending { requested, .. }) => requested.elapsed() > STALE_AFTER,
                _ => true,
            };
            if !stale {
                break;
            }
            self.queue.remove(0);
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
        }
        let Some(key) = self.queue.first().cloned() else {
            return (None, dropped);
        };
        // The budget is soft: reserve one future decode, and if only fresh or pinned entries
        // are left, start anyway rather than stalling the queue.
        while self.bytes + MAX_IMAGE > MAX_TOTAL {
            let evict = self
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
                        ..
                    } if Arc::strong_count(image) == 1 && ready_at.elapsed() >= FRESH_FOR => {
                        Some((spec, size, *used))
                    }
                    _ => None,
                })
                .min_by_key(|(_, _, used)| *used)
                .map(|(spec, size, _)| (spec.clone(), *size));
            let Some((spec, size)) = evict else {
                break;
            };
            let empty = if let Some(sizes) = self.entries.get_mut(&spec) {
                if let Some(Entry::Ready { bytes, .. }) = sizes.remove(&size) {
                    self.bytes -= bytes;
                }
                sizes.is_empty()
            } else {
                false
            };
            if empty {
                self.entries.remove(&spec);
            }
        }
        self.queue.remove(0);
        if let Some(Entry::Pending { queued, .. }) = self
            .entries
            .get_mut(&key.0)
            .and_then(|sizes| sizes.get_mut(&key.1))
        {
            *queued = false;
        }
        self.running += 1;
        (Some(key), dropped)
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

fn start_next(cache: Rc<RefCell<Cache>>, cx: &App) {
    loop {
        let (key, dropped) = cache.borrow_mut().next();
        if dropped > 0 {
            // Dropped entries may still be visible; repaint so `resolve` re-requests them.
            schedule_refresh(cache.clone(), cx);
        }
        let Some(key) = key else {
            return;
        };
        let service = images::download_service(cx);
        let request = key.clone();
        let task = Tokio::spawn(cx, async move {
            let service = service.context("download service unavailable")?;
            let path = service
                .load(&request.0, request.1)
                .await
                .context("animation download failed")?;
            tokio::task::spawn_blocking(move || {
                let bytes = std::fs::read(path)?;
                decode(&bytes, request.1)
            })
            .await
            .context("decode worker failed")?
        });
        let cache = cache.clone();
        cx.spawn(async move |cx| {
            let result = task.await;
            let success = {
                let mut state = cache.borrow_mut();
                state.running -= 1;
                let success = matches!(result, Ok(Ok(_)));
                let entry = match result {
                    Ok(Ok((image, bytes))) => {
                        state.bytes += bytes;
                        state.clock = state.clock.saturating_add(1);
                        Entry::Ready {
                            image,
                            bytes,
                            used: state.clock,
                            ready_at: Instant::now(),
                        }
                    }
                    _ => {
                        log::warn!("Animation download or decode failed (size {})", key.1);
                        Entry::Failed
                    }
                };
                let (spec, size) = key;
                state.entries.entry(spec).or_default().insert(size, entry);
                success
            };
            if success {
                let cache = cache.clone();
                cx.update(|cx| schedule_refresh(cache, cx));
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

fn source_frames(bytes: &[u8]) -> Result<((u32, u32), Frames<'_>)> {
    let format = image::guess_format(bytes)?;
    let reader = Cursor::new(bytes);
    match format {
        ImageFormat::Png => {
            let decoder = PngDecoder::with_limits(reader, limits())?;
            let dimensions = decoder.dimensions();
            if decoder.is_apng()? {
                Ok((dimensions, decoder.apng()?.into_frames()))
            } else {
                Ok((dimensions, still_frames(decoder)?))
            }
        }
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
        if bytes <= MAX_IMAGE as u64 {
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
    if pixels.dimensions() != target {
        pixels = resize(&pixels, target.0, target.1, FilterType::Triangle);
    }
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Frame::from_parts(pixels, 0, 0, delay)
}

fn decode(bytes: &[u8], size: u32) -> Result<(Arc<RenderImage>, usize)> {
    decode_with_budget(bytes, size, MAX_SOURCE_BYTES)
}

fn decode_with_budget(bytes: &[u8], size: u32, budget: u64) -> Result<(Arc<RenderImage>, usize)> {
    let (dimensions, source) = source_frames(bytes)?;
    valid_dimensions(dimensions)?;
    // The target size depends on the real frame count, so hold decoded source frames only
    // until the single pass is done. Above the budget, fall back to counting and resizing
    // in two streaming passes instead of failing.
    let frame_bytes = u64::from(dimensions.0) * u64::from(dimensions.1) * 4;
    let mut source_frames = Vec::new();
    for frame in source {
        ensure!(
            source_frames.len() < MAX_FRAMES,
            "too many animation frames"
        );
        let held = (source_frames.len() as u64 + 1)
            .checked_mul(frame_bytes)
            .context("animation size overflow")?;
        if held > budget {
            return decode_streaming(bytes, size);
        }
        let frame = frame?;
        ensure!(
            frame.buffer().dimensions() == dimensions,
            "frame size changed"
        );
        source_frames.push(frame);
    }
    let count = source_frames.len();
    let target = output_size(dimensions.0, dimensions.1, size, count)?;
    let frames: Vec<_> = source_frames
        .into_iter()
        .map(|frame| process_frame(frame, target))
        .collect();
    let bytes = target.0 as usize * target.1 as usize * 4 * count;
    Ok((Arc::new(RenderImage::new(frames)), bytes))
}

fn decode_streaming(bytes: &[u8], size: u32) -> Result<(Arc<RenderImage>, usize)> {
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
    let target = output_size(dimensions.0, dimensions.1, size, count)?;
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
    use image::{Rgba, RgbaImage};
    use png::{BitDepth, BlendOp, ColorType, DisposeOp, Encoder};

    fn apng(width: u32, height: u32, delays: &[u16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = Encoder::new(&mut bytes, width, height);
            encoder.set_color(ColorType::Rgba);
            encoder.set_depth(BitDepth::Eight);
            encoder.set_animated(delays.len() as u32, 0).unwrap();
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
        let (key, dropped) = cache.next();
        assert_eq!(dropped, 0, "unexpected stale drop");
        key
    }

    fn blank_image() -> Arc<RenderImage> {
        Arc::new(RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]))
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
                used,
                ready_at,
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
        let (image, bytes) = decode(&apng(2, 1, &[50, 0, 120]), 2).unwrap();
        assert_eq!(image.frame_count(), 3);
        assert_eq!(bytes, 24);
        assert_eq!(
            (0..3)
                .map(|index| image.delay(index).numer_denom_ms())
                .collect::<Vec<_>>(),
            [(50, 1), (100, 1), (120, 1)]
        );
        let (image, _) = decode(&apng(1, 1, &[19, 20]), 1).unwrap();
        assert_eq!(image.delay(0).numer_denom_ms(), (100, 1));
        assert_eq!(image.delay(1).numer_denom_ms(), (20, 1));
    }

    #[test]
    fn red_rgba_becomes_bgra() {
        let (image, _) = decode(&apng(1, 1, &[50]), 1).unwrap();
        assert_eq!(image.as_bytes(0).unwrap(), &[0, 0, 255, 255]);
    }

    #[test]
    fn frames_downscale_without_upscaling() {
        let input = apng(64, 32, &[50, 100]);
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
        assert!(decode(&apng(1, 1, &vec![50; 601]), 1).is_err());
        assert_eq!(
            decode(&apng(1, 1, &vec![50; 600]), 1)
                .unwrap()
                .0
                .frame_count(),
            600
        );
        assert!(decode(&apng(4097, 1, &[50]), 16).is_err());
    }

    #[test]
    fn decode_falls_back_to_streaming_with_identical_output() {
        let input = apng(64, 32, &[50, 100, 120]);
        let (single, single_bytes) = decode_with_budget(&input, 16, u64::MAX).unwrap();
        // Two 64x32 frames need 16KB, so one frame fits and the second triggers fallback.
        let budget = u64::from(64_u32 * 32 * 4);
        let (streamed, streamed_bytes) = decode_with_budget(&input, 16, budget).unwrap();
        assert_eq!(single_bytes, streamed_bytes);
        assert_same_frames(&single, &streamed);
    }

    #[test]
    fn streaming_fallback_keeps_frame_and_dimension_limits() {
        assert!(decode_with_budget(&apng(1, 1, &vec![50; 601]), 1, 0).is_err());
        assert_eq!(
            decode_with_budget(&apng(1, 1, &vec![50; 600]), 1, 0)
                .unwrap()
                .0
                .frame_count(),
            600
        );
        assert!(decode_with_budget(&apng(4097, 1, &[50]), 16, 0).is_err());
    }

    #[test]
    fn cache_uses_all_workers_and_evicts_only_old_unheld_lru_entries() {
        let mut cache = Cache::default();
        let now = Instant::now();
        for index in 0..(max_running() as u8 + 1) {
            cache.request(&ImageSpec::DefaultAvatar { index }, 16, now);
        }
        for _ in 0..max_running() {
            assert!(next(&mut cache).is_some());
        }
        assert!(next(&mut cache).is_none());
        assert_eq!(cache.running, max_running());

        // At max-1 workers a non-empty cache must still admit the last worker.
        cache.running = max_running().saturating_sub(1);
        let mb = 1024 * 1024;
        let stale = now - FRESH_FOR - Duration::from_secs(1);
        let held = blank_image();
        let held_key = (ImageSpec::DefaultAvatar { index: 10 }, 16);
        put_ready(&mut cache, &held_key, held.clone(), 64 * mb, 0, stale);
        let older_key = (ImageSpec::DefaultAvatar { index: 11 }, 16);
        put_ready(&mut cache, &older_key, blank_image(), 80 * mb, 1, stale);
        let fresh_key = (ImageSpec::DefaultAvatar { index: 12 }, 16);
        put_ready(
            &mut cache,
            &fresh_key,
            blank_image(),
            56 * mb,
            2,
            Instant::now(),
        );
        cache.bytes = 200 * mb;
        cache.request(&ImageSpec::DefaultAvatar { index: 13 }, 16, now);
        let (key, dropped) = cache.next();
        assert!(key.is_some());
        assert_eq!(dropped, 0);
        assert_eq!(cache.running, max_running());
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
            now - FRESH_FOR - Duration::from_secs(1),
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
        let (key, dropped) = cache.next();
        assert!(key.is_some());
        assert_eq!(dropped, 0);
        assert_eq!(cache.running, 1);
        assert!(has(&cache, &held_key));
        assert!(has(&cache, &fresh_key));
        assert_eq!(cache.bytes, 256 * mb);

        // Once released, the old held entry is evictable again.
        drop(held);
        cache.request(&ImageSpec::DefaultAvatar { index: 23 }, 16, now);
        let (key, _) = cache.next();
        assert!(key.is_some());
        assert!(!has(&cache, &held_key));
        assert!(has(&cache, &fresh_key));
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
        cache.running = 0;
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
        assert_eq!(cache.next(), (Some(fresh), 0));
        cache.running = 0;
        assert_eq!(cache.next(), (None, 1));
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
        assert_eq!(cache.next(), (None, 1));
        assert!(!has(&cache, &stale));
        assert!(!matches!(
            cache
                .entries
                .get(&stale.0)
                .and_then(|sizes| sizes.get(&stale.1)),
            Some(Entry::Failed)
        ));
    }
}
