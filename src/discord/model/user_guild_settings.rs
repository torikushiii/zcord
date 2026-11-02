use super::serde_helpers::*;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotificationLevel {
    All,
    Mentions,
    Nothing,
    #[default]
    Inherit,
}

impl NotificationLevel {
    pub(crate) fn from_value(value: &Value) -> Self {
        match value.as_u64() {
            Some(0) => Self::All,
            Some(1) => Self::Mentions,
            Some(2) => Self::Nothing,
            _ => Self::Inherit,
        }
    }
}

#[derive(Clone, Default, Deserialize)]
pub struct MuteConfig {
    #[serde(default, deserialize_with = "end_time")]
    pub end_time: Option<u64>,
}

#[derive(Clone, Default, Deserialize)]
pub struct ChannelOverride {
    #[serde(default, deserialize_with = "optional_id")]
    pub channel_id: Option<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub muted: bool,
    #[serde(default, deserialize_with = "default_value")]
    pub mute_config: Option<MuteConfig>,
    #[serde(default, deserialize_with = "level")]
    pub message_notifications: NotificationLevel,
}

#[derive(Clone, Default, Deserialize)]
pub struct UserGuildSettings {
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub muted: bool,
    #[serde(default, deserialize_with = "default_value")]
    pub mute_config: Option<MuteConfig>,
    #[serde(default, deserialize_with = "level")]
    pub message_notifications: NotificationLevel,
    #[serde(default, deserialize_with = "default_value")]
    pub suppress_everyone: bool,
    #[serde(default, deserialize_with = "default_value")]
    pub suppress_roles: bool,
    #[serde(default, deserialize_with = "channel_overrides")]
    pub channel_overrides: Vec<ChannelOverride>,
}

pub fn user_guild_settings<'de, D>(d: D) -> Result<Vec<UserGuildSettings>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(d)?;
    let entries = match value {
        Value::Array(entries) => Value::Array(entries),
        Value::Object(mut object) => object.remove("entries").unwrap_or(Value::Null),
        _ => Value::Null,
    };
    Ok(parse_entries(entries))
}

/// The overrides arrive as a list, or as an object keyed by channel id.
fn channel_overrides<'de, D>(d: D) -> Result<Vec<ChannelOverride>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(parse_entries(match Value::deserialize(d)? {
        Value::Object(overrides) => overrides.into_values().collect(),
        value => value,
    }))
}

pub fn optional_level<'de, D>(d: D) -> Result<Option<NotificationLevel>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Some(NotificationLevel::from_value(&Value::deserialize(d)?)))
}

fn level<'de, D>(d: D) -> Result<NotificationLevel, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(NotificationLevel::from_value(&Value::deserialize(d)?))
}

fn end_time<'de, D>(d: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Value::deserialize(d)? {
        Value::String(text) => chrono::DateTime::parse_from_rfc3339(&text)
            .ok()
            .and_then(|time| u64::try_from(time.timestamp_millis()).ok()),
        Value::Number(number) => number.as_u64(),
        _ => None,
    })
}
