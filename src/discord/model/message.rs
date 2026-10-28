use super::serde_helpers::*;
use super::*;
use serde::Deserialize;

#[derive(Clone, Default, Deserialize)]
pub struct Emoji {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub animated: bool,
}

#[derive(Clone, Deserialize)]
pub struct Reaction {
    #[serde(default)]
    pub emoji: Emoji,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub me: bool,
    #[serde(default)]
    pub me_burst: bool,
    #[serde(default)]
    pub burst_me: bool,
}

#[derive(Deserialize)]
pub struct ReactionEvent {
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub message_id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub emoji: Emoji,
}

#[derive(Deserialize)]
pub struct ReactionManyEvent {
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub message_id: u64,
    #[serde(default, deserialize_with = "entries")]
    pub reactions: Vec<DebouncedReaction>,
}

#[derive(Deserialize)]
pub struct DebouncedReaction {
    #[serde(default, deserialize_with = "ids")]
    pub users: Vec<u64>,
    #[serde(default)]
    pub emoji: Emoji,
}

#[derive(Clone, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub flags: u64,
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(default, deserialize_with = "entries")]
    pub reactions: Vec<Reaction>,
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(default, deserialize_with = "optional_nonce")]
    pub nonce: Option<u64>,
    pub author: User,
    #[serde(default, deserialize_with = "optional_id")]
    pub webhook_id: Option<u64>,
    #[serde(deserialize_with = "timestamp")]
    pub timestamp: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, deserialize_with = "entries")]
    pub embeds: Vec<Embed>,
    #[serde(default, deserialize_with = "entries")]
    pub attachments: Vec<Attachment>,
    #[serde(default, deserialize_with = "entries")]
    pub sticker_items: Vec<Sticker>,
    #[serde(default, rename = "type")]
    pub kind: u8,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub mentions: Vec<User>,
    #[serde(default, deserialize_with = "ids")]
    pub mention_roles: Vec<u64>,
    #[serde(default)]
    pub mention_everyone: bool,
    #[serde(default)]
    pub member: Option<PartialMember>,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default)]
    pub message_reference: Option<MessageReference>,
    #[serde(default, deserialize_with = "entries")]
    pub message_snapshots: Vec<MessageSnapshot>,
    #[serde(default, deserialize_with = "nullable")]
    pub referenced_message: Option<Option<Box<ReferencedMessage>>>,
}

#[derive(Clone, Deserialize)]
pub struct MessageRequestPreview {
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(default, deserialize_with = "lenient_message")]
    pub message_preview: Option<Message>,
}

#[derive(Clone, Deserialize)]
pub struct MessageUpdate {
    #[serde(default)]
    pub flags: Option<u64>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub components: Option<Vec<Component>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub reactions: Option<Vec<Reaction>>,
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(default)]
    pub author: Option<User>,
    #[serde(default, deserialize_with = "optional_id")]
    pub webhook_id: Option<u64>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub embeds: Option<Vec<Embed>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub attachments: Option<Vec<Attachment>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub sticker_items: Option<Vec<Sticker>>,
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
    #[serde(default, deserialize_with = "nullable")]
    pub edited_timestamp: Option<Option<String>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub mentions: Option<Vec<User>>,
    #[serde(default, deserialize_with = "optional_ids")]
    pub mention_roles: Option<Vec<u64>>,
    #[serde(default)]
    pub mention_everyone: Option<bool>,
    #[serde(default)]
    pub member: Option<PartialMember>,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "nullable")]
    pub message_reference: Option<Option<MessageReference>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub message_snapshots: Option<Vec<MessageSnapshot>>,
    #[serde(default, deserialize_with = "nullable")]
    pub referenced_message: Option<Option<Box<ReferencedMessage>>>,
}

#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
pub struct MessageReference {
    #[serde(default, deserialize_with = "optional_id")]
    pub message_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub channel_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, rename = "type")]
    pub kind: u8,
}

#[derive(Clone, Deserialize)]
pub struct MessageSnapshot {
    pub message: SnapshotMessage,
}

#[derive(Clone, Default, Deserialize)]
pub struct SnapshotMessage {
    #[serde(default)]
    pub content: String,
    #[serde(default, deserialize_with = "entries")]
    pub embeds: Vec<Embed>,
    #[serde(default, deserialize_with = "entries")]
    pub attachments: Vec<Attachment>,
    #[serde(default, deserialize_with = "entries")]
    pub sticker_items: Vec<Sticker>,
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(default)]
    pub flags: u64,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub timestamp: Option<String>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub edited_timestamp: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct TypingStart {
    #[serde(deserialize_with = "snowflake")]
    pub channel_id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(deserialize_with = "snowflake")]
    pub user_id: u64,
    #[serde(default)]
    pub member: Option<PartialMember>,
}

#[derive(Clone, Deserialize)]
pub struct ReferencedMessage {
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    pub author: User,
    #[serde(default, deserialize_with = "optional_id")]
    pub webhook_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub flags: u64,
    #[serde(default)]
    pub content: String,
    #[serde(default, deserialize_with = "entries")]
    pub attachments: Vec<Attachment>,
    #[serde(default, deserialize_with = "entries")]
    pub sticker_items: Vec<Sticker>,
    #[serde(default, deserialize_with = "entries")]
    pub embeds: Vec<Embed>,
    #[serde(default, deserialize_with = "entries")]
    pub message_snapshots: Vec<MessageSnapshot>,
}

#[derive(Clone, Default, Deserialize)]
pub struct Sticker {
    #[serde(default, deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub format_type: u8,
}

#[derive(Clone, Deserialize)]
pub struct Attachment {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub flags: u64,
}
