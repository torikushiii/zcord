#[cfg(windows)]
mod capture_windows;
#[cfg(windows)]
mod encode_windows;

use super::{ScreenSource, StreamFrame};
#[cfg(windows)]
use super::{
    ScreenSourceId,
    convert::{bgra_to_nv12, fit_dimensions, scale_bgra},
};
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

const WIDTH: usize = 1280;
const HEIGHT: usize = 720;
const FPS: u32 = 30;
const CAP_BITRATE: u32 = 3_500_000;
const MIN_BITRATE: u32 = 500_000;

pub(crate) struct EncodedFrame {
    pub data: Vec<u8>,
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

pub(crate) fn spawn(
    source: ScreenSource,
    bitrate: Arc<AtomicU32>,
    keyframe: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    frames: SyncSender<EncodedFrame>,
    failure: Arc<Mutex<Option<String>>>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::thread::Builder::new()
            .name("stream-share".into())
            .spawn(move || {
                if let Err(reason) = run(source, &bitrate, &keyframe, &stop, frames) {
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
        let _ = (source, bitrate, keyframe, stop, frames, failure);
        Err("Sharing needs Windows".into())
    }
}

#[cfg(windows)]
fn run(
    source: ScreenSource,
    bitrate: &AtomicU32,
    keyframe: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    frames: SyncSender<EncodedFrame>,
) -> Result<(), String> {
    let (raw_sender, raw_receiver) = sync_channel(1);
    let _capture =
        capture_windows::Capture::start(&source, raw_sender, stop.clone(), keyframe.clone())?;
    let mut encoder = None;
    let mut applied_bitrate = 0;
    let origin = Instant::now();
    let mut last_frame = Instant::now() - Duration::from_secs(1);
    while !stop.load(Ordering::Acquire) {
        let raw = match raw_receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(frame) => frame,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let now = Instant::now();
        if now.duration_since(last_frame) < Duration::from_secs_f64(1. / f64::from(FPS)) {
            continue;
        }
        last_frame = now;
        let target = bitrate
            .load(Ordering::Acquire)
            .clamp(MIN_BITRATE, CAP_BITRATE);
        if encoder.is_none() {
            encoder = Some(encode_windows::Encoder::new(target)?);
            applied_bitrate = target;
        } else if target != applied_bitrate {
            encoder.as_mut().expect("encoder").set_bitrate(target)?;
            applied_bitrate = target;
        }
        let nv12 = bgra_to_nv12(
            &raw.data,
            raw.stride,
            raw.width as usize,
            raw.height as usize,
            WIDTH,
            HEIGHT,
        );
        let force = keyframe.swap(false, Ordering::AcqRel);
        let (data, is_keyframe) = encoder.as_mut().expect("encoder").encode(&nv12, force)?;
        if force && !is_keyframe {
            keyframe.store(true, Ordering::Release);
        }
        if data.is_empty() {
            continue;
        }
        let timestamp = (origin.elapsed().as_micros() * 90 / 1000) as u32;
        if frames
            .try_send(EncodedFrame {
                data,
                timestamp,
                keyframe: is_keyframe,
            })
            .is_err()
        {
            keyframe.store(true, Ordering::Release);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::voice::stream::ScreenSourceId;

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
