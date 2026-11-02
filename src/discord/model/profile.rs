use super::serde_helpers::*;
use super::*;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::HashMap;

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
    #[serde(
        default,
        rename = "wishlist_settings",
        deserialize_with = "wishlist_ids"
    )]
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
        self.badges.iter().find(|detail| {
            !detail.name.is_empty() && detail.name.eq_ignore_ascii_case(&badge.description)
        })
    }
}

pub(super) fn badge_id_for(id: &str) -> Option<u32> {
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
    Personal {
        #[serde(default, deserialize_with = "empty_string_none")]
        header: Option<String>,
        #[serde(default, deserialize_with = "entries")]
        sections: Vec<PersonalSection>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PersonalSection {
    Cover {
        #[serde(default, deserialize_with = "empty_string_none")]
        title: Option<String>,
        #[serde(default, deserialize_with = "empty_string_none")]
        subtitle: Option<String>,
        #[serde(default, deserialize_with = "default_value")]
        image: Option<PersonalImage>,
    },
    Fields {
        #[serde(default, deserialize_with = "personal_fields")]
        fields: Vec<PersonalField>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PersonalField {
    #[serde(default, deserialize_with = "empty_string_none")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "empty_string_none")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub image: Option<PersonalImage>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PersonalImage {
    #[serde(deserialize_with = "snowflake")]
    pub file_id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub width: Option<u32>,
    #[serde(default, deserialize_with = "default_value")]
    pub height: Option<u32>,
    #[serde(default)]
    pub is_animated: bool,
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

fn empty_string_none<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned))
}

fn personal_fields<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<PersonalField>, D::Error> {
    let value = Value::deserialize(d)?;
    let Value::Array(entries) = value else {
        return Err(serde::de::Error::custom("personal fields must be an array"));
    };
    Ok(parse_entries(Value::Array(entries)))
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
    #[serde(default, deserialize_with = "entries")]
    pub linked_applications: Vec<LinkedApplication>,
    #[serde(default, deserialize_with = "entries")]
    pub websites: Vec<GameWebsite>,
    #[serde(default, deserialize_with = "default_value")]
    pub media: GameMedia,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LinkedApplication {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GameWebsite {
    pub url: String,
    /// IGDB website category: 1 is the official game site.
    #[serde(default, deserialize_with = "default_value")]
    pub category: u64,
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
    #[serde(default, deserialize_with = "default_value")]
    pub deeplink_uri: Option<String>,
    #[serde(
        rename = "third_party_skus",
        default,
        deserialize_with = "steam_app_id"
    )]
    pub steam_app_id: Option<String>,
    #[serde(default, deserialize_with = "entries")]
    pub linked_games: Vec<LinkedGame>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LinkedGame {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub application: Option<Game>,
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
    pub collectible: Option<WishlistCollectible>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WishlistCollectible {
    #[serde(default, rename = "type")]
    pub kind: u8,
    #[serde(default, deserialize_with = "default_value")]
    pub asset: Option<String>,
    #[serde(default, deserialize_with = "default_value")]
    pub label: Option<String>,
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
            collectibles_item: Option<WishlistCollectible>,
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
            collectible: raw.collectibles_item,
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
    /// An image object (`primary` values or `dynamic` type 3); `url` is its `proxy_url` (or `url`).
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
            _ => match &raw.value {
                Value::String(text) => StatValue::Text(text.clone()),
                Value::Object(_) => primary_stat_value(&raw.value)
                    .unwrap_or_else(|| StatValue::Text(raw.value.to_string())),
                other => StatValue::Text(other.to_string()),
            },
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
    pub author_id: Option<u64>,
    /// Content inventory type: 1 PLAYED_GAME, 2 WATCHED_MEDIA, 3 TOP_GAME, ...
    pub content_type: u8,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    /// Live entries ("listening now") stay flagged live after they lapse; this says when.
    pub expires_at: Option<String>,
    pub kind: OutboxKind,
    pub traits: Vec<OutboxTrait>,
    pub participants: Vec<u64>,
    /// The entry exactly as received; replies must echo it byte-for-byte.
    pub raw: std::sync::Arc<Value>,
}

impl<'de> Deserialize<'de> for OutboxEntry {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(deserialize_with = "snowflake")]
            id: u64,
            #[serde(default, deserialize_with = "optional_id")]
            author_id: Option<u64>,
            #[serde(default, deserialize_with = "default_value")]
            content_type: u8,
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
        let value = Value::deserialize(d)?;
        let raw = Raw::deserialize(&value).map_err(serde::de::Error::custom)?;
        Ok(Self {
            id: raw.id,
            author_id: raw.author_id,
            content_type: raw.content_type,
            started_at: raw.started_at,
            ended_at: raw.ended_at,
            expires_at: raw.expires_at,
            kind: outbox_kind(&raw.extra),
            traits: outbox_traits(&raw.traits),
            participants: value
                .get("participants")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(id_value)
                .collect(),
            raw: std::sync::Arc::new(value),
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
    AggregateRange { range: u8 },
    Resurrected { last_played: String },
    Marathon,
    NewRelease,
    Streak { days: u32 },
    Trending,
    TopItemCount { count: u32 },
    TopParentCount { count: u32 },
    AggregateCount { count: u32 },
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
                4 => trait_value
                    .get("range")
                    .and_then(Value::as_u64)
                    .and_then(|range| u8::try_from(range).ok())
                    .map(|range| OutboxTrait::AggregateRange { range })
                    .unwrap_or(OutboxTrait::Unknown),
                5 => trait_value
                    .get("resurrected_last_played")
                    .and_then(Value::as_str)
                    .map(|last_played| OutboxTrait::Resurrected {
                        last_played: last_played.to_owned(),
                    })
                    .unwrap_or(OutboxTrait::Unknown),
                6 if trait_value.get("marathon").and_then(Value::as_bool) == Some(true) => {
                    OutboxTrait::Marathon
                }
                7 => OutboxTrait::NewRelease,
                8 => trait_value
                    .get("streak_count_days")
                    .and_then(Value::as_u64)
                    .and_then(|days| u32::try_from(days).ok())
                    .map(|days| OutboxTrait::Streak { days })
                    .unwrap_or(OutboxTrait::Unknown),
                9 => OutboxTrait::Trending,
                10 => trait_value
                    .get("count")
                    .and_then(Value::as_u64)
                    .and_then(|count| u32::try_from(count).ok())
                    .map(|count| OutboxTrait::TopItemCount { count })
                    .unwrap_or(OutboxTrait::Unknown),
                11 => trait_value
                    .get("count")
                    .and_then(Value::as_u64)
                    .and_then(|count| u32::try_from(count).ok())
                    .map(|count| OutboxTrait::TopParentCount { count })
                    .unwrap_or(OutboxTrait::Unknown),
                12 => trait_value
                    .get("count")
                    .and_then(Value::as_u64)
                    .and_then(|count| u32::try_from(count).ok())
                    .map(|count| OutboxTrait::AggregateCount { count })
                    .unwrap_or(OutboxTrait::Unknown),
                _ => OutboxTrait::Unknown,
            })
        })
        .collect()
}

/// The content inventory inbox; `entries` keeps only the ranked list, in rank order.
#[derive(Clone, Debug)]
pub struct ContentInbox {
    pub entries: Vec<OutboxEntry>,
    pub wait_ms_until_next_fetch: Option<u64>,
}

impl<'de> Deserialize<'de> for ContentInbox {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Ranked {
            rank: i64,
            content: OutboxEntry,
        }
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default, deserialize_with = "entries")]
            entries: Vec<Ranked>,
            #[serde(default, deserialize_with = "default_value")]
            wait_ms_until_next_fetch: Option<u64>,
        }
        let raw = Raw::deserialize(d)?;
        let mut entries = raw.entries;
        entries.sort_by_key(|entry| entry.rank);
        Ok(Self {
            entries: entries.into_iter().map(|entry| entry.content).collect(),
            wait_ms_until_next_fetch: raw.wait_ms_until_next_fetch,
        })
    }
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
    /// `widget_top.components.contained_image.fields.image`.
    pub contained: Option<WidgetImage>,
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
    pub image: Option<WidgetImage>,
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
    let contained = top
        .and_then(|top| top.get("components"))
        .and_then(|components| components.get("contained_image"))
        .and_then(|contained| contained.get("fields"))
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
                        .and_then(|image| widget_image(image, &assets));
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
        contained,
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
