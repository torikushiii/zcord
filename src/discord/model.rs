use serde::{Deserialize, Deserializer, de::DeserializeOwned};
use serde_json::Value;
use std::collections::HashMap;

pub fn snowflake<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let value = Value::deserialize(d)?;
    id_value(&value).ok_or_else(|| serde::de::Error::custom("invalid snowflake"))
}

pub fn id_value(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| value.as_u64())
}

fn optional_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(id_value(&Value::deserialize(d)?))
}

fn optional_nonce<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.as_u64(),
        _ => None,
    })
}

fn nullable_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<u64>>, D::Error> {
    optional_id(d).map(Some)
}

fn nullable_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

pub fn parse_entries<T: DeserializeOwned>(value: Value) -> Vec<T> {
    let Value::Array(entries) = value else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| match serde_json::from_value(entry) {
            Ok(entry) => Some(entry),
            Err(_) => {
                log::warn!("Skipping malformed {} entry", std::any::type_name::<T>());
                None
            }
        })
        .collect()
}

fn entries<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    Value::deserialize(d).map(parse_entries)
}

fn optional_entries<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Option<Vec<T>>, D::Error> {
    Option::<Value>::deserialize(d).map(|v| v.map(parse_entries))
}

fn ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(id_value)
        .collect())
}

fn optional_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u64>>, D::Error> {
    ids(d).map(Some)
}

// `wishlist_settings` is keyed by wishlist id ("1191169174274048062"), not an array.
fn wishlist_ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_object()
        .into_iter()
        .flat_map(|settings| settings.keys())
        .filter_map(|id| id.parse().ok())
        .collect())
}

fn default_value<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    d: D,
) -> Result<T, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(d)?).unwrap_or_default())
}

fn nullable<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(if value.is_null() {
        Some(None)
    } else {
        serde_json::from_value(value).ok().map(Some)
    })
}

fn timestamp<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = String::deserialize(d)?;
    chrono::DateTime::parse_from_rfc3339(&value)
        .map_err(|_| serde::de::Error::custom("invalid timestamp"))?;
    Ok(value)
}

fn optional_timestamp<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_str()
        .filter(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok())
        .map(str::to_owned))
}

#[derive(Clone, Default, Deserialize)]
pub struct User {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub discriminator: Option<String>,
    #[serde(default)]
    pub bot: bool,
    #[serde(default)]
    pub premium_type: Option<u8>,
    #[serde(default, deserialize_with = "nullable")]
    pub primary_guild: Option<Option<PrimaryGuild>>,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar_decoration_data: Option<Option<AvatarDecoration>>,
    #[serde(default, deserialize_with = "nullable")]
    pub collectibles: Option<Option<Collectibles>>,
    #[serde(default, deserialize_with = "nullable")]
    pub display_name_styles: Option<Option<DisplayNameStyles>>,
}

#[derive(Clone, Deserialize)]
pub struct UserUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default)]
    pub username: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub global_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub avatar: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub discriminator: Option<Option<String>>,
    #[serde(default)]
    pub bot: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    pub primary_guild: Option<Option<PrimaryGuild>>,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar_decoration_data: Option<Option<AvatarDecoration>>,
    #[serde(default, deserialize_with = "nullable")]
    pub collectibles: Option<Option<Collectibles>>,
    #[serde(default, deserialize_with = "nullable")]
    pub display_name_styles: Option<Option<DisplayNameStyles>>,
}

#[derive(Clone, Deserialize)]
pub struct AvatarDecoration {
    pub asset: String,
    #[serde(default, deserialize_with = "optional_id")]
    pub sku_id: Option<u64>,
}

#[derive(Clone, Deserialize)]
pub struct Collectibles {
    #[serde(default, deserialize_with = "default_value")]
    pub nameplate: Option<Nameplate>,
}

#[derive(Clone, Deserialize)]
pub struct Nameplate {
    pub asset: String,
    #[serde(default, deserialize_with = "default_value")]
    pub palette: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub sku_id: Option<u64>,
}

#[derive(Clone, Deserialize)]
pub struct DisplayNameStyles {
    #[serde(default, deserialize_with = "default_value")]
    pub font_id: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub effect_id: Option<u32>,
    #[serde(default, deserialize_with = "entries")]
    pub colors: Vec<u32>,
}

#[derive(Clone, Deserialize)]
pub struct Profile {
    pub user: ProfileUser,
    #[serde(default, deserialize_with = "default_value")]
    pub user_profile: Option<UserProfile>,
    #[serde(default, deserialize_with = "entries")]
    pub badges: Vec<Badge>,
    #[serde(default, deserialize_with = "entries")]
    pub connected_accounts: Vec<ConnectedAccount>,
    #[serde(default, deserialize_with = "default_value")]
    pub premium_type: Option<u8>,
    #[serde(default, deserialize_with = "default_value")]
    pub premium_since: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub premium_guild_since: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub legacy_username: Option<String>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub mutual_guilds: Option<Vec<MutualGuild>>,
    #[serde(default, deserialize_with = "default_value")]
    pub mutual_friends_count: Option<u32>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub mutual_friends: Option<Vec<User>>,
    #[serde(default, deserialize_with = "default_value")]
    pub guild_member: Option<ProfileMember>,
    #[serde(default, deserialize_with = "default_value")]
    pub guild_member_profile: Option<UserProfile>,
    #[serde(default, deserialize_with = "entries")]
    pub widgets: Vec<ProfileWidget>,
    #[serde(default, rename = "wishlist_settings", deserialize_with = "wishlist_ids")]
    pub wishlist_ids: Vec<u64>,
}

#[derive(Clone, Deserialize)]
pub struct ProfileUser {
    #[serde(flatten)]
    pub user: User,
    #[serde(default, deserialize_with = "default_value")]
    pub banner: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub banner_color: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub accent_color: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub bio: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct UserProfile {
    #[serde(default, deserialize_with = "default_value")]
    pub bio: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub pronouns: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub accent_color: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub banner: Option<String>,
    #[serde(default, deserialize_with = "optional_entries")]
    pub theme_colors: Option<Vec<u32>>,
    #[serde(default, deserialize_with = "default_value")]
    pub profile_effect: Option<ProfileEffectRef>,
    #[serde(default, deserialize_with = "entries")]
    pub collectibles: Vec<ProfileCollectible>,
}

#[derive(Clone, Deserialize)]
pub struct ProfileCollectible {
    #[serde(deserialize_with = "snowflake")]
    pub sku_id: u64,
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    pub kind: u8,
}

#[derive(Clone, Deserialize)]
pub struct ProfileEffectRef {
    #[serde(deserialize_with = "snowflake")]
    pub sku_id: u64,
}

#[derive(Clone, Deserialize)]
pub struct Badge {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default, deserialize_with = "default_value")]
    pub link: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct UserBadges {
    #[serde(default, deserialize_with = "entries")]
    pub badges: Vec<BadgeDetail>,
}

#[derive(Clone, Deserialize)]
pub struct BadgeDetail {
    pub badge_id: u32,
    #[serde(default, deserialize_with = "default_value")]
    pub name: String,
    #[serde(default, deserialize_with = "default_value")]
    pub info_label: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub simple_icon_raster_url: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub complex_icon_animated_url: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub current_tier: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub tiers: Vec<BadgeTier>,
}

#[derive(Clone, Deserialize)]
pub struct BadgeTier {
    #[serde(default, deserialize_with = "default_value")]
    pub key: String,
    #[serde(default, deserialize_with = "default_value")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub simple_icon_raster_url: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub complex_icon_animated_url: Option<String>,
}

impl BadgeDetail {
    pub fn tier(&self) -> Option<&BadgeTier> {
        let current = self.current_tier.as_deref()?;
        self.tiers.iter().find(|tier| tier.key == current)
    }
}

impl UserBadges {
    pub fn detail_for(&self, badge: &Badge) -> Option<&BadgeDetail> {
        if !badge.icon.is_empty() {
            let suffix = format!("/{}.png", badge.icon);
            let icon_matches = |detail: &BadgeDetail| {
                detail
                    .simple_icon_raster_url
                    .as_deref()
                    .is_some_and(|url| url.ends_with(&suffix))
                    || detail.tiers.iter().any(|tier| {
                        tier.simple_icon_raster_url
                            .as_deref()
                            .is_some_and(|url| url.ends_with(&suffix))
                    })
            };
            if let Some(detail) = self.badges.iter().find(|detail| icon_matches(detail)) {
                return Some(detail);
            }
        }
        if let Some(badge_id) = badge_id_for(&badge.id)
            && let Some(detail) = self
                .badges
                .iter()
                .find(|detail| detail.badge_id == badge_id)
        {
            return Some(detail);
        }
        self.badges
            .iter()
            .find(|detail| !detail.name.is_empty() && detail.name.eq_ignore_ascii_case(&badge.description))
    }
}

fn badge_id_for(id: &str) -> Option<u32> {
    match id {
        "premium" => Some(1),
        _ if id.starts_with("premium_tenure_") => Some(1),
        "partner" => Some(2),
        "certified_moderator" => Some(3),
        "hypesquad" => Some(4),
        "hypesquad_house_1" => Some(5),
        "hypesquad_house_2" => Some(6),
        "hypesquad_house_3" => Some(7),
        "bug_hunter_level_1" => Some(8),
        "bug_hunter_level_2" => Some(9),
        "verified_developer" => Some(10),
        "early_supporter" => Some(11),
        _ if id.starts_with("guild_booster_lvl") => Some(12),
        "legacy_username" => Some(13),
        "quest_completed" => Some(14),
        "orb_profile_badge" => Some(16),
        "account_age" => Some(18),
        "game_time" => Some(20),
        "game_variety" => Some(21),
        "staff" => Some(22),
        _ => None,
    }
}

#[derive(Clone, Deserialize)]
pub struct ConnectedAccount {
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    pub kind: String,
    #[serde(default, deserialize_with = "default_value")]
    pub id: String,
    #[serde(default, deserialize_with = "default_value")]
    pub name: String,
    #[serde(default, deserialize_with = "default_value")]
    pub verified: bool,
}

#[derive(Clone, Deserialize)]
pub struct MutualGuild {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub nick: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct ProfileMember {
    #[serde(flatten)]
    pub member: PartialMember,
    #[serde(default, deserialize_with = "default_value")]
    pub banner: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub joined_at: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct CollectibleProduct {
    #[serde(deserialize_with = "snowflake")]
    pub sku_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub name: String,
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    pub kind: u8,
    #[serde(default, deserialize_with = "entries")]
    pub items: Vec<CollectibleItem>,
}

#[derive(Clone, Deserialize)]
pub struct CollectibleItem {
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    pub kind: u8,
    #[serde(deserialize_with = "snowflake")]
    pub sku_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub asset: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub palette: Option<String>,
    #[serde(
        default,
        rename = "accessibilityLabel",
        deserialize_with = "default_value"
    )]
    pub accessibility_label: Option<String>,
    #[serde(
        default,
        rename = "reducedMotionSrc",
        deserialize_with = "default_value"
    )]
    pub reduced_motion_src: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub effects: Vec<EffectLayer>,
    #[serde(default, deserialize_with = "entries")]
    pub layers: Vec<FrameLayer>,
    #[serde(default, deserialize_with = "default_value")]
    pub inner_width: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub overflow_top: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub overflow_bottom: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub overflow_horizontal: Option<u32>,
}

#[derive(Clone, Deserialize)]
pub struct EffectLayer {
    #[serde(default, deserialize_with = "default_value")]
    pub src: String,
    #[serde(default, rename = "loop", deserialize_with = "default_value")]
    pub looped: bool,
    #[serde(default, deserialize_with = "default_value")]
    pub duration: u32,
    #[serde(default, deserialize_with = "default_value")]
    pub start: u32,
    #[serde(default, rename = "loopDelay", deserialize_with = "default_value")]
    pub loop_delay: u32,
    #[serde(default, rename = "zIndex", deserialize_with = "default_value")]
    pub z_index: i32,
}

#[derive(Clone, Deserialize)]
pub struct FrameLayer {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, rename = "type", deserialize_with = "default_value")]
    pub kind: String,
    #[serde(default, deserialize_with = "default_value")]
    pub order: String,
    #[serde(default, deserialize_with = "default_value")]
    pub anchor: String,
    #[serde(default, deserialize_with = "default_value")]
    pub responsive: bool,
}

#[derive(Clone, Deserialize)]
pub struct PrimaryGuild {
    #[serde(default)]
    pub identity_enabled: Option<bool>,
    #[serde(default, deserialize_with = "optional_id")]
    pub identity_guild_id: Option<u64>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub badge: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct Member {
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default, deserialize_with = "ids")]
    pub roles: Vec<u64>,
    #[serde(default)]
    pub nick: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar_decoration_data: Option<Option<AvatarDecoration>>,
    #[serde(default, deserialize_with = "nullable")]
    pub display_name_styles: Option<Option<DisplayNameStyles>>,
    #[serde(default, deserialize_with = "nullable")]
    pub collectibles: Option<Option<Collectibles>>,
}

impl Member {
    pub fn id(&self) -> Option<u64> {
        self.user_id.or_else(|| self.user.as_ref().map(|u| u.id))
    }
}

#[derive(Clone, Deserialize)]
pub struct Role {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "snowflake")]
    pub permissions: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: u32,
    #[serde(default, deserialize_with = "default_value")]
    pub colors: RoleColors,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub hoist: bool,
}

#[derive(Clone, Default, Deserialize)]
pub struct RoleColors {
    #[serde(default)]
    pub primary_color: u32,
}

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
            default_sort_order
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
pub struct GuildProperties {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "optional_id")]
    pub owner_id: Option<u64>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub icon: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_string")]
    pub banner: Option<Option<String>>,
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
    #[serde(default)]
    pub unavailable: Option<bool>,
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
        if update.unavailable.is_some() {
            self.unavailable = update.unavailable;
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

#[derive(Clone, Deserialize)]
pub struct ReadState {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "optional_id")]
    pub last_message_id: Option<u64>,
    #[serde(default)]
    pub mention_count: u32,
    #[serde(default, rename = "read_state_type")]
    pub kind: u8,
}

#[derive(Clone, Default, Deserialize)]
pub struct ReadStates {
    #[serde(default, deserialize_with = "entries")]
    pub entries: Vec<ReadState>,
}

#[derive(Clone)]
pub struct Ready {
    pub user: User,
    pub users: Vec<User>,
    pub guilds: Vec<Guild>,
    pub guild_order: Vec<Option<u64>>,
    pub private_channels: Vec<Channel>,
    pub read_state: ReadStates,
    pub session_id: String,
    pub resume_gateway_url: Option<String>,
    pub user_settings_proto: Option<String>,
    pub sessions: Vec<Session>,
}

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

impl<'de> Deserialize<'de> for Ready {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            user: User,
            #[serde(default, deserialize_with = "entries")]
            users: Vec<User>,
            #[serde(default)]
            guilds: Vec<Value>,
            #[serde(default)]
            merged_members: Vec<Value>,
            #[serde(default, deserialize_with = "entries")]
            private_channels: Vec<Channel>,
            #[serde(default)]
            read_state: ReadStates,
            session_id: String,
            #[serde(default)]
            resume_gateway_url: Option<String>,
            #[serde(default)]
            user_settings_proto: Option<String>,
            #[serde(default, deserialize_with = "entries")]
            sessions: Vec<Session>,
        }
        let wire = Wire::deserialize(d)?;
        let guild_order = wire
            .guilds
            .iter()
            .map(|guild| guild.get("id").and_then(id_value))
            .collect();
        let guilds = wire
            .guilds
            .into_iter()
            .enumerate()
            .filter_map(
                |(index, value)| match serde_json::from_value::<Guild>(value) {
                    Ok(mut guild) => {
                        if let Some(members) = wire.merged_members.get(index) {
                            guild.members = Some(parse_entries(members.clone()));
                        }
                        Some(guild)
                    }
                    Err(_) => {
                        log::warn!("Skipping malformed guild entry");
                        None
                    }
                },
            )
            .collect();
        Ok(Self {
            user: wire.user,
            users: wire.users,
            guilds,
            guild_order,
            private_channels: wire.private_channels,
            read_state: wire.read_state,
            session_id: wire.session_id,
            resume_gateway_url: wire.resume_gateway_url,
            user_settings_proto: wire.user_settings_proto,
            sessions: wire.sessions,
        })
    }
}

#[derive(Clone, Default, Deserialize)]
pub struct Embed {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<EmbedMedia>,
    #[serde(default)]
    pub image: Option<EmbedMedia>,
    #[serde(default)]
    pub video: Option<EmbedMedia>,
    #[serde(default)]
    pub provider: Option<EmbedProvider>,
    #[serde(default)]
    pub color: Option<u32>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<EmbedAuthor>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub fields: Vec<EmbedField>,
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(default)]
    pub footer: Option<EmbedFooter>,
}

#[derive(Clone, Deserialize)]
pub struct EmbedAuthor {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub proxy_icon_url: Option<String>,
}
#[derive(Clone, Deserialize)]
pub struct EmbedField {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}
#[derive(Clone, Deserialize)]
pub struct EmbedFooter {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub proxy_icon_url: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
pub struct EmbedMedia {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub flags: u64,
}

#[derive(Clone, Deserialize)]
pub struct EmbedProvider {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

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

fn component_type<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let kind = u8::deserialize(d)?;
    if matches!(kind, 1..=3 | 5..=14 | 17) {
        Ok(kind)
    } else {
        Err(serde::de::Error::custom("unknown component type"))
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Deserialize)]
pub struct Component {
    #[serde(rename = "type", deserialize_with = "component_type")]
    pub kind: u8,
    #[serde(default, deserialize_with = "entries")]
    pub components: Vec<Component>,
    #[serde(default, deserialize_with = "default_value")]
    pub accessory: Option<Box<Component>>,
    #[serde(default)]
    pub style: u8,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub emoji: Option<Emoji>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub media: Option<EmbedMedia>,
    #[serde(default)]
    pub file: Option<EmbedMedia>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub spoiler: bool,
    #[serde(default, deserialize_with = "entries")]
    pub items: Vec<GalleryItem>,
    #[serde(default = "default_true")]
    pub divider: bool,
    #[serde(default)]
    pub spacing: u8,
    #[serde(default)]
    pub accent_color: Option<u32>,
}

#[derive(Clone, Deserialize)]
pub struct GalleryItem {
    #[serde(default)]
    pub media: EmbedMedia,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub spoiler: bool,
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

#[derive(Clone, Default, Deserialize)]
pub struct PartialMember {
    #[serde(default, deserialize_with = "optional_id")]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default, deserialize_with = "nullable")]
    pub nick: Option<Option<String>>,
    #[serde(default, deserialize_with = "optional_ids")]
    pub roles: Option<Vec<u64>>,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    pub avatar_decoration_data: Option<Option<AvatarDecoration>>,
    #[serde(default, deserialize_with = "nullable")]
    pub display_name_styles: Option<Option<DisplayNameStyles>>,
    #[serde(default, deserialize_with = "nullable")]
    pub collectibles: Option<Option<Collectibles>>,
    #[serde(default)]
    pub presence: Option<Presence>,
}

impl PartialMember {
    pub fn id(&self) -> Option<u64> {
        self.user_id.or_else(|| self.user.as_ref().map(|u| u.id))
    }
}

impl From<&Member> for PartialMember {
    fn from(member: &Member) -> Self {
        Self {
            user_id: member.id(),
            user: member.user.clone(),
            nick: Some(member.nick.clone()),
            roles: Some(member.roles.clone()),
            avatar: Some(member.avatar.clone()),
            avatar_decoration_data: member.avatar_decoration_data.clone(),
            display_name_styles: member.display_name_styles.clone(),
            collectibles: member.collectibles.clone(),
            presence: None,
        }
    }
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

fn entry_groups<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    d: D,
) -> Result<Vec<Vec<T>>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .cloned()
        .map(parse_entries)
        .collect())
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
    #[serde(default, deserialize_with = "entry_groups")]
    pub merged_members: Vec<Vec<PartialMember>>,
    #[serde(default, deserialize_with = "default_value")]
    pub merged_presences: MergedPresences,
}

#[derive(Deserialize)]
pub struct MembersChunk {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(default, deserialize_with = "entries")]
    pub members: Vec<PartialMember>,
    #[serde(default, deserialize_with = "ids")]
    pub not_found: Vec<u64>,
    #[serde(default, deserialize_with = "entries")]
    pub presences: Vec<Presence>,
}

#[derive(Deserialize)]
pub struct MemberUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(flatten)]
    pub member: PartialMember,
}

#[derive(Deserialize)]
pub struct RoleUpdate {
    #[serde(deserialize_with = "snowflake")]
    pub guild_id: u64,
    pub role: Role,
}

#[derive(Default, Deserialize)]
pub struct MemberListUpdate {
    #[serde(default, deserialize_with = "snowflake")]
    pub guild_id: u64,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub member_count: u32,
    #[serde(default)]
    pub online_count: u32,
    #[serde(default, deserialize_with = "entries")]
    pub groups: Vec<MemberGroup>,
    #[serde(default, deserialize_with = "entries")]
    pub ops: Vec<MemberListOp>,
}

#[derive(Default, Deserialize)]
pub struct MemberGroup {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub count: u32,
}

#[derive(Default, Deserialize)]
pub struct MemberListOp {
    #[serde(default)]
    pub op: String,
    #[serde(default)]
    pub range: Option<[u32; 2]>,
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default, deserialize_with = "entries")]
    pub items: Vec<MemberItem>,
    #[serde(default)]
    pub item: Option<MemberItem>,
}

#[derive(Default, Deserialize)]
pub struct MemberItem {
    #[serde(default)]
    pub group: Option<MemberGroup>,
    #[serde(default)]
    pub member: Option<PartialMember>,
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

#[derive(Clone, Debug, Deserialize)]
pub struct ProfileWidget {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    pub data: WidgetData,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetData {
    FavoriteGames {
        #[serde(default, deserialize_with = "default_value")]
        games: Vec<WidgetGame>,
    },
    CurrentGames {
        #[serde(default, deserialize_with = "default_value")]
        games: Vec<WidgetGame>,
    },
    /// The profile Board labels this list "Games I Like".
    PlayedGames {
        #[serde(default, deserialize_with = "default_value")]
        games: Vec<WidgetGame>,
    },
    WantToPlayGames {
        #[serde(default, deserialize_with = "default_value")]
        games: Vec<WidgetGame>,
    },
    Application {
        #[serde(deserialize_with = "snowflake")]
        application_id: u64,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WidgetGame {
    #[serde(deserialize_with = "snowflake")]
    pub game_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub comment: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub tags: Vec<String>,
}

fn steam_app_id<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .find(|sku| sku.get("distributor").and_then(Value::as_str) == Some("steam"))
        .and_then(|sku| sku.get("id"))
        .and_then(|id| match id {
            Value::String(id) => Some(id.clone()),
            Value::Number(id) => Some(id.to_string()),
            _ => None,
        }))
}

#[derive(Clone, Debug, Deserialize)]
pub struct Game {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    pub name: String,
    #[serde(default, deserialize_with = "default_value")]
    pub icon_hash: Option<String>,
    #[serde(
        rename = "third_party_skus",
        default,
        deserialize_with = "steam_app_id"
    )]
    pub steam_app_id: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub media: GameMedia,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct GameMedia {
    #[serde(default, deserialize_with = "default_value")]
    pub cover_url: Option<String>,
    /// Some games ship covers as an asset hash instead, served from `app-icons/{game_id}/`.
    #[serde(default, deserialize_with = "default_value")]
    pub cover_hash: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PublicApplication {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    pub name: String,
    #[serde(default, deserialize_with = "default_value")]
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub cover_image: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Wishlist {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, rename = "wishlist_items", deserialize_with = "entries")]
    pub items: Vec<WishlistItem>,
    #[serde(default, deserialize_with = "entries")]
    pub applications: Vec<PublicApplication>,
}

#[derive(Clone, Debug)]
pub struct WishlistItem {
    pub sku_id: u64,
    pub name: String,
    pub is_owned: bool,
    pub application_id: Option<u64>,
    pub thumbnail_asset_id: Option<u64>,
    pub card_image_asset_id: Option<u64>,
    pub card_background_asset_id: Option<u64>,
    pub price: Option<SkuPrice>,
}

impl<'de> Deserialize<'de> for WishlistItem {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct RawSku {
            #[serde(default, deserialize_with = "default_value")]
            name: String,
            #[serde(default, deserialize_with = "optional_id")]
            application_id: Option<u64>,
            #[serde(default, deserialize_with = "optional_id")]
            thumbnail_asset_id: Option<u64>,
            #[serde(default, deserialize_with = "default_value")]
            tenant_metadata: Option<RawTenantMetadata>,
            #[serde(default, deserialize_with = "default_value")]
            price: Option<SkuPrice>,
        }
        #[derive(Deserialize)]
        struct RawTenantMetadata {
            #[serde(default, deserialize_with = "default_value")]
            social_layer: Option<RawSocialLayer>,
        }
        #[derive(Deserialize)]
        struct RawSocialLayer {
            #[serde(default, deserialize_with = "optional_id")]
            card_image_asset_id: Option<u64>,
            #[serde(default, deserialize_with = "optional_id")]
            card_background_image_asset_id: Option<u64>,
        }
        #[derive(Deserialize)]
        struct Raw {
            #[serde(deserialize_with = "snowflake")]
            sku_id: u64,
            #[serde(default, deserialize_with = "default_value")]
            sku_name: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            is_owned: bool,
            #[serde(default, deserialize_with = "default_value")]
            sku: Option<RawSku>,
        }
        let raw = Raw::deserialize(d)?;
        let sku = raw.sku;
        Ok(Self {
            sku_id: raw.sku_id,
            name: raw
                .sku_name
                .or_else(|| sku.as_ref().map(|sku| sku.name.clone()))
                .unwrap_or_default(),
            is_owned: raw.is_owned,
            application_id: sku.as_ref().and_then(|sku| sku.application_id),
            thumbnail_asset_id: sku.as_ref().and_then(|sku| sku.thumbnail_asset_id),
            card_image_asset_id: sku.as_ref().and_then(|sku| {
                sku.tenant_metadata
                    .as_ref()?
                    .social_layer
                    .as_ref()?
                    .card_image_asset_id
            }),
            card_background_asset_id: sku.as_ref().and_then(|sku| {
                sku.tenant_metadata
                    .as_ref()?
                    .social_layer
                    .as_ref()?
                    .card_background_image_asset_id
            }),
            price: sku.and_then(|sku| sku.price),
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SkuPrice {
    #[serde(default, deserialize_with = "default_value")]
    pub amount: u64,
    #[serde(default)]
    pub currency: String,
    #[serde(
        default,
        rename = "currency_exponent",
        deserialize_with = "default_value"
    )]
    pub exponent: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ApplicationIdentities {
    #[serde(default, deserialize_with = "entries")]
    pub identities: Vec<ApplicationIdentity>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApplicationIdentity {
    #[serde(deserialize_with = "snowflake")]
    pub application_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub profile: Option<IdentityProfile>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct IdentityProfile {
    #[serde(default, deserialize_with = "default_value")]
    pub username: Option<String>,
    // Discord nests these under `data.dynamic`; flattening keeps the stat list at hand.
    #[serde(default, rename = "data", deserialize_with = "identity_stats")]
    pub stats: Vec<IdentityStat>,
}

impl IdentityProfile {
    /// The URL and natural size of an `Image` stat, e.g. `featured_played_character_image`.
    pub fn image(&self, key: &str) -> Option<(&str, Option<u32>, Option<u32>)> {
        self.stats.iter().find_map(|stat| {
            if stat.name != key {
                return None;
            }
            let StatValue::Image { url, width, height } = &stat.value else {
                return None;
            };
            Some((url.as_str(), *width, *height))
        })
    }
}

fn identity_stats<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<IdentityStat>, D::Error> {
    #[derive(Deserialize)]
    struct Data {
        #[serde(default, deserialize_with = "entries")]
        dynamic: Vec<IdentityStat>,
        // Some apps (Example Team Game) only ship `primary`; others ship both.
        #[serde(default)]
        primary: Value,
    }
    let Some(data) = Option::<Data>::deserialize(d)? else {
        return Ok(Vec::new());
    };
    let mut stats = data.dynamic;
    let Some(primary) = data.primary.as_object() else {
        return Ok(stats);
    };
    for (name, value) in primary {
        if stats.iter().any(|stat| &stat.name == name) {
            continue;
        }
        if let Some(value) = primary_stat_value(value) {
            stats.push(IdentityStat {
                name: name.clone(),
                value,
            });
        }
    }
    Ok(stats)
}

fn primary_stat_value(value: &Value) -> Option<StatValue> {
    Some(match value {
        Value::Number(number) => StatValue::Number(number.as_f64()?),
        Value::String(text) => StatValue::Text(text.clone()),
        Value::Bool(flag) => StatValue::Text(flag.to_string()),
        Value::Object(object) => {
            let url = object
                .get("proxy_url")
                .and_then(Value::as_str)
                .or_else(|| object.get("url").and_then(Value::as_str))?
                .to_owned();
            let dimension = |key: &str| {
                object
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
            };
            StatValue::Image {
                url,
                width: dimension("width"),
                height: dimension("height"),
            }
        }
        Value::Null | Value::Array(_) => return None,
    })
}

#[derive(Clone, Debug)]
pub struct IdentityStat {
    pub name: String,
    pub value: StatValue,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatValue {
    Text(String),
    Number(f64),
    /// `primary` stats can hold an image object; `url` is its `proxy_url` (or `url`).
    Image {
        url: String,
        width: Option<u32>,
        height: Option<u32>,
    },
}

#[derive(Deserialize)]
struct RawIdentityStat {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    kind: u8,
    #[serde(default)]
    value: Value,
}

impl From<RawIdentityStat> for IdentityStat {
    fn from(raw: RawIdentityStat) -> Self {
        let value = match raw.kind {
            2 => StatValue::Number(
                raw.value
                    .as_f64()
                    .or_else(|| raw.value.as_str().and_then(|value| value.parse().ok()))
                    .unwrap_or_default(),
            ),
            _ => StatValue::Text(match raw.value {
                Value::String(text) => text,
                other => other.to_string(),
            }),
        };
        Self {
            name: raw.name,
            value,
        }
    }
}

impl<'de> Deserialize<'de> for IdentityStat {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        RawIdentityStat::deserialize(d).map(Self::from)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Outbox {
    #[serde(default, deserialize_with = "entries")]
    pub entries: Vec<OutboxEntry>,
}

#[derive(Clone, Debug)]
pub struct OutboxEntry {
    pub id: u64,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    /// Live entries ("listening now") stay flagged live after they lapse; this says when.
    pub expires_at: Option<String>,
    pub kind: OutboxKind,
    pub traits: Vec<OutboxTrait>,
}

impl<'de> Deserialize<'de> for OutboxEntry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(deserialize_with = "snowflake")]
            id: u64,
            #[serde(default, deserialize_with = "default_value")]
            started_at: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            ended_at: Option<String>,
            #[serde(default, deserialize_with = "default_value")]
            expires_at: Option<String>,
            #[serde(default)]
            extra: Value,
            #[serde(default)]
            traits: Value,
        }
        let raw = Raw::deserialize(d)?;
        Ok(Self {
            id: raw.id,
            started_at: raw.started_at,
            ended_at: raw.ended_at,
            expires_at: raw.expires_at,
            kind: outbox_kind(&raw.extra),
            traits: outbox_traits(&raw.traits),
        })
    }
}

#[derive(Clone, Debug)]
pub enum OutboxKind {
    PlayedGame {
        name: String,
        application_id: Option<u64>,
    },
    Listened {
        tracks: Vec<ListenedTrack>,
        /// Listening sessions carry no start/end; this is when the session last changed.
        last_update: Option<String>,
    },
    LaunchedActivity {
        name: String,
        application_id: Option<u64>,
        last_update: Option<String>,
    },
    Unknown,
}

#[derive(Clone, Debug)]
pub struct ListenedTrack {
    pub title: String,
    pub artists: Vec<String>,
    pub image_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OutboxTrait {
    Live,
    Duration { seconds: u64 },
    FirstTime,
    Resurrected { last_played: String },
    Streak { days: u32 },
    Trending,
    Unknown,
}

fn outbox_kind(extra: &Value) -> OutboxKind {
    match extra.get("type").and_then(Value::as_str) {
        Some("played_game_extra") => OutboxKind::PlayedGame {
            name: extra
                .get("game_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            application_id: extra.get("application_id").and_then(id_value),
        },
        Some("launched_activity_extra") => OutboxKind::LaunchedActivity {
            name: extra
                .get("activity_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            application_id: extra.get("application_id").and_then(id_value),
            last_update: extra
                .get("last_update")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        Some("listened_session_extra") => OutboxKind::Listened {
            last_update: extra
                .get("last_update")
                .and_then(Value::as_str)
                .map(str::to_owned),
            tracks: extra
                .get("entries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(listened_track)
                .collect(),
        },
        _ => OutboxKind::Unknown,
    }
}

fn listened_track(entry: &Value) -> Option<ListenedTrack> {
    let media = entry.get("media")?;
    Some(ListenedTrack {
        title: media.get("title").and_then(Value::as_str)?.to_owned(),
        artists: media
            .get("artists")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|artist| {
                artist
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect(),
        image_url: media
            .get("image_url")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn outbox_traits(traits: &Value) -> Vec<OutboxTrait> {
    traits
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|trait_value| {
            let kind = trait_value.get("type")?.as_u64()?;
            Some(match kind {
                1 if trait_value.get("first_time").and_then(Value::as_bool) == Some(true) => {
                    OutboxTrait::FirstTime
                }
                2 => trait_value
                    .get("duration_seconds")
                    .and_then(Value::as_u64)
                    .map(|seconds| OutboxTrait::Duration { seconds })
                    .unwrap_or(OutboxTrait::Unknown),
                3 if trait_value.get("is_live").and_then(Value::as_bool) == Some(true) => {
                    OutboxTrait::Live
                }
                5 => trait_value
                    .get("resurrected_last_played")
                    .and_then(Value::as_str)
                    .map(|last_played| OutboxTrait::Resurrected {
                        last_played: last_played.to_owned(),
                    })
                    .unwrap_or(OutboxTrait::Unknown),
                8 => trait_value
                    .get("streak_count_days")
                    .and_then(Value::as_u64)
                    .and_then(|days| u32::try_from(days).ok())
                    .map(|days| OutboxTrait::Streak { days })
                    .unwrap_or(OutboxTrait::Unknown),
                9 => OutboxTrait::Trending,
                _ => OutboxTrait::Unknown,
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct WidgetConfig {
    pub application_id: u64,
    pub display_name: Option<String>,
    /// `widget_top.components.title.fields.text`
    pub title: Option<WidgetText>,
    /// `widget_top` subtitles in numeric order (subtitle_2 before subtitle_10).
    pub subtitles: Vec<WidgetSubtitle>,
    /// `widget_top.components.hero_image.fields.image`: an uploaded asset or an identity-data key.
    pub hero: Option<WidgetImage>,
    /// `widget_bottom` stats in numeric order (stat_2 before stat_10).
    pub stats: Vec<WidgetStat>,
    /// `widget_bottom` collection items in numeric order (item_2 before item_10).
    pub collection: Vec<WidgetItem>,
    /// `surfaces.mini_profile`, the surface Discord renders in the profile popout.
    pub mini: Option<MiniProfile>,
    /// `surfaces.activity_accessory`, the strip under the current-activity card.
    pub accessory: Option<WidgetAccessory>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetAccessory {
    pub layout: String,
    pub label: Option<WidgetText>,
    pub text: Option<WidgetText>,
    pub icon: Option<WidgetImage>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MiniProfile {
    pub layout: String,
    pub text: Option<WidgetText>,
    pub label: Option<WidgetText>,
    pub icon: Option<WidgetImage>,
    pub image: Option<WidgetImage>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WidgetImage {
    Asset(WidgetAsset),
    /// A key into identity stats; the UI resolves it to an image URL. `fallback` is the config's
    /// stand-in art for when that stat is missing.
    Data {
        key: String,
        fallback: Option<Box<WidgetImage>>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum WidgetText {
    /// `value_type: "data"`: a key into identity stats, or "username".
    Data(String),
    /// `value_type: "custom_string"`.
    Literal(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetSubtitle {
    pub text: WidgetText,
    pub label: Option<WidgetText>,
    pub icon: Option<WidgetAsset>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetAsset {
    pub asset_id: u64,
    pub width: u32,
    pub height: u32,
    pub animated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetStat {
    pub value: WidgetText,
    pub label: WidgetText,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WidgetItem {
    pub name: Option<WidgetText>,
    pub description: Option<WidgetText>,
    pub image: Option<WidgetAsset>,
}

pub fn parse_featured_widget_configs(value: Value) -> HashMap<u64, WidgetConfig> {
    let mut configs = HashMap::new();
    let Some(by_application) = value.get("configs").and_then(Value::as_object) else {
        return configs;
    };
    for (application_id, value) in by_application {
        let Ok(application_id) = application_id.parse::<u64>() else {
            continue;
        };
        let Some(value) = value.as_array() else {
            continue;
        };
        if let Some(config) = select_widget_config(application_id, value) {
            configs.insert(application_id, config);
        }
    }
    configs
}

pub fn parse_application_widget_configs(value: Value, application_id: u64) -> Option<WidgetConfig> {
    let configs = match &value {
        Value::Array(configs) => configs.as_slice(),
        Value::Object(object) => match object.get("configs") {
            Some(Value::Array(configs)) => configs.as_slice(),
            Some(Value::Object(by_application)) => {
                match by_application.get(&application_id.to_string()) {
                    Some(Value::Array(configs)) => configs.as_slice(),
                    _ => by_application.values().find_map(Value::as_array)?,
                }
            }
            // A few endpoints return the config object directly.
            _ if object.contains_key("surfaces") => {
                return select_widget_config(application_id, std::slice::from_ref(&value));
            }
            _ => return None,
        },
        _ => return None,
    };
    select_widget_config(application_id, configs)
}

fn select_widget_config(application_id: u64, configs: &[Value]) -> Option<WidgetConfig> {
    let configs: Vec<&Value> = configs.iter().filter(|config| config.is_object()).collect();
    let selected = configs
        .iter()
        .find(|config| config.get("status").and_then(Value::as_str) == Some("published"))
        .or_else(|| configs.first())?;
    Some(widget_config_from_value(application_id, selected))
}

fn widget_config_from_value(application_id: u64, config: &Value) -> WidgetConfig {
    let top = config
        .get("surfaces")
        .and_then(|surfaces| surfaces.get("widget_top"));
    let bottom = config
        .get("surfaces")
        .and_then(|surfaces| surfaces.get("widget_bottom"));
    let assets = resolved_assets(config);
    let mini = mini_profile_from_value(config, &assets);
    let accessory = widget_accessory_from_value(config, &assets);
    let title = top
        .and_then(|top| top.get("components"))
        .and_then(|components| components.get("title"))
        .and_then(|title| title.get("fields"))
        .and_then(|fields| fields.get("text"))
        .and_then(widget_text);
    let subtitles = top
        .and_then(|top| top.get("components"))
        .and_then(Value::as_object)
        .map(|components| {
            let mut subtitles: Vec<(u32, WidgetSubtitle)> = components
                .iter()
                .filter_map(|(key, component)| {
                    let index = key.strip_prefix("subtitle_")?.parse().ok()?;
                    let fields = component.get("fields")?;
                    let text = widget_text(fields.get("text")?)?;
                    Some((
                        index,
                        WidgetSubtitle {
                            text,
                            label: fields.get("label").and_then(widget_text),
                            icon: fields
                                .get("icon")
                                .and_then(|icon| widget_asset(icon, &assets)),
                        },
                    ))
                })
                .collect();
            subtitles.sort_by_key(|(index, _)| *index);
            subtitles
                .into_iter()
                .map(|(_, subtitle)| subtitle)
                .collect()
        })
        .unwrap_or_default();
    let hero = top
        .and_then(|top| top.get("components"))
        .and_then(|components| components.get("hero_image"))
        .and_then(|hero| hero.get("fields"))
        .and_then(|fields| fields.get("image"))
        .and_then(|image| widget_image(image, &assets));
    let stats = bottom
        .filter(|bottom| {
            bottom.get("layout").and_then(Value::as_str) == Some("widget_bottom_stats")
        })
        .and_then(|bottom| bottom.get("components"))
        .and_then(Value::as_object)
        .map(|components| {
            let mut stats: Vec<(u32, WidgetStat)> = components
                .iter()
                .filter_map(|(key, component)| {
                    let index = key.strip_prefix("stat_")?.parse().ok()?;
                    let fields = component.get("fields")?;
                    Some((
                        index,
                        WidgetStat {
                            value: widget_text(fields.get("value")?)?,
                            label: widget_text(fields.get("label")?)?,
                        },
                    ))
                })
                .collect();
            stats.sort_by_key(|(index, _)| *index);
            stats.into_iter().map(|(_, stat)| stat).collect()
        })
        .unwrap_or_default();
    let collection = bottom
        .filter(|bottom| {
            bottom.get("layout").and_then(Value::as_str) == Some("widget_bottom_collection")
        })
        .and_then(|bottom| bottom.get("components"))
        .and_then(Value::as_object)
        .map(|components| {
            let mut items: Vec<(u32, WidgetItem)> = components
                .iter()
                .filter_map(|(key, component)| {
                    let index = key.strip_prefix("item_")?.parse().ok()?;
                    let fields = component.get("fields");
                    let name = fields
                        .and_then(|fields| fields.get("name"))
                        .and_then(widget_text);
                    let description = fields
                        .and_then(|fields| fields.get("description"))
                        .and_then(widget_text);
                    let image = fields
                        .and_then(|fields| fields.get("image"))
                        .and_then(|image| widget_asset(image, &assets));
                    if name.is_none() && description.is_none() && image.is_none() {
                        return None;
                    }
                    Some((
                        index,
                        WidgetItem {
                            name,
                            description,
                            image,
                        },
                    ))
                })
                .collect();
            items.sort_by_key(|(index, _)| *index);
            items.into_iter().map(|(_, item)| item).collect()
        })
        .unwrap_or_default();
    WidgetConfig {
        application_id,
        display_name: config
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        title,
        subtitles,
        hero,
        stats,
        collection,
        mini,
        accessory,
    }
}

fn widget_accessory_from_value(
    config: &Value,
    assets: &HashMap<String, WidgetAsset>,
) -> Option<WidgetAccessory> {
    let accessory = config.get("surfaces")?.get("activity_accessory")?;
    let fields = accessory
        .get("components")
        .and_then(|components| components.get("stat"))
        .and_then(|stat| stat.get("fields"));
    Some(WidgetAccessory {
        layout: accessory
            .get("layout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        label: fields
            .and_then(|fields| fields.get("label"))
            .and_then(widget_text),
        text: fields
            .and_then(|fields| fields.get("text"))
            .and_then(widget_text),
        icon: fields
            .and_then(|fields| fields.get("icon"))
            .and_then(|icon| widget_image(icon, assets)),
    })
}

fn mini_profile_from_value(
    config: &Value,
    assets: &HashMap<String, WidgetAsset>,
) -> Option<MiniProfile> {
    let mini = config.get("surfaces")?.get("mini_profile")?;
    let components = mini.get("components");
    let fields = components
        .and_then(|components| components.get("stat"))
        .and_then(|stat| stat.get("fields"));
    let image = |name: &str| {
        components
            .and_then(|components| components.get(name))
            .and_then(|component| component.get("fields"))
            .and_then(|fields| fields.get("image"))
            .and_then(|image| widget_image(image, assets))
    };
    let image = image("contained_image").or_else(|| image("hero_image"));
    Some(MiniProfile {
        layout: mini
            .get("layout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        text: fields
            .and_then(|fields| fields.get("text"))
            .and_then(widget_text),
        label: fields
            .and_then(|fields| fields.get("label"))
            .and_then(widget_text),
        icon: fields
            .and_then(|fields| fields.get("icon"))
            .and_then(|icon| widget_image(icon, assets)),
        image,
    })
}

fn widget_image(field: &Value, assets: &HashMap<String, WidgetAsset>) -> Option<WidgetImage> {
    let value = field.get("value")?.as_str()?;
    match field.get("value_type")?.as_str()? {
        "application_asset" => assets.get(value).cloned().map(WidgetImage::Asset),
        "data" if field.get("presentation_type").and_then(Value::as_str) == Some("image") => {
            // Only data fields carry a fallback; an unresolvable one is simply dropped.
            let fallback = field
                .get("fallback")
                .and_then(|fallback| widget_image(fallback, assets))
                .map(Box::new);
            Some(WidgetImage::Data {
                key: value.to_owned(),
                fallback,
            })
        }
        _ => None,
    }
}

fn widget_text(field: &Value) -> Option<WidgetText> {
    let value = field.get("value")?.as_str()?;
    match field.get("value_type")?.as_str()? {
        "data" => Some(WidgetText::Data(value.to_owned())),
        "custom_string" => Some(WidgetText::Literal(value.to_owned())),
        _ => None,
    }
}

fn widget_asset(field: &Value, assets: &HashMap<String, WidgetAsset>) -> Option<WidgetAsset> {
    if field.get("value_type").and_then(Value::as_str) != Some("application_asset") {
        return None;
    }
    let key = field.get("value").and_then(Value::as_str)?;
    assets.get(key).cloned()
}

fn resolved_assets(config: &Value) -> HashMap<String, WidgetAsset> {
    config
        .get("resolved_assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let key = asset.get("key").and_then(Value::as_str)?.to_owned();
            let asset_id = asset.get("asset_id").and_then(id_value)?;
            let metadata = asset.get("metadata");
            let width = metadata
                .and_then(|metadata| metadata.get("width"))
                .and_then(Value::as_u64)
                .and_then(|width| u32::try_from(width).ok())
                .unwrap_or_default();
            let height = metadata
                .and_then(|metadata| metadata.get("height"))
                .and_then(Value::as_u64)
                .and_then(|height| u32::try_from(height).ok())
                .unwrap_or_default();
            let animated = metadata
                .and_then(|metadata| metadata.get("is_animated"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some((
                key,
                WidgetAsset {
                    asset_id,
                    width,
                    height,
                    animated,
                },
            ))
        })
        .collect()
}

const INVITE_HOSTS: [(&str, bool); 3] = [
    ("discord.gg/", false),
    ("discord.com/invite/", true),
    ("discordapp.com/invite/", true),
];
const INVITE_LIMIT: usize = 3;
const INVITE_CODE_MAX: usize = 32;

/// Plain scan: codes inside markdown code spans or blocks are not skipped.
pub fn invite_codes(content: &str) -> Vec<String> {
    let mut found: Vec<(usize, usize, bool)> = Vec::new();
    for (host, subdomains) in INVITE_HOSTS {
        for (index, _) in content.match_indices(host) {
            found.push((index, host.len(), subdomains));
        }
    }
    found.sort_by_key(|(index, _, _)| *index);

    let mut codes: Vec<String> = Vec::new();
    let mut consumed = 0;
    for (index, prefix_len, subdomains) in found {
        if index < consumed {
            continue;
        }
        let Some(start) = invite_url_start(content, index, subdomains) else {
            continue;
        };
        if !invite_link_boundary(content, start) {
            continue;
        }
        let code_start = index + prefix_len;
        let code_end = content[code_start..]
            .find(|c: char| !is_invite_code_char(c))
            .map_or(content.len(), |offset| code_start + offset);
        let code = &content[code_start..code_end];
        if code.len() < 2 || code.len() > INVITE_CODE_MAX {
            continue;
        }
        if !codes.iter().any(|existing| existing == code) {
            codes.push(code.to_owned());
            if codes.len() == INVITE_LIMIT {
                break;
            }
        }
        consumed = code_end;
    }
    codes
}

fn invite_url_start(content: &str, host_start: usize, subdomains: bool) -> Option<usize> {
    let mut start = host_start;
    if subdomains {
        for subdomain in ["www.", "ptb.", "canary."] {
            if content[..start].ends_with(subdomain) {
                start -= subdomain.len();
                break;
            }
        }
    }
    for scheme in ["https://", "http://"] {
        if let Some(before) = content[..start].strip_suffix(scheme) {
            return Some(before.len());
        }
    }
    Some(start)
}

fn invite_link_boundary(content: &str, start: usize) -> bool {
    start == 0
        || !content[..start].chars().next_back().is_some_and(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | '@')
        })
}

fn is_invite_code_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Invite {
    #[serde(default)]
    pub code: String,
    #[serde(default, deserialize_with = "optional_id")]
    pub guild_id: Option<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub guild: Option<InviteGuild>,
    #[serde(default, deserialize_with = "default_value")]
    pub profile: Option<InviteProfile>,
    #[serde(default, deserialize_with = "default_value")]
    pub expires_at: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub approximate_member_count: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub approximate_presence_count: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct InviteGuild {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<u64>,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "default_value")]
    pub icon: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub banner: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub splash: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct InviteProfile {
    #[serde(default, deserialize_with = "optional_id")]
    pub id: Option<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub icon_hash: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub banner_hash: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub custom_banner_hash: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub member_count: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub online_count: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "ids")]
    pub game_application_ids: Vec<u64>,
    #[serde(default, deserialize_with = "default_value")]
    pub tag: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub badge_hash: Option<String>,
}

#[derive(Deserialize)]
pub struct SettingsUpdate {
    pub settings: SettingsProto,
    #[serde(default)]
    pub partial: bool,
}

#[derive(Deserialize)]
pub struct SettingsProto {
    #[serde(rename = "type")]
    pub kind: u8,
    pub proto: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROFILE: &str = include_str!("fixtures/profile_modal.json");
    const PRODUCTS: &str = include_str!("fixtures/collectibles_products.json");

    #[test]
    fn profile_parses_decorations_and_profile_fields() {
        let profile: Profile = serde_json::from_str(PROFILE).unwrap();
        let user = &profile.user.user;
        assert_eq!(user.id, 1191168931004416004);
        let decoration = user.avatar_decoration_data.clone().flatten().unwrap();
        assert_eq!(decoration.asset, "a_f000000000000000000000000000000b");
        assert_eq!(decoration.sku_id, Some(1191169212022784071));
        let nameplate = |collectibles: Option<&Collectibles>| {
            collectibles
                .and_then(|c| c.nameplate.as_ref())
                .map(|n| n.asset.clone())
        };
        assert_eq!(
            nameplate(user.collectibles.as_ref().and_then(Option::as_ref)).as_deref(),
            Some("nameplates/sample_plate_one/1191169216217088072/")
        );
        let member = profile.guild_member.as_ref().unwrap();
        assert_eq!(
            nameplate(member.member.collectibles.as_ref().and_then(Option::as_ref)).as_deref(),
            Some("nameplates/sample_plate_two/1191169149108224056/")
        );
        assert_eq!(
            member.member.roles.as_deref(),
            Some([1191168985530368017].as_slice())
        );
        let styles = user.display_name_styles.clone().flatten().unwrap();
        assert_eq!((styles.font_id, styles.effect_id), (Some(14), Some(8)));
        assert_eq!(styles.colors.len(), 4);
        let user_profile = profile.user_profile.as_ref().unwrap();
        assert_eq!(user_profile.pronouns.as_deref(), Some("they/them"));
        assert_eq!(
            user_profile.theme_colors.as_deref(),
            Some(&[11687486, 14261667][..])
        );
        assert_eq!(
            user_profile.profile_effect.as_ref().map(|e| e.sku_id),
            Some(1191169220411392073)
        );
        assert_eq!(
            profile.user.banner.as_deref(),
            Some("f000000000000000000000000000002e")
        );
        assert_eq!(profile.badges.len(), 5);
        assert_eq!(profile.badges[0].icon, "f000000000000000000000000000001c");
        assert_eq!(profile.connected_accounts.len(), 3);
        assert_eq!(profile.connected_accounts[1].kind, "github");
        let mutuals = profile.mutual_guilds.as_ref().unwrap();
        assert_eq!(mutuals.len(), 3);
        assert_eq!(mutuals[1].nick.as_deref(), Some("sample.user"));
        assert_eq!(profile.mutual_friends_count, Some(7));
        assert_eq!(profile.legacy_username.as_deref(), Some("Sample#0042"));
        let guild_profile = profile.guild_member_profile.as_ref().unwrap();
        assert!(guild_profile.profile_effect.is_none() && guild_profile.theme_colors.is_none());
        let frames: Vec<_> = user_profile
            .collectibles
            .iter()
            .map(|c| (c.sku_id, c.kind))
            .collect();
        assert_eq!(frames, [(1191169220411392073, 1), (1191169224605696074, 3)]);
    }

    #[test]
    fn profile_survives_malformed_entries_and_missing_mutuals() {
        let mut value: Value = serde_json::from_str(PROFILE).unwrap();
        value["badges"][1] = json!({"id": 5});
        value["user"]["avatar_decoration_data"] = json!({"asset": false});
        value["user_profile"]["theme_colors"] = json!("nope");
        let object = value.as_object_mut().unwrap();
        object.remove("mutual_guilds");
        object.remove("mutual_friends_count");
        let profile: Profile = serde_json::from_value(value).unwrap();
        assert_eq!(profile.badges.len(), 4);
        assert!(profile.user.user.avatar_decoration_data.is_none());
        assert!(profile.mutual_guilds.is_none() && profile.mutual_friends_count.is_none());
    }

    #[test]
    fn profile_parses_mutual_friends() {
        let mut value: Value = serde_json::from_str(PROFILE).unwrap();
        value["mutual_friends"] = json!([
            {"id": "42", "username": "friend", "global_name": "Friend"}
        ]);
        let profile: Profile = serde_json::from_value(value).unwrap();
        let friends = profile.mutual_friends.unwrap();
        assert_eq!(friends.len(), 1);
        assert_eq!(friends[0].id, 42);
        assert_eq!(friends[0].username, "friend");
    }

    #[test]
    fn profile_reads_wishlist_ids_from_settings_keys() {
        let profile: Profile = serde_json::from_value(json!({
            "user": {"id": "1191169170079744061", "username": "sample.collector"},
            "wishlist_settings": {
                "1191169174274048062": {
                    "visibility": 1,
                    "updated_at": "2024-01-10T00:00:00.000+00:00"
                },
                "not-a-snowflake": {"visibility": 1}
            }
        }))
        .unwrap();
        assert_eq!(profile.wishlist_ids, [1191169174274048062]);

        let profile: Profile = serde_json::from_value(json!({
            "user": {"id": "1", "username": "bare"}
        }))
        .unwrap();
        assert!(profile.wishlist_ids.is_empty());
    }

    #[test]
    fn wishlist_decodes_items_and_skips_malformed_entries() {
        let wishlist: Wishlist = serde_json::from_value(json!({
            "id": "1191169174274048062",
            "user_id": "1191169170079744061",
            "wishlist_items": [
                {
                    "sku_id": "1191169300103168092",
                    "sku_name": "Sample Add-on Bundle",
                    "is_owned": false,
                    "sku_product_line": "monthly",
                    "sku": {
                        "application_id": "1191169081999360040",
                        "name": "Sample Add-on Bundle",
                        "thumbnail_asset_id": "1191169312686080095",
                        "tenant_metadata": {
                            "social_layer": {"card_image_asset_id": "1191169316880384096"}
                        },
                        "price": {
                            "amount": 2049,
                            "currency": "usd",
                            "currency_exponent": 2
                        }
                    }
                },
                {"sku_name": "Malformed", "is_owned": true}
            ],
            "applications": [{
                "id": "1191169040056320030",
                "name": "Example Game",
                "icon": "icon-hash",
                "cover_image": "f0000000000000000000000000000025"
            }]
        }))
        .unwrap();
        assert_eq!(wishlist.id, 1191169174274048062);
        assert_eq!(wishlist.items.len(), 1);
        let item = &wishlist.items[0];
        assert_eq!(item.sku_id, 1191169300103168092);
        assert_eq!(item.name, "Sample Add-on Bundle");
        assert!(!item.is_owned);
        assert_eq!(item.application_id, Some(1191169081999360040));
        assert_eq!(item.thumbnail_asset_id, Some(1191169312686080095));
        assert_eq!(item.card_image_asset_id, Some(1191169316880384096));
        let price = item.price.as_ref().unwrap();
        assert_eq!((price.amount, price.currency.as_str(), price.exponent), (2049, "usd", 2));
        assert_eq!(wishlist.applications.len(), 1);
        assert_eq!(
            wishlist.applications[0].cover_image.as_deref(),
            Some("f0000000000000000000000000000025")
        );
    }

    #[test]
    fn user_badges_parse_leniently_and_match_profile_badges() {
        let badges: UserBadges = serde_json::from_value(json!({
            "badges": [
                {
                    "badge_id": 1,
                    "name": "Nitro",
                    "description": "unexpected profile-style field",
                    "info_label": "Subscriber since 1/1/24",
                    "rarity": 2,
                    "simple_icon_raster_url": "https://cdn.discordapp.com/assets/content/nitro.png",
                    "complex_icon_static_url": "https://cdn.discordapp.com/assets/content/nitro.svg",
                    "current_tier": "platinum",
                    "tiers": [
                        {
                            "key": "platinum",
                            "name": "Platinum",
                            "simple_icon_raster_url": "https://cdn.discordapp.com/assets/content/tier.png"
                        },
                        {"key": "gold"}
                    ],
                    "unexpected": {"nested": true}
                },
                {"badge_id": 13},
                {"badge_id": 99, "name": "Mystery", "unexpected": true}
            ]
        }))
        .unwrap();
        assert_eq!(badges.badges.len(), 3);
        assert_eq!(badges.badges[0].name, "Nitro");
        assert_eq!(badges.badges[1].name, "");
        assert!(badges.badges[1].info_label.is_none());
        assert!(badges.badges[1].tiers.is_empty());
        assert_eq!(badges.badges[0].tier().unwrap().key, "platinum");
        assert!(badges.badges[1].tier().is_none());

        let badge = |id: &str, description: &str, icon: &str| Badge {
            id: id.to_owned(),
            description: description.to_owned(),
            icon: icon.to_owned(),
            link: None,
        };
        assert_eq!(
            badges
                .detail_for(&badge("unknown", "nope", "tier"))
                .map(|detail| detail.badge_id),
            Some(1)
        );
        assert_eq!(
            badges
                .detail_for(&badge("premium_tenure_12_month_v2", "nope", "none"))
                .map(|detail| detail.badge_id),
            Some(1)
        );
        assert_eq!(
            badges
                .detail_for(&badge("unknown", "mystery", "none"))
                .map(|detail| detail.badge_id),
            Some(99)
        );
        assert!(
            badges
                .detail_for(&badge("unknown", "nope", "none"))
                .is_none()
        );
        assert_eq!(badge_id_for("guild_booster_lvl7"), Some(12));
    }

    #[test]
    fn user_cosmetics_distinguish_absent_from_null() {
        let absent: User = serde_json::from_value(json!({"id": "1"})).unwrap();
        assert!(absent.avatar_decoration_data.is_none());
        assert!(absent.collectibles.is_none() && absent.display_name_styles.is_none());
        let cleared: User = serde_json::from_value(json!({
            "id": "1",
            "avatar_decoration_data": null,
            "collectibles": null,
            "display_name_styles": null
        }))
        .unwrap();
        assert!(matches!(cleared.avatar_decoration_data, Some(None)));
        assert!(matches!(cleared.collectibles, Some(None)));
        assert!(matches!(cleared.display_name_styles, Some(None)));
        let member: PartialMember = serde_json::from_value(json!({
            "user": {"id": "1"},
            "collectibles": {"nameplate": {"asset": "nameplates/a/1/", "sku_id": "1"}}
        }))
        .unwrap();
        let collectibles = member.collectibles.flatten().unwrap();
        assert_eq!(collectibles.nameplate.unwrap().asset, "nameplates/a/1/");
    }

    #[test]
    fn collectible_products_parse_effects_and_frames() {
        let products: Vec<CollectibleProduct> = serde_json::from_str(PRODUCTS).unwrap();
        assert_eq!(
            products.iter().map(|p| p.kind).collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        let decoration = &products[0].items[0];
        assert_eq!(
            decoration.asset.as_deref(),
            Some("a_f0000000000000000000000000000016")
        );
        let effect = &products[1].items[0];
        assert_eq!(effect.kind, 1);
        assert!(effect.reduced_motion_src.is_some() && effect.accessibility_label.is_some());
        let layers: Vec<_> = effect
            .effects
            .iter()
            .map(|e| (e.looped, e.duration, e.start, e.loop_delay, e.z_index))
            .collect();
        assert_eq!(
            layers,
            [(false, 4981, 0, 0, 110), (true, 4980, 7981, 3000, 100)]
        );
        let nameplate = &products[2].items[0];
        assert_eq!(nameplate.palette.as_deref(), Some("forest"));
        let frame = &products[3].items[0];
        assert_eq!(frame.layers.len(), 6);
        assert_eq!(
            (
                frame.inner_width,
                frame.overflow_top,
                frame.overflow_bottom,
                frame.overflow_horizontal
            ),
            (Some(1200), Some(304), Some(212), Some(56))
        );
        let top = &frame.layers[1];
        assert_eq!(
            (
                top.id,
                top.kind.as_str(),
                top.order.as_str(),
                top.anchor.as_str(),
                top.responsive
            ),
            (1191169279131648087, "staple", "front", "top", false)
        );
    }

    #[test]
    fn reply_tristate_and_depth_one_and_partial_fields() {
        let base = json!({"id":"1","channel_id":"2","author":{"id":"3"},"timestamp":"2026-10-22T00:00:00Z"});
        let message: Message = serde_json::from_value(base.clone()).unwrap();
        assert!(message.referenced_message.is_none());
        assert_eq!(message.kind, 0);
        let mut value = base;
        value["referenced_message"] = Value::Null;
        let message: Message = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(message.referenced_message, Some(None)));
        value["referenced_message"] =
            json!({"id":"4","author":{"id":"5"},"referenced_message":{"id":null,"author":false}});
        let message: Message = serde_json::from_value(value).unwrap();
        assert_eq!(message.referenced_message.unwrap().unwrap().id, 4);
        let update: MessageUpdate =
            serde_json::from_value(json!({"id":"1","channel_id":"2"})).unwrap();
        assert!(
            update.kind.is_none()
                && update.mentions.is_none()
                && update.referenced_message.is_none()
        );
        let update: MessageUpdate = serde_json::from_value(json!({"id":"1","channel_id":"2","referenced_message":null,"edited_timestamp":null,"mention_roles":["8","bad"],"future":true})).unwrap();
        assert!(matches!(update.referenced_message, Some(None)));
        assert_eq!(update.edited_timestamp, Some(None));
        assert_eq!(update.mention_roles, Some(vec![8]));
        let member: PartialMember =
            serde_json::from_value(json!({"user":{"id":"3"},"nick":null})).unwrap();
        assert_eq!(member.nick, Some(None));
        assert!(member.roles.is_none());
    }

    #[test]
    fn nonce_accepts_strings_integers_and_garbage() {
        let base = json!({"id":"1","channel_id":"2","author":{"id":"3"},"timestamp":"2026-10-22T00:00:00Z"});
        let parse = |extra: Value| {
            let mut value = base.clone();
            value["nonce"] = extra;
            serde_json::from_value::<Message>(value).unwrap().nonce
        };
        assert_eq!(
            parse(json!("1176000000000000000")),
            Some(1176000000000000000)
        );
        assert_eq!(
            parse(json!(1176000000000000000u64)),
            Some(1176000000000000000)
        );
        assert_eq!(parse(json!("not-a-snowflake")), None);
        assert_eq!(parse(json!(null)), None);
        assert_eq!(parse(json!({"nested": true})), None);
        assert_eq!(serde_json::from_value::<Message>(base).unwrap().nonce, None);
    }

    #[test]
    fn message_sticker_items_parse_and_default() {
        let base = json!({"id":"1","channel_id":"2","author":{"id":"3"},"timestamp":"2026-10-22T00:00:00Z"});
        let message: Message = serde_json::from_value(base.clone()).unwrap();
        assert!(message.sticker_items.is_empty());

        let message: Message = serde_json::from_value(json!({
            "id": "1",
            "channel_id": "2",
            "author": {"id": "3"},
            "timestamp": "2026-10-22T00:00:00Z",
            "sticker_items": [
                {"id": "99", "name": "wave", "format_type": 2},
                {"name": "no-id", "format_type": 1},
                {"id": "not-a-snowflake", "name": "skipped"}
            ]
        }))
        .unwrap();
        assert_eq!(message.sticker_items.len(), 2);
        assert_eq!(
            (
                message.sticker_items[0].id,
                message.sticker_items[0].name.as_str(),
                message.sticker_items[0].format_type
            ),
            (99, "wave", 2)
        );
        assert_eq!(
            (
                message.sticker_items[1].id,
                message.sticker_items[1].name.as_str(),
                message.sticker_items[1].format_type
            ),
            (0, "no-id", 1)
        );

        let snapshot: SnapshotMessage = serde_json::from_value(json!({
            "sticker_items": [{"id": "7", "name": "forwarded", "format_type": 4}]
        }))
        .unwrap();
        assert_eq!(snapshot.sticker_items[0].name, "forwarded");
        assert_eq!(snapshot.sticker_items[0].format_type, 4);

        let update: MessageUpdate = serde_json::from_value(json!({
            "id": "1",
            "channel_id": "2",
            "sticker_items": [{"id": "8", "name": "edited", "format_type": 3}]
        }))
        .unwrap();
        assert_eq!(update.sticker_items.unwrap()[0].format_type, 3);
        let plain: MessageUpdate =
            serde_json::from_value(json!({"id": "1", "channel_id": "2"})).unwrap();
        assert!(plain.sticker_items.is_none());
    }

    #[test]
    fn message_snapshots_are_lenient() {
        let message: Message = serde_json::from_value(json!({
            "id": "1",
            "channel_id": "2",
            "author": {"id": "3"},
            "timestamp": "2026-10-22T00:00:00Z",
            "message_reference": {"type": 1, "channel_id": "4", "guild_id": "5"},
            "message_snapshots": [
                {"message": {
                    "content": "forwarded",
                    "embeds": [{}],
                    "attachments": [{"id": "6"}],
                    "components": [],
                    "flags": 0,
                    "type": 0,
                    "timestamp": "not a timestamp",
                    "edited_timestamp": "2026-10-22T01:00:00Z"
                }},
                {"message": "malformed"}
            ]
        }))
        .unwrap();
        assert_eq!(message.message_reference.unwrap().kind, 1);
        assert_eq!(message.message_snapshots.len(), 1);
        let snapshot = &message.message_snapshots[0].message;
        assert_eq!(snapshot.content, "forwarded");
        assert_eq!(snapshot.attachments[0].id, 6);
        assert_eq!(snapshot.embeds.len(), 1);
        assert!(snapshot.timestamp.is_none());
        assert_eq!(
            snapshot.edited_timestamp.as_deref(),
            Some("2026-10-22T01:00:00Z")
        );
        let update: MessageUpdate = serde_json::from_value(json!({
            "id": "1",
            "channel_id": "2",
            "message_snapshots": [{"message": {"content": "edited"}}]
        }))
        .unwrap();
        assert_eq!(
            update.message_snapshots.unwrap()[0].message.content,
            "edited"
        );
    }

    #[test]
    fn profile_widgets_decode_per_entry() {
        let profile: Profile = serde_json::from_value(json!({
            "user": {"id": "1", "username": "synthetic"},
            "widgets": [
                {
                    "id": "10",
                    "data": {
                        "type": "favorite_games",
                        "games": [{"game_id": "20", "comment": "fun", "tags": ["cozy"]}]
                    }
                },
                {"id": "11", "data": {"type": "application", "application_id": "30"}},
                {"id": "12", "data": {"type": "application", "application_id": "not-an-id"}},
                {"id": "13", "data": {"type": "personal"}},
                {
                    "id": "14",
                    "data": {
                        "type": "played_games",
                        "games": [{"game_id": "21"}, {"game_id": "22"}]
                    }
                },
                {"data": {"type": "personal"}}
            ]
        }))
        .unwrap();
        assert_eq!(profile.widgets.len(), 4);
        let WidgetData::FavoriteGames { games } = &profile.widgets[0].data else {
            panic!("wrong widget kind");
        };
        assert_eq!(games[0].game_id, 20);
        assert_eq!(games[0].comment.as_deref(), Some("fun"));
        assert_eq!(games[0].tags, ["cozy"]);
        assert!(matches!(
            profile.widgets[1].data,
            WidgetData::Application { application_id: 30 }
        ));
        assert!(matches!(profile.widgets[2].data, WidgetData::Unknown));
        let WidgetData::PlayedGames { games } = &profile.widgets[3].data else {
            panic!("Games I Like was not decoded");
        };
        assert_eq!(
            games.iter().map(|game| game.game_id).collect::<Vec<_>>(),
            [21, 22]
        );
    }

    #[test]
    fn presence_activity_decodes_application_and_timestamps() {
        let presence: Presence = serde_json::from_value(json!({
            "user_id": "2",
            "activities": [
                {
                    "type": 0,
                    "name": "Synthetic Game",
                    "application_id": "123",
                    "timestamps": {"start": 1700000000000u64, "end": "1700000001000"}
                },
                {
                    "type": 0,
                    "name": "Broken Game",
                    "application_id": "not-an-id",
                    "timestamps": {"start": "not-a-number", "end": null}
                },
                {
                    "type": 0,
                    "name": "String Timestamps",
                    "timestamps": "not-an-object"
                }
            ]
        }))
        .unwrap();
        let activities = presence.activities.unwrap();
        assert_eq!(activities.len(), 3);
        assert_eq!(activities[0].application_id, Some(123));
        let timestamps = activities[0].timestamps.as_ref().unwrap();
        assert_eq!(timestamps.start, Some(1_700_000_000_000));
        assert!(activities[1].application_id.is_none());
        let timestamps = activities[1].timestamps.as_ref().unwrap();
        assert!(timestamps.start.is_none());
        assert!(activities[2].timestamps.is_none());
    }

    #[test]
    fn presence_activity_decodes_platform_and_flags_leniently() {
        let presence: Presence = serde_json::from_value(json!({
            "user_id": "2",
            "activities": [
                {"type": 0, "name": "Console", "platform": "xbox", "flags": 256},
                {"type": 0, "name": "Desktop"},
                {"type": 0, "name": "Null Flags", "platform": null, "flags": null},
                {"type": 0, "name": "Garbage Flags", "platform": 7, "flags": "not-a-number"}
            ]
        }))
        .unwrap();
        let activities = presence.activities.unwrap();
        assert_eq!(activities.len(), 4);
        assert_eq!(activities[0].platform.as_deref(), Some("xbox"));
        assert_eq!(activities[0].flags, 256);
        assert!(activities[1].platform.is_none());
        assert_eq!(activities[1].flags, 0);
        assert!(activities[2].platform.is_none());
        assert_eq!(activities[2].flags, 0);
        assert!(activities[3].platform.is_none());
        assert_eq!(activities[3].flags, 0);
    }

    #[test]
    fn presence_activity_decodes_created_at_from_gateway_payload() {
        let presence: Presence = serde_json::from_value(json!({
            "user_id": "2",
            "activities": [{
                "type": 0,
                "session_id": "test-session",
                "platform": "xbox",
                "official_game_id": null,
                "name": "Example Idle Game",
                "id": "f00000000000000f",
                "flags": 288,
                "details": "Example Adventurer",
                "created_at": 1704067200000u64,
                "content_classification": {"loaded": true, "data": null},
                "assets": {
                    "large_image": "mp:avatars/1191168989724672018/a_f0000000000000000000000000000026.gif??size=4096&keep_aspect_ratio=true&quality=lossless"
                },
                "application_id": "1191168989724672018"
            }]
        }))
        .unwrap();
        let activities = presence.activities.unwrap();
        let activity = &activities[0];
        assert_eq!(activity.created_at, Some(1_704_067_200_000));
        assert_eq!(activity.flags, 288);
        assert_eq!(activity.platform.as_deref(), Some("xbox"));
        assert_eq!(activity.application_id, Some(1191168989724672018));
        assert!(activity.timestamps.is_none());
    }

    #[test]
    fn games_and_public_applications_decode_per_entry() {
        let games: Vec<Game> = parse_entries(json!([
            {
                "id": "1",
                "name": "Synthetic Game",
                "icon_hash": "hash",
                "third_party_skus": [
                    {"distributor": "gop", "id": "ignored"},
                    {"distributor": "steam", "id": "3513350"},
                    {"distributor": "steam", "id": "999"}
                ],
                "media": {
                    "cover_url": "https://cdn.example/cover.png"
                }
            },
            {"id": "not-an-id", "name": "dropped"},
            {"id": "2", "name": "Bare"}
        ]));
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].name, "Synthetic Game");
        assert_eq!(games[0].steam_app_id.as_deref(), Some("3513350"));
        assert_eq!(
            games[0].media.cover_url.as_deref(),
            Some("https://cdn.example/cover.png")
        );
        assert!(games[1].icon_hash.is_none());
        assert!(games[1].steam_app_id.is_none());
        assert!(games[1].media.cover_url.is_none());

        let applications: Vec<PublicApplication> = parse_entries(json!([
            {"id": "3", "name": "Synthetic App", "icon": "icon", "cover_image": "cover"},
            {"id": "bad", "name": "dropped"}
        ]));
        assert_eq!(applications.len(), 1);
        assert_eq!(applications[0].id, 3);
        assert_eq!(applications[0].icon.as_deref(), Some("icon"));
    }

    #[test]
    fn application_identity_stats_flatten_dynamic() {
        let identities: ApplicationIdentities = serde_json::from_value(json!({
            "identities": [
                {
                    "application_id": "7",
                    "profile": {
                        "username": "synthetic",
                        "data": {"dynamic": [
                            {"type": 1, "name": "server", "value": "SEA"},
                            {"type": 2, "name": "level", "value": 56.0},
                            {"type": 2, "name": "broken", "value": "not-a-number"}
                        ]}
                    }
                },
                {"application_id": "bad"}
            ]
        }))
        .unwrap();
        assert_eq!(identities.identities.len(), 1);
        let profile = identities.identities[0].profile.as_ref().unwrap();
        assert_eq!(profile.username.as_deref(), Some("synthetic"));
        assert_eq!(profile.stats.len(), 3);
        assert_eq!(profile.stats[0].name, "server");
        assert_eq!(profile.stats[0].value, StatValue::Text("SEA".into()));
        assert_eq!(profile.stats[1].value, StatValue::Number(56.0));
        assert_eq!(profile.stats[2].value, StatValue::Number(0.0));
    }

    #[test]
    fn application_identity_stats_include_primary_values() {
        let identities: ApplicationIdentities = serde_json::from_value(json!({
            "identities": [{
                "application_id": "1191169081999360040",
                "profile": {
                    "username": "sample.player",
                    "data": {
                        "primary": {
                            "season": "Sample Season",
                            "playtime_hours": 0.78,
                            "featured_played_character_image": {
                                "id": "1191169325268992098",
                                "url": "https://example.com/raw.png",
                                "proxy_url": "https://images-ext-1.discordapp.net/external/hash/raw.png",
                                "width": 1920,
                                "height": 2200
                            }
                        }
                    }
                }
            }]
        }))
        .unwrap();
        let profile = identities.identities[0].profile.as_ref().unwrap();
        let names: Vec<_> = profile.stats.iter().map(|stat| stat.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "season",
                "playtime_hours",
                "featured_played_character_image"
            ]
        );
        let stat = |name: &str| {
            profile
                .stats
                .iter()
                .find(|stat| stat.name == name)
                .unwrap()
        };
        assert_eq!(
            stat("season").value,
            StatValue::Text("Sample Season".into())
        );
        assert_eq!(stat("playtime_hours").value, StatValue::Number(0.78));
        assert_eq!(
            stat("featured_played_character_image").value,
            StatValue::Image {
                url: "https://images-ext-1.discordapp.net/external/hash/raw.png".into(),
                width: Some(1920),
                height: Some(2200),
            }
        );
        assert_eq!(
            profile.image("featured_played_character_image"),
            Some((
                "https://images-ext-1.discordapp.net/external/hash/raw.png",
                Some(1920),
                Some(2200)
            ))
        );
    }

    #[test]
    fn application_identity_dynamic_stats_win_over_primary_duplicates() {
        let identities: ApplicationIdentities =
            serde_json::from_value(json!({
                "identities": [{
                    "application_id": "7",
                    "profile": {
                        "data": {
                            "dynamic": [
                                {"type": 2, "name": "level", "value": 56.0},
                                {"type": 1, "name": "server", "value": "SEA"}
                            ],
                            "primary": {
                                "server": "Ignored",
                                "season": "Sample Season"
                            }
                        }
                    }
                }]
            }))
            .unwrap();
        let profile = identities.identities[0].profile.as_ref().unwrap();
        let names: Vec<_> = profile.stats.iter().map(|stat| stat.name.as_str()).collect();
        assert_eq!(names, ["level", "server", "season"]);
        let server = profile
            .stats
            .iter()
            .find(|stat| stat.name == "server")
            .unwrap();
        assert_eq!(server.value, StatValue::Text("SEA".into()));
    }

    #[test]
    fn outbox_decodes_played_games_listening_and_traits() {
        let outbox: Outbox = serde_json::from_value(json!({
            "entries": [
                {
                    "id": "10",
                    "started_at": "2026-01-01T00:00:00Z",
                    "traits": [
                        {"type": 2, "duration_seconds": 90},
                        {"type": 8, "streak_count_days": 3},
                        {"type": 9, "trending": 1}
                    ],
                    "extra": {
                        "type": "played_game_extra",
                        "game_name": "Synthetic Game",
                        "application_id": "20"
                    }
                },
                {
                    "id": "11",
                    "expires_at": "2026-01-01T00:10:00Z",
                    "traits": [{"type": 3, "is_live": true}],
                    "extra": {
                        "type": "listened_session_extra",
                        "last_update": "2026-01-01T00:05:00Z",
                        "entries": [
                            {"media": {
                                "title": "Track One",
                                "parent_title": "Album",
                                "image_url": "https://i.scdn.co/image/synthetic",
                                "artists": [{"name": "Artist A"}, {"name": "Artist B"}]
                            }},
                            {"media": {"title": "Track Two", "artists": []}}
                        ]
                    }
                },
                {
                    "id": "12",
                    "traits": [
                        {"type": 1, "first_time": true},
                        {"type": 5, "resurrected_last_played": "2025-12-01T00:00:00Z"}
                    ],
                    "extra": {"type": "played_game_extra", "game_name": "Another Game"}
                },
                {"id": "13", "extra": {"type": "future_extra"}},
                {"id": "bad", "extra": {"type": "played_game_extra"}}
            ]
        }))
        .unwrap();
        assert_eq!(outbox.entries.len(), 4);
        assert_eq!(outbox.entries[0].id, 10);
        assert_eq!(
            outbox.entries[0].started_at.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
        assert_eq!(
            outbox.entries[0].traits,
            [
                OutboxTrait::Duration { seconds: 90 },
                OutboxTrait::Streak { days: 3 },
                OutboxTrait::Trending
            ]
        );
        assert!(matches!(
            &outbox.entries[0].kind,
            OutboxKind::PlayedGame { name, application_id: Some(20) } if name == "Synthetic Game"
        ));
        assert_eq!(outbox.entries[1].traits, [OutboxTrait::Live]);
        assert_eq!(
            outbox.entries[1].expires_at.as_deref(),
            Some("2026-01-01T00:10:00Z")
        );
        let OutboxKind::Listened {
            tracks,
            last_update,
        } = &outbox.entries[1].kind
        else {
            panic!("wrong outbox kind");
        };
        assert_eq!(last_update.as_deref(), Some("2026-01-01T00:05:00Z"));
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].title, "Track One");
        assert_eq!(tracks[0].artists, ["Artist A", "Artist B"]);
        assert_eq!(
            tracks[0].image_url.as_deref(),
            Some("https://i.scdn.co/image/synthetic")
        );
        assert!(tracks[1].artists.is_empty());
        assert_eq!(
            outbox.entries[2].traits,
            [
                OutboxTrait::FirstTime,
                OutboxTrait::Resurrected {
                    last_played: "2025-12-01T00:00:00Z".into()
                }
            ]
        );
        assert!(matches!(
            &outbox.entries[2].kind,
            OutboxKind::PlayedGame {
                application_id: None,
                ..
            }
        ));
        assert!(matches!(outbox.entries[3].kind, OutboxKind::Unknown));
        assert!(outbox.entries[3].traits.is_empty());
    }

    #[test]
    fn outbox_decodes_launched_activities() {
        let outbox: Outbox = serde_json::from_value(json!({
            "entries": [{
                "id": "1191169321074688097",
                "author_id": "1191169170079744061",
                "content_type": 8,
                "traits": [{"type": 2, "duration_seconds": 243}],
                "extra": {
                    "type": "launched_activity_extra",
                    "activity_name": "Example Puzzle",
                    "application_id": "1191169035862016029"
                },
                "participants": ["1191169170079744061"]
            }]
        }))
        .unwrap();
        assert_eq!(outbox.entries.len(), 1);
        let OutboxKind::LaunchedActivity {
            name,
            application_id,
            last_update,
        } = &outbox.entries[0].kind
        else {
            panic!("wrong outbox kind");
        };
        assert_eq!(name, "Example Puzzle");
        assert_eq!(*application_id, Some(1191169035862016029));
        assert!(last_update.is_none());
        assert_eq!(
            outbox.entries[0].traits,
            [OutboxTrait::Duration { seconds: 243 }]
        );
    }

    #[test]
    fn widget_configs_sort_stats_and_resolve_assets() {
        let configs = parse_featured_widget_configs(json!({
            "configs": {
                "100": [
                    {
                        "application_id": "100",
                        "status": "draft",
                        "surfaces": {
                            "widget_top": {
                                "layout": "widget_top_hero",
                                "components": {
                                    "title": {"fields": {"text": {
                                        "value_type": "custom_string",
                                        "value": "Draft"
                                    }}}
                                }
                            }
                        }
                    },
                    {
                        "application_id": "100",
                        "display_name": "Sample Widget",
                        "status": "published",
                        "surfaces": {
                            "widget_top": {
                                "layout": "widget_top_hero",
                                "components": {
                                    "title": {"fields": {"text": {
                                        "value_type": "data",
                                        "value": "username"
                                    }}},
                                    "subtitle_2": {"fields": {"text": {
                                        "value_type": "data",
                                        "value": "server_name"
                                    }}},
                                    "subtitle_10": {"fields": {"text": {
                                        "value_type": "custom_string",
                                        "value": "Tenth"
                                    }}},
                                    "subtitle_1": {"fields": {
                                        "text": {
                                            "value_type": "custom_string",
                                            "value": "First"
                                        },
                                        "label": {
                                            "value_type": "custom_string",
                                            "value": "label one"
                                        },
                                        "icon": {
                                            "value_type": "application_asset",
                                            "value": "icon"
                                        }
                                    }},
                                    "subtitle_3": {"fields": {"text": {
                                        "value_type": "custom_string",
                                        "value": "Third"
                                    }}},
                                    "subtitle_4": {"fields": {"text": {
                                        "value_type": "unsupported",
                                        "value": "Skipped"
                                    }}},
                                    "hero_image": {"fields": {"image": {
                                        "value_type": "application_asset",
                                        "value": "hero"
                                    }}}
                                }
                            },
                            "widget_bottom": {
                                "layout": "widget_bottom_stats",
                                "components": {
                                    "stat_10": {"fields": {
                                        "value": {"value_type": "data", "value": "level"},
                                        "label": {"value_type": "custom_string", "value": "Level"}
                                    }},
                                    "stat_2": {"fields": {
                                        "value": {"value_type": "custom_string", "value": "42"},
                                        "label": {"value_type": "custom_string", "value": "Two"}
                                    }},
                                    "stat_1": {"fields": {
                                        "value": {"value_type": "data", "value": "rank"},
                                        "label": {"value_type": "custom_string", "value": "Rank"}
                                    }},
                                    "stat_3": {"fields": {
                                        "value": {"value_type": "unsupported", "value": "x"},
                                        "label": {"value_type": "custom_string", "value": "Skipped"}
                                    }}
                                }
                            }
                        },
                        "resolved_assets": [
                            {
                                "key": "hero",
                                "asset_id": "900",
                                "metadata": {"width": 640, "height": 480}
                            },
                            {
                                "key": "icon",
                                "asset_id": "902",
                                "metadata": {
                                    "width": 32,
                                    "height": 32,
                                    "is_animated": true
                                }
                            },
                            {
                                "key": "other",
                                "asset_id": "901",
                                "metadata": {"width": 1, "height": 1}
                            }
                        ]
                    }
                ]
            }
        }));
        assert_eq!(configs.len(), 1);
        let config = configs.get(&100).unwrap();
        assert_eq!(config.application_id, 100);
        assert_eq!(config.display_name.as_deref(), Some("Sample Widget"));
        assert_eq!(config.title, Some(WidgetText::Data("username".into())));
        assert_eq!(
            config.subtitles,
            [
                WidgetSubtitle {
                    text: WidgetText::Literal("First".into()),
                    label: Some(WidgetText::Literal("label one".into())),
                    icon: Some(WidgetAsset {
                        asset_id: 902,
                        width: 32,
                        height: 32,
                        animated: true
                    }),
                },
                WidgetSubtitle {
                    text: WidgetText::Data("server_name".into()),
                    label: None,
                    icon: None,
                },
                WidgetSubtitle {
                    text: WidgetText::Literal("Third".into()),
                    label: None,
                    icon: None,
                },
                WidgetSubtitle {
                    text: WidgetText::Literal("Tenth".into()),
                    label: None,
                    icon: None,
                }
            ]
        );
        assert_eq!(
            config.hero,
            Some(WidgetImage::Asset(WidgetAsset {
                asset_id: 900,
                width: 640,
                height: 480,
                animated: false
            }))
        );
        assert!(config.collection.is_empty());
        assert_eq!(config.stats.len(), 3);
        assert_eq!(config.stats[0].value, WidgetText::Data("rank".into()));
        assert_eq!(config.stats[0].label, WidgetText::Literal("Rank".into()));
        assert_eq!(config.stats[1].value, WidgetText::Literal("42".into()));
        assert_eq!(config.stats[1].label, WidgetText::Literal("Two".into()));
        assert_eq!(config.stats[2].value, WidgetText::Data("level".into()));
    }

    #[test]
    fn widget_collections_sort_items_and_skip_empty_ones() {
        let configs = parse_featured_widget_configs(json!({
            "configs": {
                "200": [{
                    "application_id": "200",
                    "display_name": "Collection",
                    "surfaces": {
                        "widget_bottom": {
                            "layout": "widget_bottom_collection",
                            "components": {
                                "item_10": {"fields": {
                                    "name": {"value_type": "custom_string", "value": "Tenth"}
                                }},
                                "item_2": {"fields": {
                                    "name": {"value_type": "custom_string", "value": "Second"},
                                    "description": {"value_type": "data", "value": "bio"},
                                    "image": {"value_type": "application_asset", "value": "art"}
                                }},
                                "item_1": {"fields": {
                                    "name": {"value_type": "custom_string", "value": "First"}
                                }},
                                "item_11": {"fields": {
                                    "name": {"value_type": "unsupported", "value": "Skipped"},
                                    "description": {"value_type": "unsupported", "value": "Skipped"},
                                    "image": {"value_type": "unsupported", "value": "Skipped"}
                                }}
                            }
                        }
                    },
                    "resolved_assets": [{
                        "key": "art",
                        "asset_id": "300",
                        "metadata": {
                            "width": 120,
                            "height": 80,
                            "is_animated": true
                        }
                    }]
                }]
            }
        }));
        let config = configs.get(&200).unwrap();
        assert_eq!(config.collection.len(), 3);
        assert_eq!(
            config.collection[0].name,
            Some(WidgetText::Literal("First".into()))
        );
        assert_eq!(
            config.collection[1].name,
            Some(WidgetText::Literal("Second".into()))
        );
        assert_eq!(
            config.collection[1].description,
            Some(WidgetText::Data("bio".into()))
        );
        assert_eq!(
            config.collection[1].image,
            Some(WidgetAsset {
                asset_id: 300,
                width: 120,
                height: 80,
                animated: true
            })
        );
        assert_eq!(
            config.collection[2].name,
            Some(WidgetText::Literal("Tenth".into()))
        );
        assert!(config.collection[2].description.is_none() && config.collection[2].image.is_none());
        assert!(config.stats.is_empty());
    }

    #[test]
    fn application_widget_configs_carry_the_popout_mini_profile() {
        // Synthetic response for `/applications/{id}/widget-configs`.
        let config = parse_application_widget_configs(
            json!([{
                "config_id": "1191169182662656064",
                "application_id": "1191169178468352063",
                "display_name": "Sample Widget",
                "status": "published",
                "surfaces": {
                    "mini_profile": {"layout": "mini_profile_contained_stat", "components": {
                        "stat": {"fields": {"text": {"value_type": "custom_string", "presentation_type": "text", "value": "Sample statistic"}}},
                        "contained_image": {"fields": {"image": {"value_type": "application_asset", "presentation_type": "image", "value": "sample_contained_image"}}}
                    }},
                    "widget_top": {"layout": "widget_top_hero", "components": {
                        "title": {"fields": {"text": {"value_type": "custom_string", "presentation_type": "text", "value": "Sample heading"}}}
                    }}
                },
                "resolved_assets": [{"key": "sample_contained_image", "asset_id": "1191169186856960065"}]
            }]),
            1191169178468352063,
        )
        .unwrap();
        assert_eq!(config.title, Some(WidgetText::Literal("Sample heading".into())));
        let mini = config.mini.unwrap();
        assert_eq!(mini.text, Some(WidgetText::Literal("Sample statistic".into())));
        assert!(
            matches!(mini.image, Some(WidgetImage::Asset(asset)) if asset.asset_id == 1191169186856960065)
        );
    }

    #[test]
    fn widget_configs_carry_data_heroes() {
        let config = parse_application_widget_configs(
            json!([{
                "application_id": "1191168989724672018",
                "status": "published",
                "surfaces": {
                    "widget_top": {
                        "layout": "widget_top_hero",
                        "components": {
                            "hero_image": {"fields": {"image": {
                                "value_type": "data",
                                "presentation_type": "image",
                                "value": "featured_played_character_image"
                            }}}
                        }
                    }
                }
            }]),
            1191168989724672018,
        )
        .unwrap();
        assert_eq!(
            config.hero,
            Some(WidgetImage::Data {
                key: "featured_played_character_image".into(),
                fallback: None,
            })
        );
    }

    #[test]
    fn widget_configs_carry_hero_fallbacks() {
        // Synthetic config with an application-asset fallback.
        let config = parse_application_widget_configs(
            json!([{
                "application_id": "1191169081999360040",
                "status": "published",
                "surfaces": {
                    "widget_top": {
                        "layout": "widget_top_hero",
                        "components": {
                            "hero_image": {"fields": {"image": {
                                "value_type": "data",
                                "presentation_type": "image",
                                "value": "featured_played_character_image",
                                "fallback": {
                                    "value_type": "application_asset",
                                    "presentation_type": "image",
                                    "value": "sample_hero"
                                }
                            }}}
                        }
                    }
                },
                "resolved_assets": [{
                    "key": "sample_hero",
                    "asset_id": "1191169203634176069",
                    "metadata": {"width": 2106, "height": 4681, "is_animated": false}
                }]
            }]),
            1191169081999360040,
        )
        .unwrap();
        assert_eq!(
            config.hero,
            Some(WidgetImage::Data {
                key: "featured_played_character_image".into(),
                fallback: Some(Box::new(WidgetImage::Asset(WidgetAsset {
                    asset_id: 1191169203634176069,
                    width: 2106,
                    height: 4681,
                    animated: false,
                }))),
            })
        );

        // A fallback that cannot be resolved is dropped, but the data hero survives.
        let config = parse_application_widget_configs(
            json!([{
                "application_id": "7",
                "status": "published",
                "surfaces": {
                    "widget_top": {"components": {
                        "hero_image": {"fields": {"image": {
                            "value_type": "data",
                            "presentation_type": "image",
                            "value": "featured_played_character_image",
                            "fallback": {
                                "value_type": "application_asset",
                                "presentation_type": "image",
                                "value": "missing"
                            }
                        }}}
                    }}
                }
            }]),
            7,
        )
        .unwrap();
        assert_eq!(
            config.hero,
            Some(WidgetImage::Data {
                key: "featured_played_character_image".into(),
                fallback: None,
            })
        );
    }

    #[test]
    fn application_widget_configs_carry_the_activity_accessory() {
        // Synthetic response for `/applications/1191168989724672018/widget-configs`.
        let config = parse_application_widget_configs(
            json!([{
                "application_id": "1191168989724672018",
                "status": "published",
                "surfaces": {
                    "activity_accessory": {
                        "layout": "activity_accessory_stat",
                        "components": {"stat": {"fields": {
                            "text": {
                                "value_type": "custom_string",
                                "presentation_type": "text",
                                "value": ""
                            },
                            "icon": {
                                "value_type": "application_asset",
                                "presentation_type": "image",
                                "value": "sample_accessory"
                            },
                            "label": {
                                "value_type": "custom_string",
                                "presentation_type": "text",
                                "value": "Current Mood"
                            }
                        }}}
                    }
                },
                "resolved_assets": [{
                    "key": "sample_accessory",
                    "asset_id": "1191169207828480070",
                    "metadata": {"width": 500, "height": 500, "is_animated": false}
                }]
            }]),
            1191168989724672018,
        )
        .unwrap();
        assert_eq!(
            config.accessory,
            Some(WidgetAccessory {
                layout: "activity_accessory_stat".into(),
                label: Some(WidgetText::Literal("Current Mood".into())),
                text: Some(WidgetText::Literal(String::new())),
                icon: Some(WidgetImage::Asset(WidgetAsset {
                    asset_id: 1191169207828480070,
                    width: 500,
                    height: 500,
                    animated: false,
                })),
            })
        );

        let config = parse_application_widget_configs(
            json!([{
                "application_id": "7",
                "status": "published",
                "surfaces": {"widget_top": {"components": {}}}
            }]),
            7,
        )
        .unwrap();
        assert!(config.accessory.is_none());
    }

    #[test]
    fn widget_mini_profiles_parse_hero_and_contained_layouts() {
        let configs = parse_featured_widget_configs(json!({
            "configs": {
                "300": [{
                    "application_id": "300",
                    "status": "published",
                    "surfaces": {
                        "mini_profile": {
                            "layout": "mini_profile_hero_stat",
                            "components": {
                                "stat": {"fields": {
                                    "text": {
                                        "value_type": "data",
                                        "presentation_type": "text",
                                        "value": "rank_name"
                                    },
                                    "icon": {
                                        "value_type": "data",
                                        "presentation_type": "image",
                                        "value": "rank_image"
                                    },
                                    "label": {
                                        "value_type": "data",
                                        "presentation_type": "text",
                                        "value": "season"
                                    }
                                }},
                                "hero_image": {"fields": {"image": {
                                    "value_type": "data",
                                    "presentation_type": "image",
                                    "value": "featured_play"
                                }}}
                            }
                        }
                    }
                }]
            }
        }));
        let mini = configs.get(&300).unwrap().mini.as_ref().unwrap();
        assert_eq!(mini.layout, "mini_profile_hero_stat");
        assert_eq!(mini.text, Some(WidgetText::Data("rank_name".into())));
        assert_eq!(mini.label, Some(WidgetText::Data("season".into())));
        assert_eq!(
            mini.icon,
            Some(WidgetImage::Data {
                key: "rank_image".into(),
                fallback: None,
            })
        );
        assert_eq!(
            mini.image,
            Some(WidgetImage::Data {
                key: "featured_play".into(),
                fallback: None,
            })
        );

        let configs = parse_featured_widget_configs(json!({
            "configs": {
                "301": [{
                    "application_id": "301",
                    "status": "published",
                    "surfaces": {
                        "mini_profile": {
                            "layout": "mini_profile_contained_stat",
                            "components": {
                                "stat": {"fields": {
                                    "text": {
                                        "value_type": "data",
                                        "presentation_type": "number",
                                        "value": "lev"
                                    },
                                    "icon": {
                                        "value_type": "application_asset",
                                        "presentation_type": "image",
                                        "value": "icon"
                                    }
                                }},
                                "contained_image": {"fields": {"image": {
                                    "value_type": "application_asset",
                                    "presentation_type": "image",
                                    "value": "art"
                                }}}
                            }
                        }
                    },
                    "resolved_assets": [
                        {
                            "key": "art",
                            "asset_id": "400",
                            "metadata": {"width": 64, "height": 64, "is_animated": false}
                        },
                        {"key": "icon", "asset_id": "401"}
                    ]
                }]
            }
        }));
        let mini = configs.get(&301).unwrap().mini.as_ref().unwrap();
        assert_eq!(mini.layout, "mini_profile_contained_stat");
        assert_eq!(mini.text, Some(WidgetText::Data("lev".into())));
        assert_eq!(
            mini.icon,
            Some(WidgetImage::Asset(WidgetAsset {
                asset_id: 401,
                width: 0,
                height: 0,
                animated: false
            }))
        );
        assert_eq!(
            mini.image,
            Some(WidgetImage::Asset(WidgetAsset {
                asset_id: 400,
                width: 64,
                height: 64,
                animated: false
            }))
        );

        let configs = parse_featured_widget_configs(json!({
            "configs": {
                "302": [{
                    "application_id": "302",
                    "status": "published",
                    "surfaces": {
                        "mini_profile": {
                            "layout": "mini_profile_contained_stat",
                            "components": {
                                "stat": {"fields": {"text": {
                                    "value_type": "custom_string",
                                    "value": "kept"
                                }}},
                                "contained_image": {"fields": {"image": {
                                    "value_type": "application_asset",
                                    "value": "missing"
                                }}}
                            }
                        }
                    }
                }]
            }
        }));
        let mini = configs.get(&302).unwrap().mini.as_ref().unwrap();
        assert_eq!(mini.text, Some(WidgetText::Literal("kept".into())));
        assert!(mini.image.is_none());

        let configs = parse_featured_widget_configs(json!({
            "configs": {
                "303": [{
                    "application_id": "303",
                    "status": "published",
                    "surfaces": {"widget_top": {"components": {}}}
                }]
            }
        }));
        assert!(configs.get(&303).unwrap().mini.is_none());
    }

    #[test]
    fn application_widget_configs_accept_known_shapes() {
        let config = json!({
            "application_id": "7",
            "status": "published",
            "surfaces": {
                "widget_top": {
                    "components": {
                        "title": {"fields": {"text": {
                            "value_type": "custom_string",
                            "value": "Title"
                        }}}
                    }
                }
            }
        });
        for response in [
            json!([config.clone()]),
            json!({"configs": [config.clone()]}),
            json!({"configs": {"7": [config.clone()]}}),
            config.clone(),
        ] {
            let parsed = parse_application_widget_configs(response, 7).unwrap();
            assert_eq!(parsed.application_id, 7);
            assert_eq!(parsed.title, Some(WidgetText::Literal("Title".into())));
        }
        assert!(parse_application_widget_configs(json!({"configs": []}), 7).is_none());
        assert!(parse_application_widget_configs(json!({}), 7).is_none());
        assert!(parse_application_widget_configs(json!(null), 7).is_none());
    }

    #[test]
    fn invite_links_extract_in_order_dedupe_and_cap() {
        let content = "join https://discord.gg/sampleInviteOne or https://www.discord.com/invite/sampleInviteTwo/ \
                       and (https://canary.discordapp.com/invite/abc_123?x=1) plus discord.gg/sampleInviteOne";
        assert_eq!(invite_codes(content), ["sampleInviteOne", "sampleInviteTwo", "abc_123"]);

        assert!(invite_codes(
            "https://notdiscord.gg/abc https://discord.gg.evil.com/abc https://evil.com/discord.gg/abc"
        )
        .is_empty());
        assert_eq!(
            invite_codes(
                "https://discord.gg/a1 https://discord.gg/a2 https://discord.gg/a3 https://discord.gg/a4"
            ),
            ["a1", "a2", "a3"]
        );
        assert_eq!(
            invite_codes("https://ptb.discord.com/invite/ptb1 https://discordapp.com/invite/dapp"),
            ["ptb1", "dapp"]
        );
    }

    #[test]
    fn invite_decodes_profile_counts_and_games() {
        // Trimmed from GET /invites/sampleInviteOne?with_counts=true&with_expiration=true.
        let invite: Invite = serde_json::from_value(json!({
            "code": "sampleInviteOne",
            "expires_at": null,
            "guild": {
                "id": "1191168926810112003",
                "name": "Sample Server Two",
                "splash": null,
                "banner": null,
                "icon": "f000000000000000000000000000000a"
            },
            "guild_id": "1191168926810112003",
            "profile": {
                "id": "1191168926810112003",
                "name": "Sample Server Two",
                "icon_hash": "f000000000000000000000000000000a",
                "member_count": 87,
                "online_count": 25,
                "banner_hash": null,
                "custom_banner_hash": null,
                "game_application_ids": ["1191169119748096049", "1191169031667712028"]
            },
            "approximate_member_count": 87,
            "approximate_presence_count": 25
        }))
        .unwrap();
        assert_eq!(invite.code, "sampleInviteOne");
        assert_eq!(invite.guild_id, Some(1191168926810112003));
        assert_eq!(invite.guild.as_ref().unwrap().name, "Sample Server Two");
        let profile = invite.profile.as_ref().unwrap();
        assert_eq!(profile.member_count, Some(87));
        assert_eq!(profile.online_count, Some(25));
        assert_eq!(
            profile.game_application_ids,
            [1191169119748096049, 1191169031667712028]
        );

        let bare: Invite = serde_json::from_value(json!({
            "code": "bare",
            "guild_id": "1",
            "profile": {"member_count": null, "game_application_ids": "nope"}
        }))
        .unwrap();
        let profile = bare.profile.unwrap();
        assert!(profile.member_count.is_none());
        assert!(profile.game_application_ids.is_empty());
    }

    #[test]
    #[ignore]
    fn har_captures_decode() {
        let har = |dir: &str, prefix: &str| {
            std::fs::read_dir(dir)
                .ok()?
                .flatten()
                .find(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
                .and_then(|entry| std::fs::read_to_string(entry.path()).ok())
        };
        if let Some(text) = har("/tmp/har", "08_") {
            let profile: Profile = serde_json::from_str(&text).unwrap();
            assert!(!profile.widgets.is_empty());
        }
        if let Some(text) = har("/tmp/har", "16_") {
            let games: Vec<Game> = serde_json::from_str(&text).unwrap();
            assert!(!games.is_empty());
        }
        for prefix in ["13_", "21_"] {
            if let Some(text) = har("/tmp/har", prefix) {
                let outbox: Outbox = serde_json::from_str(&text).unwrap();
                assert!(!outbox.entries.is_empty());
            }
        }
        if let Some(text) = har("/tmp/har", "09_") {
            let identities: ApplicationIdentities = serde_json::from_str(&text).unwrap();
            assert!(!identities.identities.is_empty());
        }
        if let Some(text) = har("/tmp/har", "17_") {
            let applications: Vec<PublicApplication> = serde_json::from_str(&text).unwrap();
            assert!(!applications.is_empty());
        }
        if let Some(text) = har("/tmp/har2", "017_") {
            let configs = parse_featured_widget_configs(serde_json::from_str(&text).unwrap());
            assert!(!configs.is_empty());
            assert!(configs.values().any(|config| {
                config.stats.len() == 6
                    && matches!(config.hero.as_ref(), Some(WidgetImage::Asset(asset))
                        if asset.asset_id != 0 && asset.width > 0 && asset.height > 0)
            }));
        }
        if let Some(text) = har("/tmp/har2", "018_") {
            let configs = parse_featured_widget_configs(serde_json::from_str(&text).unwrap());
            assert!(!configs.is_empty());
            assert!(configs.values().any(|config| {
                config.display_name.as_ref().is_some_and(|name| !name.is_empty())
                    && config.collection.len() == 4
                    && config.subtitles.iter().any(|subtitle| {
                        subtitle.label.is_some()
                            && subtitle.icon.as_ref().is_some_and(|icon| icon.animated)
                    })
            }));
        }
        if let Some(text) = har("/tmp/har2", "085_") {
            let games: Vec<Game> = serde_json::from_str(&text).unwrap();
            assert!(!games.is_empty());
            assert!(games.iter().any(|game| game.steam_app_id.is_some()));
            let raw: Vec<Value> = serde_json::from_str(&text).unwrap();
            assert_eq!(games.len(), raw.len());
            for (raw_game, game) in raw.iter().zip(&games) {
                let expected = raw_game
                    .get("third_party_skus")
                    .and_then(Value::as_array)
                    .and_then(|skus| {
                        skus.iter().find(|sku| {
                            sku.get("distributor").and_then(Value::as_str) == Some("steam")
                        })
                    })
                    .and_then(|sku| sku.get("id"))
                    .and_then(|id| {
                        id.as_str()
                            .map(str::to_owned)
                            .or_else(|| id.as_u64().map(|id| id.to_string()))
                    });
                assert_eq!(game.steam_app_id, expected);
            }
        }
    }
}
