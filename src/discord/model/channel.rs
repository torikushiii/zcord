use super::serde_helpers::*;
use super::*;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct Overwrite {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(deserialize_with = "snowflake")]
    pub allow: u64,
    #[serde(deserialize_with = "snowflake")]
    pub deny: u64,
}

#[derive(Clone, Default, Deserialize)]
pub struct ThreadMetadata {
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub create_timestamp: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct ForumTag {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub moderated: bool,
    #[serde(default)]
    pub emoji: Option<Emoji>,
}

#[derive(Clone, Deserialize)]
pub struct Channel {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, rename = "type")]
    pub kind: Option<u8>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default, deserialize_with = "nullable_id")]
    pub parent_id: Option<Option<u64>>,
    #[serde(default, deserialize_with = "nullable_id")]
    pub last_message_id: Option<Option<u64>>,
    #[serde(default)]
    pub permission_overwrites: Option<Vec<Overwrite>>,
    #[serde(default, deserialize_with = "optional_id")]
    pub owner_id: Option<u64>,
    #[serde(default)]
    pub thread_metadata: Option<ThreadMetadata>,
    #[serde(default)]
    pub message_count: Option<u32>,
    #[serde(default, deserialize_with = "optional_ids")]
    pub applied_tags: Option<Vec<u64>>,
    #[serde(default)]
    pub flags: Option<u64>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub available_tags: Option<Vec<ForumTag>>,
    #[serde(default)]
    pub default_sort_order: Option<u8>,
    #[serde(default, deserialize_with = "ids")]
    pub recipient_ids: Vec<u64>,
    #[serde(default, deserialize_with = "entries")]
    pub recipients: Vec<User>,
    #[serde(default)]
    pub is_message_request: Option<bool>,
    #[serde(default)]
    pub is_spam: Option<bool>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub icon: Option<Option<String>>,
}

impl Channel {
    pub fn merge(&mut self, update: &Self) {
        macro_rules! merge { ($($field:ident),*) => { $(if update.$field.is_some() { self.$field = update.$field.clone(); })* }; }
        merge!(
            guild_id,
            kind,
            name,
            position,
            parent_id,
            last_message_id,
            permission_overwrites,
            owner_id,
            thread_metadata,
            message_count,
            applied_tags,
            flags,
            available_tags,
            default_sort_order,
            is_message_request,
            is_spam,
            icon
        );
        if !update.recipient_ids.is_empty() {
            self.recipient_ids.clone_from(&update.recipient_ids);
        }
        if !update.recipients.is_empty() {
            self.recipients.clone_from(&update.recipients);
        }
    }
}

#[derive(Clone, Default, Deserialize)]
pub struct ForumSearch {
    #[serde(default, deserialize_with = "entries")]
    pub threads: Vec<Channel>,
    #[serde(default, deserialize_with = "entries")]
    pub first_messages: Vec<Message>,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Clone, Deserialize)]
pub struct ThreadListSync {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "entries")]
    pub threads: Vec<Channel>,
}
