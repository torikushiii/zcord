use super::serde_helpers::*;
use super::*;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct ScheduledMessage {
    #[serde(deserialize_with = "snowflake")]
    pub scheduled_message_id: u64,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub send_at_timestamp: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub state: u8,
    pub create_args: ScheduledMessageArgs,
    #[serde(default, deserialize_with = "lenient_message")]
    pub message_preview: Option<Message>,
}

#[derive(Clone, Deserialize)]
pub struct ScheduledMessageArgs {
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub content: String,
    #[serde(default, deserialize_with = "default_value")]
    pub flags: u64,
}

impl ScheduledMessage {
    pub const SCHEDULED: u8 = 0;
}
