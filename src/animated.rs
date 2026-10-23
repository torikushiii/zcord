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
    collections::{HashMap, VecDeque},
    io::Cursor,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

const MAX_IMAGE: usize = 96 * 1024 * 1024;
const MAX_TOTAL: usize = 256 * 1024 * 1024;
const MAX_FRAMES: usize = 600;
const MAX_DIMENSION: u32 = 4096;
const MAX_RUNNING: usize = 2;
type Key = (ImageSpec, u32);

enum Entry {
    Pending,
    Ready {
        image: Arc<RenderImage>,
        bytes: usize,
        used: u64,
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
    let cache = cx.try_global::<AnimatedCache>()?.0.clone();
    let key = (spec.clone(), images::rounded_size(size));
    {
        let mut state = cache.borrow_mut();
        state.clock = state.clock.saturating_add(1);
        let used = state.clock;
        match state.entries.get_mut(&key) {
            Some(Entry::Ready {
                image, used: stamp, ..
            }) => {
                *stamp = used;
                return Some(image.clone());
            }
            Some(Entry::Failed) => return None,
            Some(Entry::Pending) => {}
            None => {
                state.entries.insert(key.clone(), Entry::Pending);
                state.queue.push_back(key);
            }
        }
        if state.pump_pending {
            return None;
        }
        state.pump_pending = true;
    }
    cx.spawn(async move |cx| {
        cache.borrow_mut().pump_pending = false;
        cx.update(|cx| start_next(cache, cx));
    })
    .detach();
    None
}

impl Cache {
    fn next(&mut self) -> Option<Key> {
        if self.running >= MAX_RUNNING || self.queue.is_empty() {
            return None;
        }
        let reserved = (self.running + 1) * MAX_IMAGE;
        while self.bytes + reserved > MAX_TOTAL {
            let oldest = self
                .entries
                .iter()
                .filter_map(|(key, entry)| match entry {
                    Entry::Ready { image, used, .. } if Arc::strong_count(image) == 1 => {
                        Some((key.clone(), *used))
                    }
                    _ => None,
                })
                .min_by_key(|(_, used)| *used)?;
            if let Some(Entry::Ready { bytes, .. }) = self.entries.remove(&oldest.0) {
                self.bytes -= bytes;
            }
        }
        self.running += 1;
        self.queue.pop_front()
    }
}

fn start_next(cache: Rc<RefCell<Cache>>, cx: &App) {
    loop {
        let Some(key) = cache.borrow_mut().next() else {
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
            let refresh = {
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
                        }
                    }
                    _ => {
                        log::warn!("Animation download or decode failed (size {})", key.1);
                        Entry::Failed
                    }
                };
                state.entries.insert(key, entry);
                let refresh = success && !state.refresh_pending;
                state.refresh_pending |= refresh;
                refresh
            };
            cx.update(|cx| start_next(cache.clone(), cx));
            if refresh {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                cache.borrow_mut().refresh_pending = false;
                cx.update(|cx| cx.refresh_windows());
            }
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

fn decode(bytes: &[u8], size: u32) -> Result<(Arc<RenderImage>, usize)> {
    let (dimensions, source) = source_frames(bytes)?;
    let mut count = 0;
    for frame in source {
        ensure!(count < MAX_FRAMES, "too many animation frames");
        ensure!(
            frame?.buffer().dimensions() == dimensions,
            "frame size changed"
        );
        count += 1;
    }
    let target = output_size(dimensions.0, dimensions.1, size, count)?;
    let (_, source) = source_frames(bytes)?;
    let mut frames = Vec::with_capacity(count);
    for frame in source {
        ensure!(frames.len() < count, "frame count changed");
        let frame = frame?;
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
        frames.push(Frame::from_parts(pixels, 0, 0, delay));
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
    fn cache_reserves_two_workers_and_evicts_only_unheld_lru_entries() {
        let mut cache = Cache::default();
        for index in 0..3 {
            let key = (ImageSpec::DefaultAvatar { index }, 16);
            cache.entries.insert(key.clone(), Entry::Pending);
            cache.queue.push_back(key);
        }
        assert!(cache.next().is_some());
        assert!(cache.next().is_some());
        assert!(cache.next().is_none());
        assert_eq!(cache.running, 2);

        cache.running = 0;
        let held = Arc::new(RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]));
        let held_key = (ImageSpec::DefaultAvatar { index: 10 }, 16);
        cache.entries.insert(
            held_key.clone(),
            Entry::Ready {
                image: held.clone(),
                bytes: MAX_IMAGE,
                used: 0,
            },
        );
        let older_key = (ImageSpec::DefaultAvatar { index: 11 }, 16);
        let newer_key = (ImageSpec::DefaultAvatar { index: 12 }, 16);
        for (key, used) in [(older_key.clone(), 1), (newer_key.clone(), 2)] {
            cache.entries.insert(
                key,
                Entry::Ready {
                    image: Arc::new(RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))])),
                    bytes: 64 * 1024 * 1024,
                    used,
                },
            );
        }
        cache.bytes = MAX_IMAGE + 128 * 1024 * 1024;
        assert!(cache.next().is_some());
        assert!(!cache.entries.contains_key(&older_key));
        assert!(cache.entries.contains_key(&newer_key));
        assert!(cache.entries.contains_key(&held_key));
        assert!(cache.bytes + cache.running * MAX_IMAGE <= MAX_TOTAL);

        cache
            .queue
            .push_back((ImageSpec::DefaultAvatar { index: 13 }, 16));
        assert!(cache.next().is_none());
        assert!(cache.entries.contains_key(&held_key));
        assert_eq!(cache.bytes, MAX_IMAGE);
        drop(held);
        assert!(cache.next().is_some());
        assert!(!cache.entries.contains_key(&held_key));
        assert!(cache.bytes + cache.running * MAX_IMAGE <= MAX_TOTAL);
    }
}
