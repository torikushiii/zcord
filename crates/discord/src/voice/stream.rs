mod convert;
mod decode;
#[cfg(windows)]
mod decode_windows;
mod h264;
mod handshake;
mod history;
mod pacer;
mod packetize;
pub(super) mod sender;
pub(super) mod share;
pub(super) mod viewer;

use std::sync::{Arc, Mutex};

pub(crate) use convert::nv12_to_bgra;

pub struct StreamFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub type FrameSlot = Arc<Mutex<Option<StreamFrame>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScreenSourceId {
    Monitor(u64),
    Window(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenSource {
    pub id: ScreenSourceId,
    pub name: String,
    pub is_window: bool,
}

/// Go Live quality in Discord's terms: `resolution` is the output height, and 0 keeps the
/// source size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamQuality {
    pub resolution: u32,
    pub fps: u32,
}

impl StreamQuality {
    pub const STANDARD: Self = Self {
        resolution: 720,
        fps: 30,
    };
    #[cfg_attr(not(windows), allow(dead_code))]
    const MAX_SOURCE_WIDTH: u32 = 3840;
    #[cfg_attr(not(windows), allow(dead_code))]
    const MAX_SOURCE_HEIGHT: u32 = 2160;

    /// The encoded size for a captured frame. A preset fits the picture to its 16:9 box,
    /// scaling up as Discord's client does, so a smaller window still streams at the preset's
    /// height; source keeps the window's size, within 4K. NV12 needs both sides even, so an
    /// odd last row or column is dropped first.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn frame_size(self, source_width: u32, source_height: u32) -> (u32, u32) {
        let (width, height) = ((source_width & !1) as usize, (source_height & !1) as usize);
        let (width, height) = if self.resolution == 0 {
            convert::fit_dimensions(
                width,
                height,
                Self::MAX_SOURCE_WIDTH as usize,
                Self::MAX_SOURCE_HEIGHT as usize,
            )
        } else {
            convert::fit_even(
                width,
                height,
                (self.resolution * 16 / 9) as usize,
                self.resolution as usize,
            )
        };
        (width as u32 & !1, height as u32 & !1)
    }

    /// The bitrate to start at, before a frame shows the real height; source counts as large.
    pub(crate) fn initial_bitrate(self) -> u32 {
        max_bitrate(if self.resolution == 0 {
            u32::MAX
        } else {
            self.resolution
        })
    }
}

/// The most a stream of frames `height` lines tall may send, as Discord's client caps it:
/// 3.5 Mbit/s up to 720p and 9 Mbit/s above, whatever the frame rate.
/// This matches the 9 Mbit/s estimate Discord sends back.
pub(crate) fn max_bitrate(height: u32) -> u32 {
    if height > 720 { 9_000_000 } else { 3_500_000 }
}

pub(crate) struct Picture {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub y: Vec<u8>,
    pub uv: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DecodeError {
    Failed,
    TooLarge,
}

pub(crate) trait Decoder: Send {
    fn decode(&mut self, access_unit: &[u8]) -> Result<Vec<Picture>, DecodeError>;
}

pub(crate) fn decoder() -> Result<Box<dyn Decoder>, String> {
    #[cfg(windows)]
    {
        decode_windows::decoder()
    }
    #[cfg(not(windows))]
    {
        Err("Watching streams needs Windows".into())
    }
}

pub(crate) fn stream_key(guild_id: Option<u64>, channel_id: u64, user_id: u64) -> String {
    match guild_id {
        Some(guild_id) => format!("guild:{guild_id}:{channel_id}:{user_id}"),
        None => format!("call:{channel_id}:{user_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_keys_cover_guild_and_dm_calls() {
        assert_eq!(stream_key(Some(10), 20, 30), "guild:10:20:30");
        assert_eq!(stream_key(None, 20, 30), "call:20:30");
    }

    #[test]
    fn stream_quality_sizes_frames_evenly() {
        let quality = |resolution, fps| StreamQuality { resolution, fps };
        assert_eq!(quality(720, 30).frame_size(1920, 1080), (1280, 720));
        assert_eq!(quality(1440, 60).frame_size(1922, 1119), (2476, 1440));
        assert_eq!(quality(720, 30).frame_size(1440, 1080), (960, 720));
        assert_eq!(quality(480, 30).frame_size(1920, 1080), (852, 480));
        assert_eq!(quality(0, 15).frame_size(1365, 767), (1364, 766));
        assert_eq!(quality(0, 15).frame_size(7680, 4320), (3840, 2160));
    }

    #[test]
    fn stream_quality_caps_bitrate_by_size_and_rate() {
        let quality = |resolution, fps| StreamQuality { resolution, fps };
        assert_eq!(StreamQuality::STANDARD.initial_bitrate(), 3_500_000);
        assert_eq!(quality(1080, 60).initial_bitrate(), 9_000_000);
        assert_eq!(quality(0, 15).initial_bitrate(), 9_000_000);
        assert_eq!(max_bitrate(580), 3_500_000);
    }
}
