use super::serde_helpers::*;
use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ChannelInfo {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(deserialize_with = "entries")]
    pub channels: Vec<ChannelInfoEntry>,
}

#[derive(Deserialize)]
pub struct ChannelInfoEntry {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    // A status-only update must not clear the session; explicit null ends it.
    #[serde(default, deserialize_with = "nullable")]
    pub voice_start_time: Option<Option<u64>>,
}

#[derive(Deserialize)]
pub struct VoiceChannelStartTimeUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "nullable")]
    pub voice_start_time: Option<Option<u64>>,
}

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
