use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize)]
pub struct Pin {
    #[serde(deserialize_with = "timestamp")]
    pub pinned_at: String,
    pub message: Message,
}

#[derive(Clone)]
pub struct PinPage {
    pub items: Vec<Pin>,
    pub has_more: bool,
}

#[derive(Clone)]
pub struct ThreadPage {
    pub threads: Vec<Channel>,
    pub has_more: bool,
}

#[derive(Clone, Deserialize)]
pub struct ThreadMember {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub join_timestamp: Option<String>,
    #[serde(default)]
    pub member: Option<Member>,
}

#[derive(Clone, Deserialize)]
pub struct ThreadMembersUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default)]
    pub member_count: u32,
    #[serde(default, deserialize_with = "entries")]
    pub added_members: Vec<ThreadMember>,
    #[serde(default, deserialize_with = "ids")]
    pub removed_member_ids: Vec<u64>,
}

#[derive(Clone, Deserialize)]
pub struct ThreadMemberUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub member_count: Option<u32>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub join_timestamp: Option<String>,
    #[serde(default)]
    pub member: Option<Member>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ThreadSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
}

#[derive(Clone, Deserialize)]
pub struct ChannelPinsUpdate {
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub last_pin_timestamp: Option<String>,
}
