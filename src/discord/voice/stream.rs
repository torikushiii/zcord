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
}
