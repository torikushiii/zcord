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
    #[serde(default, deserialize_with = "nullable")]
    pub profile: Option<Option<GuildProfile>>,
    #[serde(default, deserialize_with = "default_value")]
    pub premium_tier: Option<u8>,
    #[serde(default, deserialize_with = "default_value")]
    pub premium_progress_bar_enabled: Option<bool>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub vanity_url_code: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub hub_type: Option<Option<i64>>,
}

#[derive(Clone, Deserialize)]
pub struct ScheduledEvent {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "optional_id")]
    pub channel_id: Option<u64>,
    #[serde(default)]
    pub status: u8,
}

impl ScheduledEvent {
    pub fn upcoming(&self) -> bool {
        !matches!(self.status, 3 | 4)
    }
}

#[derive(Clone, Default, Deserialize)]
pub struct GuildProfile {
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub badge: Option<String>,
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
    #[serde(default, deserialize_with = "default_value")]
    pub premium_subscription_count: Option<u32>,
    #[serde(default)]
    pub member_count: Option<u32>,
    #[serde(default)]
    pub joined_at: Option<String>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub guild_scheduled_events: Option<Vec<ScheduledEvent>>,
    #[serde(default)]
    pub experiments: Option<ApexAssignments>,
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
    pub fn has_feature(&self, name: &str) -> bool {
        self.features()
            .is_some_and(|features| features.iter().any(|feature| feature == name))
    }
    pub fn premium_tier(&self) -> u8 {
        self.properties
            .premium_tier
            .or(self.legacy.premium_tier)
            .unwrap_or(0)
    }
    pub fn premium_progress_bar_enabled(&self) -> bool {
        self.properties
            .premium_progress_bar_enabled
            .or(self.legacy.premium_progress_bar_enabled)
            .unwrap_or(false)
    }
    pub fn vanity_url_code(&self) -> Option<&str> {
        self.properties
            .vanity_url_code
            .as_ref()
            .or(self.legacy.vanity_url_code.as_ref())?
            .as_deref()
    }
    pub fn hub_type(&self) -> Option<i64> {
        self.properties.hub_type.or(self.legacy.hub_type).flatten()
    }
    pub fn tag(&self) -> Option<(&str, Option<&str>)> {
        let profile = self
            .properties
            .profile
            .as_ref()
            .or(self.legacy.profile.as_ref())?
            .as_ref()?;
        let tag = profile.tag.as_deref().filter(|tag| !tag.is_empty())?;
        Some((tag, profile.badge.as_deref()))
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
        if let Some(profile) = update
            .properties
            .profile
            .as_ref()
            .or(update.legacy.profile.as_ref())
        {
            self.properties.profile = Some(profile.clone());
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
        if let Some(tier) = update
            .properties
            .premium_tier
            .or(update.legacy.premium_tier)
        {
            self.properties.premium_tier = Some(tier);
        }
        if let Some(enabled) = update
            .properties
            .premium_progress_bar_enabled
            .or(update.legacy.premium_progress_bar_enabled)
        {
            self.properties.premium_progress_bar_enabled = Some(enabled);
        }
        if update.premium_subscription_count.is_some() {
            self.premium_subscription_count = update.premium_subscription_count;
        }
        if let Some(code) = update
            .properties
            .vanity_url_code
            .as_ref()
            .or(update.legacy.vanity_url_code.as_ref())
        {
            self.properties.vanity_url_code = Some(code.clone());
        }
        if let Some(kind) = update.properties.hub_type.or(update.legacy.hub_type) {
            self.properties.hub_type = Some(kind);
        }
        if update.guild_scheduled_events.is_some() {
            self.guild_scheduled_events
                .clone_from(&update.guild_scheduled_events);
        }
        if update.experiments.is_some() {
            self.experiments.clone_from(&update.experiments);
        }
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
