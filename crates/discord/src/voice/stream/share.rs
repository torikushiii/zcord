#[cfg(windows)]
mod audio_windows;
#[cfg(windows)]
mod capture_windows;
#[cfg(windows)]
mod encode_windows;
#[cfg(windows)]
mod icon_windows;

use super::{
    FrameSlot, ScreenSource, StreamFrame, StreamQuality,
    convert::{fit_dimensions, scale_bgra},
};
#[cfg(windows)]
use super::{ScreenSourceId, convert::bgra_to_nv12, max_bitrate};
use base64::Engine as _;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32},
    mpsc::SyncSender,
};
#[cfg(windows)]
use std::{
    sync::{atomic::Ordering, mpsc::sync_channel},
    time::{Duration, Instant},
};

const MIN_BITRATE: u32 = 500_000;
#[cfg(windows)]
const PREVIEW_EVERY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const PREVIEW_WIDTH: usize = 512;
const PREVIEW_HEIGHT: usize = 288;
const PREVIEW_QUALITY: u8 = 92;

pub(crate) type PreviewSink = Box<dyn FnMut(String) + Send>;

pub(crate) struct AudioFrame {
    pub opus: Vec<u8>,
    pub sample: u64,
}

pub(crate) struct LiveShare {
    picture: Mutex<(ScreenSource, StreamQuality)>,
    version: AtomicU32,
    pub audio: AtomicBool,
    #[cfg_attr(not(windows), allow(dead_code))]
    local: Arc<LocalPreview>,
}

#[derive(Default)]
pub(crate) struct LocalPreview {
    pub frames: FrameSlot,
    pub bounds: Mutex<Option<(u32, u32)>>,
}

impl LiveShare {
    pub(crate) fn new(
        source: ScreenSource,
        quality: StreamQuality,
        audio: bool,
        local: Arc<LocalPreview>,
    ) -> Arc<Self> {
        Arc::new(Self {
            picture: Mutex::new((source, quality)),
            version: AtomicU32::new(0),
            audio: AtomicBool::new(audio),
            local,
        })
    }

    pub(crate) fn change(&self, source: Option<ScreenSource>, quality: Option<StreamQuality>) {
        let mut picture = self
            .picture
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(source) = source {
            picture.0 = source;
        }
        if let Some(quality) = quality {
            picture.1 = quality;
        }
        self.version
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    fn current(&self) -> (ScreenSource, StreamQuality, u32) {
        let picture = self
            .picture
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let version = self.version.load(std::sync::atomic::Ordering::Acquire);
        (picture.0.clone(), picture.1, version)
    }
}

pub(crate) fn spawn_audio(
    frames: SyncSender<AudioFrame>,
    stop: Arc<AtomicBool>,
    live: Arc<LiveShare>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        audio_windows::spawn(frames, stop, live)
    }
    #[cfg(not(windows))]
    {
        let _ = (frames, stop, live);
        Err("Stream audio needs Windows".into())
    }
}

pub(crate) struct EncodedFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub timestamp: u32,
    pub keyframe: bool,
}

#[cfg(windows)]
pub(crate) struct RawFrame {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub data: Vec<u8>,
}

pub(crate) fn sources() -> Vec<ScreenSource> {
    #[cfg(windows)]
    {
        capture_windows::sources()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

pub(crate) fn thumbnail(source: &ScreenSource) -> Option<StreamFrame> {
    #[cfg(windows)]
    {
        let raw = capture_windows::thumbnail(source)?;
        let (width, height) = fit_dimensions(raw.width as usize, raw.height as usize, 480, 270);
        if width == 0 || height == 0 {
            return None;
        }
        let bgra = scale_bgra(
            &raw.data,
            raw.stride,
            raw.width as usize,
            raw.height as usize,
            width,
            height,
        );
        Some(StreamFrame {
            width: width as u32,
            height: height as u32,
            bgra,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = source;
        None
    }
}

pub(crate) fn icon(source: &ScreenSource) -> Option<StreamFrame> {
    #[cfg(windows)]
    {
        match source.id {
            ScreenSourceId::Window(id) => icon_windows::window_icon(id),
            ScreenSourceId::Monitor(_) => None,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = source;
        None
    }
}

pub(crate) fn spawn(
    live: Arc<LiveShare>,
    bitrate: Arc<AtomicU32>,
    keyframe: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    frames: SyncSender<EncodedFrame>,
    failure: Arc<Mutex<Option<String>>>,
    previews: PreviewSink,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::thread::Builder::new()
            .name("stream-share".into())
            .spawn(move || {
                let result = run(&live, &bitrate, &keyframe, &stop, &frames, previews);
                if let Err(reason) = result {
                    *failure
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
                }
                stop.store(true, Ordering::Release);
            })
            .map_err(|_| "Could not start the screen capture thread".to_owned())?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (live, bitrate, keyframe, stop, frames, failure, previews);
        Err("Sharing needs Windows".into())
    }
}

#[cfg(windows)]
fn run(
    live: &LiveShare,
    bitrate: &AtomicU32,
    keyframe: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    frames: &SyncSender<EncodedFrame>,
    mut previews: PreviewSink,
) -> Result<(), String> {
    let origin = Instant::now();
    let mut preview_due = Instant::now();
    while !stop.load(Ordering::Acquire) {
        let (source, quality, version) = live.current();
        let picture = Picture {
            source: &source,
            quality,
            version,
            origin,
        };
        if !capture(
            live,
            &picture,
            bitrate,
            keyframe,
            stop,
            frames,
            &mut previews,
            &mut preview_due,
        )? {
            break;
        }
        keyframe.store(true, Ordering::Release);
        preview_due = Instant::now();
    }
    Ok(())
}

#[cfg(windows)]
struct Picture<'a> {
    source: &'a ScreenSource,
    quality: StreamQuality,
    version: u32,
    origin: Instant,
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn capture(
    live: &LiveShare,
    picture: &Picture,
    bitrate: &AtomicU32,
    keyframe: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    frames: &SyncSender<EncodedFrame>,
    previews: &mut PreviewSink,
    preview_due: &mut Instant,
) -> Result<bool, String> {
    let Picture {
        source,
        quality,
        version,
        origin,
    } = *picture;
    let (raw_sender, raw_receiver) = sync_channel(1);
    let _capture = capture_windows::Capture::start(source, raw_sender, stop.clone(), quality.fps)?;
    let mut encoder = None;
    let mut size = (0, 0);
    let mut applied_bitrate = 0;
    let frame_interval = Duration::from_secs_f64(1. / f64::from(quality.fps.max(1)));
    let mut last_frame = Instant::now() - Duration::from_secs(1);
    let mut stats = PipelineStats::new();
    while !stop.load(Ordering::Acquire) {
        if live.version.load(Ordering::Acquire) != version {
            return Ok(true);
        }
        stats.log_if_due();
        let raw = match raw_receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => frame,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        stats.captured += 1;
        let now = Instant::now();
        if now >= *preview_due && !is_black(&raw.data, raw.stride, raw.width, raw.height) {
            if let Some(thumbnail) = preview(&raw.data, raw.stride, raw.width, raw.height) {
                (*previews)(thumbnail);
            }
            *preview_due = now + PREVIEW_EVERY;
        }
        if now.duration_since(last_frame) < frame_interval * 3 / 4 {
            continue;
        }
        last_frame = now;
        if encoder.is_none() {
            size = quality.frame_size(raw.width, raw.height);
            if size.0 == 0 || size.1 == 0 {
                continue;
            }
        }
        let target = bitrate
            .load(Ordering::Acquire)
            .clamp(MIN_BITRATE, max_bitrate(size.1));
        if encoder.is_none() {
            encoder = Some(encode_windows::Encoder::new(
                target,
                size.0,
                size.1,
                quality.fps,
            )?);
            applied_bitrate = target;
        } else if target != applied_bitrate {
            encoder.as_mut().expect("encoder").set_bitrate(target)?;
            applied_bitrate = target;
        }
        stats.raw_size = (raw.width, raw.height);
        stats.resized += u32::from(raw.width & !1 > size.0 || raw.height & !1 > size.1);
        let converting = Instant::now();
        let nv12 = bgra_to_nv12(
            &raw.data,
            raw.stride,
            raw.width as usize,
            raw.height as usize,
            size.0 as usize,
            size.1 as usize,
        );
        stats.convert += converting.elapsed();
        let encoding = Instant::now();
        let force = keyframe.swap(false, Ordering::AcqRel);
        let (data, is_keyframe) = encoder.as_mut().expect("encoder").encode(&nv12, force)?;
        stats.encode += encoding.elapsed();
        stats.encoded += 1;
        if force && !is_keyframe {
            keyframe.store(true, Ordering::Release);
        }
        let local_bounds = *live
            .local
            .bounds
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(bounds) = local_bounds
            && let Some(frame) = local_frame(&raw.data, raw.stride, raw.width, raw.height, bounds)
        {
            *live
                .local
                .frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(frame);
        }
        if data.is_empty() {
            stats.empty += 1;
            continue;
        }
        let sending = Instant::now();
        let timestamp = (origin.elapsed().as_micros() * 90 / 1000) as u32;
        // Wait for the sender rather than drop an encoded frame: later frames reference it, so a
        // drop costs a keyframe, and at high resolutions each keyframe outlasts several frame
        // intervals, which would force the next one. Capture frames are skipped meanwhile.
        if frames
            .send(EncodedFrame {
                data,
                width: size.0,
                height: size.1,
                fps: quality.fps,
                timestamp,
                keyframe: is_keyframe,
            })
            .is_err()
        {
            break;
        }
        stats.send_wait += sending.elapsed();
    }
    Ok(false)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn is_black(bgra: &[u8], stride: usize, width: u32, height: u32) -> bool {
    let row_bytes = width as usize * 4;
    (0..height as usize).step_by(7).all(|row| {
        bgra.get(row * stride..row * stride + row_bytes)
            .is_none_or(|row| {
                row.as_chunks::<4>()
                    .0
                    .iter()
                    .step_by(7)
                    .all(|pixel| pixel[0].max(pixel[1]).max(pixel[2]) <= 10)
            })
    })
}

#[cfg_attr(not(windows), allow(dead_code))]
fn preview(bgra: &[u8], stride: usize, width: u32, height: u32) -> Option<String> {
    let (width, height) = (width as usize, height as usize);
    let (small_width, small_height) = fit_dimensions(width, height, PREVIEW_WIDTH, PREVIEW_HEIGHT);
    if small_width == 0 || small_height == 0 {
        return None;
    }
    let small = scale_bgra(bgra, stride, width, height, small_width, small_height);
    let rgb: Vec<u8> = small
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| [pixel[2], pixel[1], pixel[0]])
        .collect();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, PREVIEW_QUALITY)
        .encode(
            &rgb,
            small_width as u32,
            small_height as u32,
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn local_frame(
    bgra: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    (max_width, max_height): (u32, u32),
) -> Option<StreamFrame> {
    let (width, height) = (width as usize, height as usize);
    let (small_width, small_height) =
        fit_dimensions(width, height, max_width as usize, max_height as usize);
    if small_width == 0 || small_height == 0 {
        return None;
    }
    Some(StreamFrame {
        width: small_width as u32,
        height: small_height as u32,
        bgra: scale_bgra(bgra, stride, width, height, small_width, small_height),
    })
}

#[cfg(windows)]
struct PipelineStats {
    since: Instant,
    captured: u32,
    encoded: u32,
    empty: u32,
    raw_size: (u32, u32),
    resized: u32,
    convert: Duration,
    encode: Duration,
    send_wait: Duration,
}

#[cfg(windows)]
impl PipelineStats {
    const INTERVAL: Duration = Duration::from_secs(5);

    fn new() -> Self {
        Self {
            since: Instant::now(),
            captured: 0,
            encoded: 0,
            empty: 0,
            raw_size: (0, 0),
            resized: 0,
            convert: Duration::ZERO,
            encode: Duration::ZERO,
            send_wait: Duration::ZERO,
        }
    }

    fn log_if_due(&mut self) {
        let elapsed = self.since.elapsed();
        if elapsed < Self::INTERVAL {
            return;
        }
        let seconds = elapsed.as_secs_f64();
        let per_frame =
            |total: Duration| total.as_secs_f64() * 1000. / f64::from(self.encoded.max(1));
        log::debug!(
            "stream capture: {}x{} captured, {} resized; {:.1} fps captured, {:.1} fps encoded, {} empty outputs; per frame: convert {:.1} ms, encode {:.1} ms, waiting on sender {:.1} ms",
            self.raw_size.0,
            self.raw_size.1,
            self.resized,
            f64::from(self.captured) / seconds,
            f64::from(self.encoded) / seconds,
            self.empty,
            per_frame(self.convert),
            per_frame(self.encode),
            per_frame(self.send_wait),
        );
        *self = Self::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::stream::ScreenSourceId;

    #[test]
    fn black_pictures_wait_for_the_first_drawn_one() {
        let black = vec![0u8; 16 * 16 * 4];
        assert!(is_black(&black, 64, 16, 16));
        let mut drawn = black.clone();
        drawn[0..4].copy_from_slice(&[0, 0, 200, 255]);
        assert!(!is_black(&drawn, 64, 16, 16));
    }

    #[test]
    fn previews_are_jpeg_data_uris_fitted_to_512_by_288() {
        let picture = [40u8, 80, 160, 255].repeat(1920 * 1080);
        let thumbnail = preview(&picture, 1920 * 4, 1920, 1080).unwrap();
        let encoded = thumbnail.strip_prefix("data:image/jpeg;base64,").unwrap();
        let jpeg = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let image = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((image.width(), image.height()), (512, 288));
    }

    #[test]
    fn non_windows_has_no_sources() {
        if cfg!(not(windows)) {
            assert!(sources().is_empty());
        }
    }

    #[test]
    fn screen_source_ids_distinguish_windows_and_monitors() {
        assert_ne!(ScreenSourceId::Monitor(1), ScreenSourceId::Window(1));
    }
}
