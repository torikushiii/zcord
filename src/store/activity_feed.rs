use super::*;

const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
const STREAK_WINDOW_MS: i64 = 48 * 60 * 60 * 1000;
const MIN_STREAK_DAYS: u32 = 3;
const CARD_LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityCard {
    pub entry_id: u64,
    pub user_id: u64,
    pub game_name: String,
    pub application_id: Option<u64>,
    pub live_since_ms: Option<i64>,
    pub streak_days: Option<u32>,
    pub most_played_hours: Option<u32>,
    pub created_ms: i64,
    pub first_time: bool,
    pub trending: bool,
}

fn parse_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp_millis())
}

pub(super) fn entry_view(entry: &model::OutboxEntry, now_ms: i64) -> Option<ActivityEntryView> {
    let model::OutboxKind::PlayedGame {
        name,
        application_id,
    } = &entry.kind
    else {
        return None;
    };
    let user_id = entry.author_id?;
    let created_ms = (entry.id >> 22).checked_add(DISCORD_EPOCH_MS).unwrap_or(0) as i64;
    let live = entry
        .traits
        .iter()
        .any(|trait_| matches!(trait_, model::OutboxTrait::Live))
        && entry
            .expires_at
            .as_deref()
            .is_none_or(|expires| parse_timestamp(expires).is_none_or(|t| t > now_ms));
    let live_since_ms = live
        .then(|| entry.started_at.as_deref().and_then(parse_timestamp))
        .flatten();
    let streak_days = entry
        .traits
        .iter()
        .find_map(|trait_| match trait_ {
            model::OutboxTrait::Streak { days } => Some(*days),
            _ => None,
        })
        .filter(|days| *days >= MIN_STREAK_DAYS && now_ms - created_ms < STREAK_WINDOW_MS);
    let most_played_hours = entry
        .traits
        .iter()
        .any(|trait_| matches!(trait_, model::OutboxTrait::AggregateRange { .. }))
        .then(|| {
            entry.traits.iter().find_map(|trait_| match trait_ {
                model::OutboxTrait::Duration { seconds } => Some(*seconds),
                _ => None,
            })
        })
        .flatten()
        .map(|seconds| (seconds as f64 / 3600.0).round() as u32)
        .filter(|hours| *hours > 0);
    Some(ActivityEntryView {
        entry_id: entry.id,
        user_id,
        game_name: name.clone(),
        application_id: *application_id,
        created_ms,
        live_since_ms,
        ended_ms: entry.ended_at.as_deref().and_then(parse_timestamp),
        streak_days,
        most_played_hours,
        first_time: entry
            .traits
            .iter()
            .any(|trait_| matches!(trait_, model::OutboxTrait::FirstTime)),
        trending: entry
            .traits
            .iter()
            .any(|trait_| matches!(trait_, model::OutboxTrait::Trending)),
    })
}

impl Store {
    pub fn set_content_inbox(&mut self, inbox: Option<model::ContentInbox>) {
        self.content_inbox = inbox;
    }

    pub fn content_inbox(&self) -> Option<&model::ContentInbox> {
        self.content_inbox.as_ref()
    }

    pub fn activity_authors(&self) -> Vec<u64> {
        let mut authors = Vec::new();
        for entry in self.content_inbox.iter().flat_map(|inbox| &inbox.entries) {
            if entry.content_type == 1
                && let Some(author_id) = entry.author_id
                && !authors.contains(&author_id)
            {
                authors.push(author_id);
            }
        }
        authors
    }

    pub fn activity_cards(&self, guild_id: u64, now_ms: i64) -> Vec<ActivityCard> {
        let Some(guild) = self.guild_by_id(guild_id) else {
            return Vec::new();
        };
        let mut cards = Vec::new();
        for entry in self.content_inbox.iter().flat_map(|inbox| &inbox.entries) {
            if cards.len() >= CARD_LIMIT {
                break;
            }
            if entry.content_type != 1 {
                continue;
            }
            let Some(user_id) = entry.author_id else {
                continue;
            };
            let member = guild.member_ids.contains(&user_id)
                || self
                    .social
                    .guilds
                    .get(&guild_id)
                    .is_some_and(|social| social.members.contains_key(&user_id));
            if !member && user_id != self.current_user_id {
                continue;
            }
            let Some(view) = entry_view(entry, now_ms) else {
                continue;
            };
            cards.push(ActivityCard {
                entry_id: view.entry_id,
                user_id,
                game_name: view.game_name,
                application_id: view.application_id,
                live_since_ms: view.live_since_ms,
                streak_days: view.streak_days,
                most_played_hours: view.most_played_hours,
                created_ms: view.created_ms,
                first_time: view.first_time,
                trending: view.trending,
            });
        }
        cards
    }
}
