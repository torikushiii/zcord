use super::experiments::{game_servers, server_theme};
use super::view::BoostGoal;
use crate::discord::{self, model};
use chrono::{DateTime, Duration, Utc};

const LEVEL_3_BOOSTS: u32 = 14;
const PERK_BOOSTS: [(&str, u32); 6] = [
    ("GUILD_TAGS", 3),
    ("ENHANCED_ROLE_COLORS", 3),
    ("GUILD_TAGS_BADGE_PACK_PETS", 3),
    ("GUILD_TAGS_BADGE_PACK_FLEX", 5),
    ("GUILD_TAGS_BADGE_PACK_PLANT", 3),
    ("GUILD_TAGS_BADGE_PACK_CREEPY_CRAWLIES", 2),
];
const SERVER_THEME_BOOSTS: u32 = 3;
const GAME_SERVER_BOOSTS: u32 = 3;

const COMPLETED_HOME_ACTIONS: u32 = 1 << 6;
const IS_GUILD_RESOURCE_CHANNEL: u64 = 1 << 7;
const NEW_MEMBER_DAYS: i64 = 7;

const VOICE: u8 = 2;
const STAGE: u8 = 13;

pub(super) fn boost_goal(
    guild: &model::Guild,
    experiments: &model::Experiments,
) -> Option<BoostGoal> {
    if !guild.premium_progress_bar_enabled() {
        return None;
    }
    let levels = if guild.has_feature("PREMIUM_TIER_3_OVERRIDE") {
        0
    } else {
        LEVEL_3_BOOSTS
    };
    let perks: u32 = PERK_BOOSTS.iter().map(|(_, boosts)| boosts).sum();
    let gated = u32::from(server_theme(experiments, guild)) * SERVER_THEME_BOOSTS
        + u32::from(game_servers(experiments, guild)) * GAME_SERVER_BOOSTS;
    Some(BoostGoal {
        boosts: guild.premium_subscription_count.unwrap_or(0),
        goal: levels + perks + gated,
    })
}

pub(super) fn server_guide(guild: &model::Guild, user_id: u64) -> bool {
    ["COMMUNITY", "GUILD_SERVER_GUIDE", "GUILD_ONBOARDING"]
        .iter()
        .all(|feature| guild.has_feature(feature))
        && !guild.has_feature("HUB")
        && (new_member(guild, user_id)
            || guild
                .channels
                .iter()
                .flatten()
                .any(|channel| channel.flags.unwrap_or(0) & IS_GUILD_RESOURCE_CHANNEL != 0))
}

fn new_member(guild: &model::Guild, user_id: u64) -> bool {
    let Some(member) = guild
        .members
        .iter()
        .flatten()
        .find(|member| member.id() == Some(user_id))
    else {
        return false;
    };
    member.flags & COMPLETED_HOME_ACTIONS == 0
        && member
            .joined_at
            .as_deref()
            .or(guild.joined_at.as_deref())
            .and_then(|joined| DateTime::parse_from_rfc3339(joined).ok())
            .is_some_and(|joined| {
                Utc::now().signed_duration_since(joined) < Duration::days(NEW_MEMBER_DAYS)
            })
}

pub(super) fn events(guild: &model::Guild, user_id: u64) -> Option<u32> {
    if guild.has_feature("HUB") {
        return None;
    }
    let channels = guild.channels.as_deref().unwrap_or_default();
    let visible = |id: u64| {
        channels
            .iter()
            .find(|channel| channel.id == id)
            .is_some_and(|channel| discord::can_view(guild, channel, user_id))
    };
    let upcoming = guild
        .guild_scheduled_events
        .iter()
        .flatten()
        .filter(|event| event.upcoming() && event.channel_id.is_none_or(visible))
        .count();
    let upcoming = u32::try_from(upcoming).unwrap_or(u32::MAX);
    (upcoming > 0 || can_create_events(guild, channels, user_id)).then_some(upcoming)
}

fn can_create_events(guild: &model::Guild, channels: &[model::Channel], user_id: u64) -> bool {
    discord::guild_permissions(guild, user_id) & discord::CREATE_EVENTS != 0
        || channels.iter().any(|channel| {
            let required = match channel.kind {
                Some(VOICE) => discord::VIEW_CHANNEL | discord::CONNECT,
                Some(STAGE) => {
                    discord::MANAGE_CHANNELS
                        | discord::VIEW_CHANNEL
                        | discord::MUTE_MEMBERS
                        | discord::MOVE_MEMBERS
                }
                _ => return false,
            } | discord::CREATE_EVENTS;
            discord::channel_permissions(guild, channel, user_id) & required == required
        })
}

pub(super) fn channels_and_roles(guild: &model::Guild) -> bool {
    guild.has_feature("COMMUNITY")
        && !guild.has_feature("HUB")
        && guild.has_feature("GUILD_ONBOARDING_HAS_PROMPTS")
}
