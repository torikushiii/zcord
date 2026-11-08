mod games;
mod outbox;
mod widget_config;
mod widgets;

pub use games::*;
pub use outbox::*;
pub use widget_config::*;
pub use widgets::*;

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
    #[serde(default, deserialize_with = "default_value")]
    pub application: Option<ProfileApplication>,
}

#[derive(Clone, Default, Deserialize)]
pub struct ProfileApplication {
    #[serde(deserialize_with = "snowflake")]
    pub id: u64,
    #[serde(default, deserialize_with = "default_value")]
    pub flags: u64,
    #[serde(default, deserialize_with = "ids")]
    pub popular_application_command_ids: Vec<u64>,
    #[serde(default)]
    install_params: Option<Value>,
    #[serde(default)]
    custom_install_url: Option<String>,
}

/// https://docs.discord.food/resources/application#application-flags
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataAccess {
    Messages,
    Presence,
    Members,
}

impl ProfileApplication {
    pub fn installable(&self) -> bool {
        self.install_params.is_some() || self.custom_install_url.is_some()
    }

    pub fn data_access(&self) -> Vec<DataAccess> {
        const PRESENCE: u64 = 1 << 12 | 1 << 13;
        const MEMBERS: u64 = 1 << 14 | 1 << 15;
        const MESSAGE_CONTENT: u64 = 1 << 18 | 1 << 19;
        [
            (MESSAGE_CONTENT, DataAccess::Messages),
            (PRESENCE, DataAccess::Presence),
            (MEMBERS, DataAccess::Members),
        ]
        .into_iter()
        .filter(|(bits, _)| self.flags & bits != 0)
        .map(|(_, access)| access)
        .collect()
    }
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

#[derive(Clone, Default, Deserialize)]
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

#[derive(Clone, Default, Deserialize)]
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
    pub kind: u16,
    #[serde(default, deserialize_with = "default_value")]
    pub premium_type: Option<u8>,
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
    pub title: Option<String>,
    #[serde(
        default,
        rename = "thumbnailPreviewSrc",
        deserialize_with = "default_value"
    )]
    pub thumbnail_preview_src: Option<String>,
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
