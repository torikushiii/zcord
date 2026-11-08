use super::*;
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Deserialize)]
pub struct SavedMessage {
    #[serde(default, deserialize_with = "lenient_message")]
    pub message: Option<Message>,
    pub save_data: SaveData,
}

#[derive(Clone, Deserialize)]
pub struct SaveData {
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub message_id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub saved_at: Option<String>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub due_at: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
pub struct SavedMessages {
    #[serde(default, deserialize_with = "entries")]
    pub reminders: Vec<SavedMessage>,
    #[serde(default, deserialize_with = "ids")]
    pub bookmark_ids: Vec<u64>,
}

#[derive(Clone, Default, Deserialize)]
pub struct BookmarkPage {
    #[serde(default, deserialize_with = "entries")]
    pub results: Vec<SavedMessage>,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedMessageRemoved {
    pub channel_id: u64,
    pub message_id: u64,
}

impl SavedMessageRemoved {
    pub fn from_value(value: &Value) -> Option<Self> {
        let ids = value.get("save_data").unwrap_or(value);
        Some(Self {
            channel_id: ids.get("channel_id").and_then(id_value)?,
            message_id: ids.get("message_id").and_then(id_value)?,
        })
    }
}
