use super::serde_helpers::*;
use super::*;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Default, Deserialize)]
pub struct Session {
    #[serde(default, deserialize_with = "default_value")]
    pub session_id: String,
    #[serde(default, deserialize_with = "default_value")]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub activities: Vec<Activity>,
    #[serde(default, deserialize_with = "default_value")]
    pub active: bool,
}

#[derive(Clone, Default, Deserialize)]
pub struct Presence {
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub activities: Option<Vec<Activity>>,
}

#[derive(Clone, Default, Deserialize)]
pub struct Activity {
    #[serde(default, rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub details: Option<String>,
    #[serde(default)]
    pub emoji: Option<ActivityEmoji>,
    #[serde(default, deserialize_with = "optional_id")]
    pub application_id: Option<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub platform: Option<String>,
    /// Bitfield; `1 << 8` marks an embedded Discord Activity.
    #[serde(default, deserialize_with = "default_value")]
    pub flags: u64,
    /// Unix ms. Used as the timer start when `timestamps.start` is missing.
    #[serde(default, deserialize_with = "optional_id")]
    pub created_at: Option<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub timestamps: Option<ActivityTimestamps>,
    #[serde(default, deserialize_with = "default_value")]
    pub assets: Option<ActivityAssets>,
}

#[derive(Clone, Default, Deserialize)]
pub struct ActivityAssets {
    #[serde(default, deserialize_with = "default_value")]
    pub large_image: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub large_text: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub small_image: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub small_text: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
pub struct ActivityTimestamps {
    #[serde(default, deserialize_with = "optional_id")]
    pub start: Option<u64>,
}

#[derive(Clone, Default, Deserialize)]
pub struct ActivityEmoji {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub animated: bool,
}

#[derive(Default, Deserialize)]
pub struct MergedPresences {
    #[serde(default, deserialize_with = "entry_groups")]
    pub guilds: Vec<Vec<Presence>>,
    #[serde(default, deserialize_with = "entries")]
    pub friends: Vec<Presence>,
}

#[derive(Default, Deserialize)]
pub struct ReadySupplemental {
    #[serde(default)]
    pub guilds: Vec<Value>,
    #[serde(default, deserialize_with = "entries")]
    pub users: Vec<User>,
    #[serde(default, deserialize_with = "entry_groups")]
    pub merged_members: Vec<Vec<PartialMember>>,
    #[serde(default, deserialize_with = "default_value")]
    pub merged_presences: MergedPresences,
}
