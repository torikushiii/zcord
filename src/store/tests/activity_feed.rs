use super::*;

const NOW_MS: i64 = 1_800_000_000_000;
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

fn snowflake_at(timestamp_ms: u64) -> u64 {
    (timestamp_ms - DISCORD_EPOCH_MS) << 22
}

fn entry(
    id: u64,
    user_id: u64,
    content_type: u8,
    name: &str,
    traits: Vec<model::OutboxTrait>,
    started_at: Option<&str>,
    expires_at: Option<&str>,
) -> model::OutboxEntry {
    model::OutboxEntry {
        id,
        author_id: Some(user_id),
        content_type,
        started_at: started_at.map(str::to_owned),
        ended_at: None,
        expires_at: expires_at.map(str::to_owned),
        kind: model::OutboxKind::PlayedGame {
            name: name.to_owned(),
            application_id: Some(9000),
        },
        traits,
        participants: Vec::new(),
        raw: std::sync::Arc::new(json!({"id": id.to_string()})),
    }
}

fn inbox(entries: Vec<model::OutboxEntry>) -> model::ContentInbox {
    model::ContentInbox {
        entries,
        wait_ms_until_next_fetch: None,
    }
}

#[test]
fn activity_cards_keep_rank_order_cap_and_only_member_played_games() {
    let mut store = Store::from_ready(ready());
    let recent = snowflake_at(NOW_MS as u64 - 1_000);
    let mut entries = Vec::new();
    for (index, (user, name)) in [(1, "Own Game"), (2, "Non-Member"), (1, "Rank Two")]
        .into_iter()
        .enumerate()
    {
        entries.push(entry(
            recent + 3 + index as u64,
            user,
            1,
            name,
            Vec::new(),
            None,
            None,
        ));
    }
    entries.push(entry(recent + 4, 1, 2, "Listened", Vec::new(), None, None));
    for index in 0..15u64 {
        entries.push(entry(
            recent + 10 + index,
            1,
            1,
            "Extra",
            Vec::new(),
            None,
            None,
        ));
    }
    store.set_content_inbox(Some(inbox(entries)));

    let cards = store.activity_cards(200, NOW_MS);
    assert_eq!(cards.len(), 10, "at most ten cards");
    assert_eq!(cards[0].user_id, 1);
    assert_eq!(cards[0].game_name, "Own Game");
    assert_eq!(cards[0].entry_id, recent + 3, "rank order is preserved");
    assert!(cards.iter().all(|card| card.user_id == 1));
    assert!(cards.iter().all(|card| card.game_name != "Listened"));
    assert_eq!(store.activity_authors(), [1, 2]);
}

#[test]
fn activity_cards_keep_the_current_user_outside_the_member_cache() {
    let mut store = Store::from_ready(ready());
    store.apply(Event::GuildUpsert(Box::new(
        serde_json::from_value(json!({"id": "300", "name": "Empty"})).unwrap(),
    )));
    let id = snowflake_at(NOW_MS as u64 - 1_000);
    store.set_content_inbox(Some(inbox(vec![entry(
        id,
        1,
        1,
        "Solo",
        Vec::new(),
        None,
        None,
    )])));
    assert_eq!(store.activity_cards(300, NOW_MS).len(), 1);
    let unknown = store.activity_cards(300, NOW_MS);
    assert_eq!(unknown[0].user_id, 1);
    assert!(store.activity_cards(999, NOW_MS).is_empty());
}

#[test]
fn activity_cards_compute_live_streak_and_most_played() {
    let mut store = Store::from_ready(ready());
    let now = NOW_MS as u64;
    let fresh = snowflake_at(now - 1_000);
    let stale = snowflake_at(now - 49 * 60 * 60 * 1000);
    let entries = vec![
        entry(
            fresh,
            1,
            1,
            "Live",
            vec![
                model::OutboxTrait::Live,
                model::OutboxTrait::Streak { days: 3 },
            ],
            Some("2026-01-01T00:00:00Z"),
            None,
        ),
        entry(
            fresh + 1,
            1,
            1,
            "Lapsed",
            vec![model::OutboxTrait::Live],
            Some("2026-01-01T00:00:00Z"),
            Some("2026-01-01T00:00:01Z"),
        ),
        entry(
            fresh + 2,
            1,
            1,
            "Short Streak",
            vec![model::OutboxTrait::Streak { days: 2 }],
            None,
            None,
        ),
        entry(
            stale,
            1,
            1,
            "Old Streak",
            vec![model::OutboxTrait::Streak { days: 7 }],
            None,
            None,
        ),
        entry(
            fresh + 3,
            1,
            1,
            "Most Played",
            vec![
                model::OutboxTrait::AggregateRange { range: 1 },
                model::OutboxTrait::Duration { seconds: 52105 },
            ],
            None,
            None,
        ),
        entry(
            fresh + 4,
            1,
            1,
            "Barely Played",
            vec![
                model::OutboxTrait::AggregateRange { range: 1 },
                model::OutboxTrait::Duration { seconds: 1000 },
            ],
            None,
            None,
        ),
    ];
    store.set_content_inbox(Some(inbox(entries)));
    let cards = store.activity_cards(200, NOW_MS);
    let card = |name: &str| cards.iter().find(|card| card.game_name == name).unwrap();
    assert_eq!(
        card("Live").live_since_ms,
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .ok()
            .map(|t| t.timestamp_millis())
    );
    assert!(
        card("Lapsed").live_since_ms.is_none(),
        "expired live is hidden"
    );
    assert_eq!(card("Live").streak_days, Some(3));
    assert!(
        card("Short Streak").streak_days.is_none(),
        "streaks under 3 days"
    );
    assert!(
        card("Old Streak").streak_days.is_none(),
        "streaks over 48h old"
    );
    assert_eq!(card("Most Played").most_played_hours, Some(14));
    assert!(
        card("Barely Played").most_played_hours.is_none(),
        "sub-hour sessions are hidden"
    );
    assert_eq!(
        card("Live").created_ms,
        (fresh >> 22) as i64 + DISCORD_EPOCH_MS as i64
    );
}

#[test]
fn content_inbox_clear_and_authors_ignore_other_content_types() {
    let mut store = Store::from_ready(ready());
    assert!(store.content_inbox().is_none());
    assert!(store.activity_authors().is_empty());
    let id = snowflake_at(NOW_MS as u64 - 1_000);
    store.set_content_inbox(Some(inbox(vec![
        entry(id, 1, 1, "Game", Vec::new(), None, None),
        entry(id + 1, 2, 2, "Media", Vec::new(), None, None),
        entry(id + 2, 1, 1, "Game Again", Vec::new(), None, None),
    ])));
    assert_eq!(store.content_inbox().unwrap().entries.len(), 3);
    assert_eq!(store.activity_authors(), [1]);
    store.set_content_inbox(None);
    assert!(store.content_inbox().is_none());
    assert!(store.activity_authors().is_empty());
}

#[test]
fn activity_components_map_and_drop_the_flagged_legacy_embed() {
    let message: model::Message = serde_json::from_value(json!({
        "id": "100", "channel_id": "200", "type": 0,
        "author": {"id": "1", "username": "me"},
        "content": "played this",
        "timestamp": "2026-01-01T00:00:00Z",
        "components": [{
            "type": 16, "id": 1,
            "content_inventory_entry": {
                "id": "1191169321074688097",
                "author_id": "1191169170079744061",
                "author_type": 0, "content_type": 1,
                "traits": [
                    {"type": 3, "is_live": true},
                    {"type": 1, "first_time": true},
                    {"type": 9, "trending": true},
                    {"type": 4, "range": 1},
                    {"type": 2, "duration_seconds": 7200}
                ],
                "extra": {"type": "played_game_extra", "game_name": "Synthetic",
                          "application_id": "9001"},
                "started_at": "2026-01-01T00:00:00Z"
            }
        }],
        "embeds": [
            {"type": "rich", "flags": 32, "title": "flagged"},
            {"type": "rich", "title": "kept"}
        ]
    }))
    .unwrap();
    let mapped = mapping::map_message(&message).unwrap();
    let Component::ActivityEntry(entry) = &mapped.components[0] else {
        panic!("type-16 component must map to an activity entry");
    };
    assert_eq!(entry.entry_id, 1191169321074688097);
    assert_eq!(entry.user_id, 1191169170079744061);
    assert_eq!(entry.game_name, "Synthetic");
    assert_eq!(entry.application_id, Some(9001));
    assert!(entry.first_time);
    assert!(entry.trending);
    assert_eq!(entry.most_played_hours, Some(2));
    assert!(entry.live_since_ms.is_some());
    assert_eq!(
        mapped.embeds.len(),
        1,
        "the flags-32 legacy embed is dropped"
    );
    assert_eq!(
        mapped.embeds[0]
            .title
            .as_ref()
            .map(|title| title.text.as_str()),
        Some("kept")
    );
}
