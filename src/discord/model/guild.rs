use super::serde_helpers::*;
use super::*;
use serde::Deserialize;

#[derive(Clone, Default, Deserialize)]
pub struct GuildProperties {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub owner_id: Option<u64>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub icon: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub banner: Option<Option<String>>,
    #[serde(default)]
    pub features: Option<Vec<String>>,
    #[serde(default, deserialize_with = "optional_level")]
    pub default_message_notifications: Option<NotificationLevel>,
}

#[derive(Clone, Deserialize)]
pub struct Guild {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub properties: GuildProperties,
    #[serde(flatten)]
    pub legacy: GuildProperties,
    #[serde(default, deserialize_with = "optional_entries")]
    pub channels: Option<Vec<Channel>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub threads: Option<Vec<Channel>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub roles: Option<Vec<Role>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub members: Option<Vec<Member>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub emojis: Option<Vec<GuildEmoji>>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub stickers: Option<Vec<StickerDetails>>,
    #[serde(default, deserialize_with = "entries")]
    pub voice_states: Vec<VoiceState>,
    #[serde(default)]
    pub unavailable: Option<bool>,
    #[serde(default)]
    pub premium_tier: u8,
}

impl Guild {
    pub fn name(&self) -> &str {
        self.properties
            .name
            .as_deref()
            .or(self.legacy.name.as_deref())
            .unwrap_or("")
    }
    pub fn owner_id(&self) -> Option<u64> {
        self.properties.owner_id.or(self.legacy.owner_id)
    }
    pub fn icon(&self) -> Option<String> {
        self.properties
            .icon
            .as_ref()
            .or(self.legacy.icon.as_ref())
            .cloned()
            .flatten()
    }
    pub fn banner(&self) -> Option<String> {
        self.properties
            .banner
            .as_ref()
            .or(self.legacy.banner.as_ref())
            .cloned()
            .flatten()
    }
    pub fn features(&self) -> Option<&[String]> {
        self.properties
            .features
            .as_deref()
            .or(self.legacy.features.as_deref())
    }
    pub fn notification_default(&self) -> Option<NotificationLevel> {
        self.properties
            .default_message_notifications
            .or(self.legacy.default_message_notifications)
    }
    pub fn fill_voice_state_guilds(&mut self) {
        for state in &mut self.voice_states {
            state.guild_id = Some(self.id);
        }
    }

    pub fn merge(&mut self, update: &Self) {
        if let Some(name) = update
            .properties
            .name
            .as_ref()
            .or(update.legacy.name.as_ref())
        {
            self.properties.name = Some(name.clone());
        }
        if let Some(id) = update.owner_id() {
            self.properties.owner_id = Some(id);
        }
        if let Some(icon) = update
            .properties
            .icon
            .as_ref()
            .or(update.legacy.icon.as_ref())
        {
            self.properties.icon = Some(icon.clone());
        }
        if let Some(banner) = update
            .properties
            .banner
            .as_ref()
            .or(update.legacy.banner.as_ref())
        {
            self.properties.banner = Some(banner.clone());
        }
        if let Some(features) = update.features() {
            self.properties.features = Some(features.to_vec());
        }
        if update.channels.is_some() {
            self.channels.clone_from(&update.channels);
        }
        if update.threads.is_some() {
            self.threads.clone_from(&update.threads);
        }
        if update.roles.is_some() {
            self.roles.clone_from(&update.roles);
        }
        if update.members.is_some() {
            self.members.clone_from(&update.members);
        }
        if update.emojis.is_some() {
            self.emojis.clone_from(&update.emojis);
        }
        if update.stickers.is_some() {
            self.stickers.clone_from(&update.stickers);
        }
        if !update.voice_states.is_empty() {
            self.voice_states.clone_from(&update.voice_states);
        }
        if update.unavailable.is_some() {
            self.unavailable = update.unavailable;
        }
        self.premium_tier = update.premium_tier;
    }
}

#[derive(Clone, Deserialize)]
pub struct GuildEmoji {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub animated: bool,
    #[serde(default = "default_true")]
    pub available: bool,
    #[serde(default, deserialize_with = "ids")]
    pub roles: Vec<u64>,
    #[serde(default = "default_true")]
    pub require_colons: bool,
}

#[derive(Clone, Deserialize)]
pub struct GuildEmojisUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "entries")]
    pub emojis: Vec<GuildEmoji>,
}

#[derive(Clone, Deserialize)]
pub struct GuildStickersUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "optional_entries")]
    pub stickers: Option<Vec<StickerDetails>>,
}
