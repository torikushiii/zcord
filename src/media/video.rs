use crate::media::images::ImageSpec;
use gpui::{App, RenderImage};
use std::sync::Arc;

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

#[cfg(any(windows, test))]
mod animation {
    use image::Delay;
    use std::time::Duration;

    pub const MAX_CLIP: usize = 64 * 1024 * 1024;
    pub const MAX_FRAMES: usize = 300;

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

    pub fn delays(samples: &[(i64, Option<i64>)]) -> Vec<Delay> {
        let (sum, count) = samples
            .windows(2)
            .fold((0u128, 0u128), |(sum, count), pair| {
                match pair[1].0.checked_sub(pair[0].0).filter(|delta| *delta > 0) {
                    Some(delta) => (sum + delta as u128, count + 1),
                    None => (sum, count),
                }
            });
        let average = sum.checked_div(count).unwrap_or(1_000_000) as i64;
        samples
            .iter()
            .enumerate()
            .map(|(index, &(timestamp, duration))| {
                let ticks = if let Some(next) = samples.get(index + 1) {
                    next.0.saturating_sub(timestamp)
                } else {
                    duration.filter(|duration| *duration > 0).unwrap_or(average)
                };
                let ticks = ticks.max(200_000) as u64;
                let duration = Duration::new(ticks / 10_000_000, (ticks % 10_000_000) as u32 * 100);
                Delay::from_saturating_duration(duration)
            })
            .collect()
    }

    pub fn estimated_frames(duration: Option<u64>, rate: Option<(u32, u32)>) -> usize {
        match (duration, rate) {
            (Some(duration), Some((numerator, denominator)))
                if duration > 0 && numerator > 0 && denominator > 0 =>
            {
                let numerator = u128::from(duration) * u128::from(numerator);
                let denominator = 10_000_000u128 * u128::from(denominator);
                numerator.div_ceil(denominator).clamp(1, MAX_FRAMES as u128) as usize
            }
            _ => MAX_FRAMES,
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
    use crate::media::images;
    use anyhow::{Context, Result, ensure};
    use gpui::Global;
    use gpui_tokio::Tokio;
    use image::{
        Frame, RgbaImage,
        imageops::{FilterType, resize},
    };
    use std::{
        cell::RefCell,
        collections::{HashMap, VecDeque},
        os::windows::ffi::OsStrExt,
        path::Path,
        rc::Rc,
        time::Duration,
    };
    use windows::{
        Win32::{
            Media::MediaFoundation::*,
            System::Com::{
                COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
                StructuredStorage::{PropVariantClear, PropVariantToUInt64},
            },
        },
        core::{Interface, PCWSTR, w},
    };

    const MAX_TOTAL: usize = 256 * 1024 * 1024;
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
        running: bool,
        refresh_pending: bool,
    }

    struct VideoCache(Rc<RefCell<Cache>>);
    impl Global for VideoCache {}

    pub fn init(cx: &mut App) {
        cx.set_global(VideoCache(Rc::new(RefCell::new(Cache::default()))));
    }

    pub fn resolve(spec: &ImageSpec, size: u32, cx: &App) -> Option<Arc<RenderImage>> {
        if !matches!(spec, ImageSpec::Video { .. }) {
            return None;
        }
        let cache = cx.try_global::<VideoCache>()?.0.clone();
        let key = (spec.clone(), images::rounded_size(size));
        {
            let mut state = cache.borrow_mut();
            state.clock = state.clock.saturating_add(1);
            let used = state.clock;
            if let Some(entry) = state.entries.get_mut(&key) {
                match entry {
                    Entry::Ready {
                        image, used: stamp, ..
                    } => {
                        *stamp = used;
                        return Some(image.clone());
                    }
                    Entry::Failed => return None,
                    Entry::Pending => {}
                }
            } else {
                state.entries.insert(key.clone(), Entry::Pending);
                state.queue.push_back(key);
            }
        }
        start_next(cache, cx);
        None
    }

    fn start_next(cache: Rc<RefCell<Cache>>, cx: &App) {
        let key = {
            let mut state = cache.borrow_mut();
            if state.running {
                return;
            }
            let Some(key) = state.queue.front().cloned() else {
                return;
            };
            while state.bytes > MAX_TOTAL - MAX_CLIP {
                let oldest = state
                    .entries
                    .iter()
                    .filter_map(|(key, entry)| match entry {
                        Entry::Ready { image, used, .. } if Arc::strong_count(image) == 1 => {
                            Some((key.clone(), *used))
                        }
                        _ => None,
                    })
                    .min_by_key(|(_, used)| *used);
                let Some((oldest, _)) = oldest else { return };
                if let Some(Entry::Ready { bytes, .. }) = state.entries.remove(&oldest) {
                    state.bytes -= bytes;
                }
            }
            state.queue.pop_front();
            state.running = true;
            key
        };
        let service = images::download_service(cx);
        let request = key.clone();
        let task = Tokio::spawn(cx, async move {
            let service = service.context("download service unavailable")?;
            let path = service
                .load(&request.0, request.1)
                .await
                .context("video download failed")?;
            tokio::task::spawn_blocking(move || decode(&path, request.1))
                .await
                .context("decode worker failed")?
        });
        cx.spawn(async move |cx| {
            let result = task.await;
            let refresh = {
                let mut state = cache.borrow_mut();
                state.running = false;
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
                        log::warn!("Video download or decode failed (size {})", key.1);
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

    fn decode(path: &Path, size: u32) -> Result<(Arc<RenderImage>, usize)> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        let _com = Com;
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)? };
        let _foundation = Foundation;
        let mut attributes = None;
        unsafe { MFCreateAttributes(&mut attributes, 1)? };
        let attributes = attributes.context("missing source reader attributes")?;
        unsafe { attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)? };
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let reader = unsafe { MFCreateSourceReaderFromURL(PCWSTR(path.as_ptr()), &attributes) }
            .or_else(|_| unsafe {
                let stream = MFCreateFile(
                    MF_ACCESSMODE_READ,
                    MF_OPENMODE_FAIL_IF_NOT_EXIST,
                    MF_FILEFLAGS_NONE,
                    PCWSTR(path.as_ptr()),
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
        let duration = unsafe {
            reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
        }
        .ok()
        .and_then(|mut value| {
            let duration = unsafe { PropVariantToUInt64(&value) }.ok();
            let _ = unsafe { PropVariantClear(&mut value) };
            duration
        });
        let output = unsafe { MFCreateMediaType()? };
        unsafe {
            output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            reader.SetCurrentMediaType(stream, None, &output)?;
        }
        let mut layout = current_layout(&reader, stream)?;
        let mut target = output_size(
            layout.crop.width,
            layout.crop.height,
            size,
            estimated_frames(duration, rate),
        )
        .context("invalid output dimensions")?;
        let mut frame_bytes = target.0 as usize * target.1 as usize * 4;
        let mut frame_limit = MAX_FRAMES.min(MAX_CLIP / frame_bytes);
        let mut pixels = Vec::new();
        let mut timing = Vec::new();
        for _ in 0..MAX_FRAMES * 20 {
            let mut flags = 0;
            let mut timestamp = 0;
            let mut sample = None;
            unsafe {
                reader.ReadSample(
                    stream,
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
                layout = current_layout(&reader, stream)?;
                if pixels.is_empty() {
                    target = output_size(
                        layout.crop.width,
                        layout.crop.height,
                        size,
                        estimated_frames(duration, rate),
                    )
                    .context("invalid output dimensions")?;
                    frame_bytes = target.0 as usize * target.1 as usize * 4;
                    frame_limit = MAX_FRAMES.min(MAX_CLIP / frame_bytes);
                }
            }
            if let Some(sample) = sample {
                let duration = unsafe { sample.GetSampleDuration() }.ok();
                let buffer = unsafe { sample.ConvertToContiguousBuffer()? };
                let mut image = copy_frame(&buffer, &layout)?;
                if target != (layout.crop.width, layout.crop.height) {
                    image = resize(&image, target.0, target.1, FilterType::Triangle);
                }
                for pixel in image.pixels_mut() {
                    pixel.0[3] = 255;
                }
                pixels.push(image);
                timing.push((timestamp, duration));
            }
            if pixels.len() >= frame_limit || flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
        }
        ensure!(!pixels.is_empty(), "video has no frames");
        let bytes = pixels.len() * frame_bytes;
        let frames: Vec<Frame> = pixels
            .into_iter()
            .zip(delays(&timing))
            .map(|(image, delay)| Frame::from_parts(image, 0, 0, delay))
            .collect();
        Ok((Arc::new(RenderImage::new(frames)), bytes))
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

    fn milliseconds(samples: &[(i64, Option<i64>)]) -> Vec<f64> {
        delays(samples)
            .iter()
            .map(|delay| {
                let (numerator, denominator) = delay.numer_denom_ms();
                f64::from(numerator) / f64::from(denominator)
            })
            .collect()
    }

    #[test]
    fn timestamp_delays_and_last_sample_duration() {
        assert_eq!(
            milliseconds(&[
                (0, Some(900_000)),
                (333_333, Some(500_000)),
                (1_000_000, Some(400_000))
            ]),
            [33.3333, 66.6667, 40.0]
        );
        assert_eq!(
            milliseconds(&[(0, None), (400_000, None), (1_000_000, None)]),
            [40.0, 60.0, 50.0]
        );
        assert_eq!(milliseconds(&[(0, None)]), [100.0]);
        assert_eq!(milliseconds(&[(0, Some(250_000))]), [25.0]);
        assert!(delays(&[]).is_empty());
    }

    #[test]
    fn short_and_nonmonotonic_delays_are_clamped() {
        assert_eq!(
            milliseconds(&[(10_000, None), (10_001, None), (0, Some(-1))]),
            [20.0, 20.0, 20.0]
        );
        assert_eq!(milliseconds(&[(0, Some(1))]), [20.0]);
        assert_eq!(
            milliseconds(&[(i64::MAX, None), (i64::MIN, Some(0))]),
            [20.0, 100.0]
        );
    }

    #[test]
    fn duration_and_frame_rate_estimates_are_bounded() {
        assert_eq!(estimated_frames(Some(10_000_000), Some((30, 1))), 30);
        assert_eq!(estimated_frames(Some(10_000_001), Some((30, 1))), 31);
        assert_eq!(estimated_frames(Some(10_000_000), Some((30_000, 1001))), 30);
        for duration in [None, Some(0), Some(u64::MAX)] {
            assert_eq!(estimated_frames(duration, Some((u32::MAX, 1))), MAX_FRAMES);
        }
        assert_eq!(estimated_frames(Some(1), Some((1, 1))), 1);
        assert_eq!(estimated_frames(Some(1), Some((0, 1))), MAX_FRAMES);
        assert_eq!(estimated_frames(Some(1), Some((1, 0))), MAX_FRAMES);
        assert_eq!(estimated_frames(Some(1), None), MAX_FRAMES);
    }

    #[test]
    fn output_scaling_obeys_size_and_pixel_budget() {
        assert_eq!(output_size(400, 200, 128, 10), Some((128, 64)));
        assert_eq!(output_size(200, 400, 128, 10), Some((64, 128)));
        assert_eq!(output_size(40, 20, 128, 10), Some((40, 20)));
        assert_eq!(output_size(0, 200, 128, 10), None);
        assert_eq!(output_size(200, 0, 128, 10), None);
        assert_eq!(output_size(200, 100, 0, 10), None);
        assert_eq!(output_size(4096, 4096, 4096, 1), Some((4096, 4096)));
        assert_eq!(output_size(4096, 4096, 4096, MAX_FRAMES), Some((236, 236)));
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
