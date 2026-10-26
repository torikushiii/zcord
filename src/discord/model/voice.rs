use super::serde_helpers::*;
use super::*;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct VoiceState {
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub channel_id: Option<u64>,
    #[serde(deserialize_with = "snowflake")]
    pub user_id: u64,
    #[serde(default)]
    pub self_mute: bool,
    #[serde(default)]
    pub self_deaf: bool,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub deaf: bool,
    #[serde(default)]
    pub self_stream: bool,
    #[serde(default)]
    pub self_video: bool,
    #[serde(default)]
    pub member: Option<Member>,
}
