use super::*;

fn upsert(store: &mut Store, payload: serde_json::Value) {
    store.apply(Event::GuildUpsert(Box::new(
        serde_json::from_value(payload).unwrap(),
    )));
}

#[test]
fn emoji_origin_uses_exact_ids_across_joined_guilds() {
    let mut ready = ready();
    for guild in &mut ready.guilds {
        guild.emojis = Some(
            serde_json::from_value(json!([{
                "id": (guild.id + 9000).to_string(), "name": "same_name",
                "animated": true, "available": false, "roles": ["7"]
            }]))
            .unwrap(),
        );
    }
    let store = Store::from_ready(ready);
    for guild_id in [100, 200] {
        let (guild, emoji) = store.emoji_origin(guild_id + 9000).unwrap();
        assert_eq!(guild.id, guild_id);
        assert_eq!(emoji.guild_id, guild_id);
        assert_eq!(emoji.id, guild_id + 9000);
        assert_eq!(emoji.name, "same_name");
        assert!(emoji.animated && !emoji.available);
        assert_eq!(emoji.roles, [7]);
    }
    assert!(store.emoji_origin(99999).is_none());
    assert!(Store::seed().emoji_origin(99999).is_none());
}

#[test]
fn emoji_origin_reflects_updates_removals_and_guild_deletion() {
    let mut store = Store::from_ready(ready());
    upsert(
        &mut store,
        json!({"id": "200", "emojis": [{"id": "9001", "name": "old"}]}),
    );
    assert_eq!(store.emoji_origin(9001).unwrap().1.name, "old");

    store.apply(Event::GuildEmojisUpdate(
        serde_json::from_value(json!({
            "guild_id": "200", "emojis": [{"id": "9001", "name": "renamed"}]
        }))
        .unwrap(),
    ));
    assert_eq!(store.emoji_origin(9001).unwrap().1.name, "renamed");

    store.apply(Event::GuildEmojisUpdate(
        serde_json::from_value(json!({
            "guild_id": "200", "emojis": [{"id": "9002", "name": "replacement"}]
        }))
        .unwrap(),
    ));
    assert!(store.emoji_origin(9001).is_none());
    assert_eq!(store.emoji_origin(9002).unwrap().0.id, 200);

    upsert(&mut store, json!({"id": "200", "emojis": []}));
    assert!(store.emoji_origin(9002).is_none());
    upsert(
        &mut store,
        json!({"id": "100", "emojis": [{"id": "9003", "name": "departed"}]}),
    );
    assert!(store.emoji_origin(9003).is_some());
    store.apply(Event::GuildDelete { id: 100 });
    assert!(store.emoji_origin(9003).is_none());
}

#[test]
fn sticker_origin_uses_exact_ready_catalog_ids() {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../../discord/fixtures/ready.json")).unwrap();
    for guild in payload["guilds"].as_array_mut().unwrap() {
        let Some(guild_id) = model::id_value(&guild["id"]) else {
            continue;
        };
        guild["stickers"] = json!([{
            "id": (guild_id + 9000).to_string(), "name": "same_name",
            "format_type": 1, "type": 2, "available": false
        }]);
    }
    let store = Store::from_ready(serde_json::from_value(payload).unwrap());
    assert_eq!(store.sticker_origin(9100).unwrap().id, 100);
    assert_eq!(store.sticker_origin(9200).unwrap().id, 200);
    assert!(store.sticker_origin(99999).is_none());
    assert!(Store::from_ready(ready()).sticker_origin(9100).is_none());
    assert!(Store::seed().sticker_origin(9100).is_none());
}

#[test]
fn sticker_catalog_updates_preserve_omitted_and_null_but_replace_arrays() {
    let mut store = Store::from_ready(ready());
    upsert(
        &mut store,
        json!({
            "id": "200", "stickers": [
                {"id": "9001", "name": "old", "format_type": 1},
                {"name": "missing-id"}, {"id": null}, {"id": "bad"}
            ]
        }),
    );
    assert_eq!(store.sticker_origin(9001).unwrap().id, 200);
    assert_eq!(
        store.source.as_ref().unwrap().guilds[0]
            .stickers
            .as_ref()
            .unwrap()
            .len(),
        1
    );

    upsert(&mut store, json!({"id": "200", "name": "Renamed"}));
    assert_eq!(store.sticker_origin(9001).unwrap().name, "Renamed");
    upsert(&mut store, json!({"id": "200", "stickers": null}));
    assert!(store.sticker_origin(9001).is_some());

    upsert(
        &mut store,
        json!({
            "id": "200", "stickers": [{"id": "9002", "name": "new", "format_type": 4}]
        }),
    );
    assert!(store.sticker_origin(9001).is_none());
    assert_eq!(store.sticker_origin(9002).unwrap().id, 200);
    upsert(&mut store, json!({"id": "200", "stickers": []}));
    assert!(store.sticker_origin(9002).is_none());

    upsert(
        &mut store,
        json!({"id": "300", "name": "New guild", "stickers": [{"id": "9003"}]}),
    );
    assert_eq!(store.sticker_origin(9003).unwrap().id, 300);
    store.apply(Event::GuildDelete { id: 300 });
    assert!(store.sticker_origin(9003).is_none());
}

#[test]
fn guild_stickers_update_replaces_only_target_catalog_and_clears_on_empty_array() {
    let mut store = Store::from_ready(ready());
    upsert(
        &mut store,
        json!({
            "id": "200", "name": "Target", "icon": "icon", "banner": "banner",
            "features": ["DISCOVERABLE"], "emojis": [{"id": "8001", "name": "emoji"}],
            "stickers": [{"id": "9001", "name": "old"}]
        }),
    );
    upsert(
        &mut store,
        json!({"id": "100", "stickers": [{"id": "9100", "name": "unrelated"}]}),
    );
    let before = store.source.as_ref().unwrap().guilds[0].clone();
    let guild_count = store.guilds.len();
    let source_guild_count = store.source.as_ref().unwrap().guilds.len();

    let update = serde_json::from_value(json!({
        "guild_id": "200", "stickers": [{"id": "9002", "name": "new", "format_type": 4}]
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::GuildStickersUpdate(update)),
        Change::Structure
    );
    assert!(store.sticker_origin(9001).is_none());
    assert_eq!(store.sticker_origin(9002).unwrap().id, 200);
    assert_eq!(store.sticker_origin(9100).unwrap().id, 100);
    let source = &store.source.as_ref().unwrap().guilds[0];
    let stickers = source.stickers.as_ref().unwrap();
    assert_eq!(stickers.len(), 1);
    assert_eq!(stickers[0].name, "new");
    assert_eq!(stickers[0].format_type, 4);

    let update = serde_json::from_value(json!({"guild_id": "200", "stickers": []})).unwrap();
    assert_eq!(
        store.apply(Event::GuildStickersUpdate(update)),
        Change::Structure
    );
    assert!(store.sticker_origin(9002).is_none());
    assert_eq!(store.sticker_origin(9100).unwrap().id, 100);
    assert_eq!(store.emoji_origin(8001).unwrap().0.id, 200);
    assert_eq!(store.guild_discoverable(200), Some(true));
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_some());
    assert_eq!(store.guilds.len(), guild_count);
    assert_eq!(
        store.source.as_ref().unwrap().guilds.len(),
        source_guild_count
    );
    let source = &store.source.as_ref().unwrap().guilds[0];
    assert!(source.stickers.as_ref().unwrap().is_empty());
    assert_eq!(source.name(), before.name());
    assert_eq!(source.owner_id(), before.owner_id());
    assert_eq!(source.icon(), before.icon());
    assert_eq!(source.banner(), before.banner());
    assert_eq!(source.features(), before.features());
    assert_eq!(
        source.channels.as_ref().map(Vec::len),
        before.channels.as_ref().map(Vec::len)
    );
    assert_eq!(
        source.roles.as_ref().map(Vec::len),
        before.roles.as_ref().map(Vec::len)
    );
    assert_eq!(
        source.members.as_ref().map(Vec::len),
        before.members.as_ref().map(Vec::len)
    );
}

#[test]
fn guild_stickers_update_ignores_unknown_guilds_and_missing_or_null_catalogs() {
    let mut store = Store::from_ready(ready());
    upsert(
        &mut store,
        json!({"id": "200", "stickers": [{"id": "9001", "name": "retained"}]}),
    );
    let guild_count = store.guilds.len();
    let source_guild_count = store.source.as_ref().unwrap().guilds.len();
    for payload in [
        json!({"guild_id": "200"}),
        json!({"guild_id": "200", "stickers": null}),
        json!({"guild_id": "300", "stickers": [{"id": "9001"}, {"id": "9002"}]}),
        json!({"guild_id": "300", "stickers": []}),
    ] {
        let update = serde_json::from_value(payload).unwrap();
        assert_eq!(
            store.apply(Event::GuildStickersUpdate(update)),
            Change::None
        );
        assert_eq!(store.sticker_origin(9001).unwrap().id, 200);
        assert!(store.sticker_origin(9002).is_none());
        assert!(store.guild_by_id(300).is_none());
        assert_eq!(store.guilds.len(), guild_count);
        assert_eq!(
            store.source.as_ref().unwrap().guilds.len(),
            source_guild_count
        );
    }
}

#[test]
fn guild_stickers_reads_only_the_exact_joined_source_catalog() {
    let mut store = Store::from_ready(ready());
    assert!(store.guild_stickers(200).is_none());
    assert!(store.guild_stickers(100).is_none());
    assert!(store.guild_stickers(300).is_none());
    assert!(Store::seed().guild_stickers(200).is_none());
    upsert(
        &mut store,
        json!({"id": "200", "stickers": [
            {"id": "9001", "name": "same_name", "format_type": 2},
            {"id": "9002", "name": "second", "format_type": 4}
        ]}),
    );
    upsert(
        &mut store,
        json!({"id": "100", "stickers": [
            {"id": "9100", "name": "same_name", "format_type": 1}
        ]}),
    );
    let stickers = store.guild_stickers(200).unwrap();
    assert_eq!(stickers.len(), 2);
    assert_eq!(stickers[0].id, 9001);
    assert_eq!(stickers[0].name, "same_name");
    assert_eq!(stickers[0].format_type, 2);
    assert_eq!(stickers[1].id, 9002);
    assert_eq!(stickers[1].format_type, 4);
    let stickers = store.guild_stickers(100).unwrap();
    assert_eq!(stickers.len(), 1);
    assert_eq!(stickers[0].id, 9100);
    assert_eq!(stickers[0].format_type, 1);
    assert!(store.guild_stickers(300).is_none());
}

#[test]
fn guild_stickers_tracks_replacements_and_known_empty_catalogs() {
    let mut store = Store::from_ready(ready());
    for (catalog, expected_ids) in [
        (json!([{"id": "9001", "name": "old"}]), vec![9001]),
        (json!([{"id": "9002", "name": "new"}]), vec![9002]),
        (json!([]), vec![]),
    ] {
        let update = serde_json::from_value(json!({
            "guild_id": "200", "stickers": catalog
        }))
        .unwrap();
        store.apply(Event::GuildStickersUpdate(update));
        let ids: Vec<_> = store
            .guild_stickers(200)
            .unwrap()
            .iter()
            .map(|sticker| sticker.id)
            .collect();
        assert_eq!(ids, expected_ids);
        assert!(store.guild_stickers(100).is_none());
    }
    upsert(&mut store, json!({"id": "200", "stickers": null}));
    assert!(store.guild_stickers(200).unwrap().is_empty());
    store.apply(Event::GuildDelete { id: 200 });
    assert!(store.guild_stickers(200).is_none());
}

#[test]
fn guild_discoverable_distinguishes_known_features_from_unknown() {
    let mut store = Store::from_ready(ready());
    assert_eq!(store.guild_discoverable(99999), None);
    assert_eq!(store.guild_discoverable(100), None);
    assert_eq!(Store::seed().guild_discoverable(100), None);

    upsert(&mut store, json!({"id": "100", "features": null}));
    assert_eq!(store.guild_discoverable(100), None);
    upsert(&mut store, json!({"id": "100", "features": []}));
    assert_eq!(store.guild_discoverable(100), Some(false));
    upsert(
        &mut store,
        json!({"id": "100", "features": ["COMMUNITY", "VANITY_URL"]}),
    );
    assert_eq!(store.guild_discoverable(100), Some(false));
    upsert(
        &mut store,
        json!({"id": "100", "features": ["DISCOVERABLE"]}),
    );
    assert_eq!(store.guild_discoverable(100), Some(true));
    upsert(&mut store, json!({"id": "100", "name": "Renamed"}));
    upsert(
        &mut store,
        json!({"id": "100", "properties": {"features": null}}),
    );
    assert_eq!(store.guild_discoverable(100), Some(true));
    upsert(
        &mut store,
        json!({"id": "100", "properties": {"features": []}}),
    );
    assert_eq!(store.guild_discoverable(100), Some(false));
    store.apply(Event::GuildDelete { id: 100 });
    assert_eq!(store.guild_discoverable(100), None);
}

#[test]
fn guild_discoverable_reads_ready_properties_before_legacy_features() {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../../discord/fixtures/ready.json")).unwrap();
    let guilds = payload["guilds"].as_array_mut().unwrap();
    let guild = guilds
        .iter_mut()
        .find(|guild| model::id_value(&guild["id"]) == Some(200))
        .unwrap();
    guild["properties"]["features"] = json!([]);
    guild["features"] = json!(["DISCOVERABLE"]);
    let guild = guilds
        .iter_mut()
        .find(|guild| model::id_value(&guild["id"]) == Some(100))
        .unwrap();
    guild["features"] = json!(["DISCOVERABLE"]);
    let mut store = Store::from_ready(serde_json::from_value(payload).unwrap());
    assert_eq!(store.guild_discoverable(200), Some(false));
    assert_eq!(store.guild_discoverable(100), Some(true));

    upsert(
        &mut store,
        json!({"id": "200", "features": ["DISCOVERABLE"]}),
    );
    assert_eq!(store.guild_discoverable(200), Some(true));
    upsert(
        &mut store,
        json!({
            "id": "200", "features": ["DISCOVERABLE"], "properties": {"features": []}
        }),
    );
    assert_eq!(store.guild_discoverable(200), Some(false));
}

fn stickers_ready(nitro: bool) -> model::Ready {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../../discord/fixtures/ready.json")).unwrap();
    payload["user"]["premium_type"] = if nitro { json!(2) } else { json!(null) };
    for guild in payload["guilds"].as_array_mut().unwrap() {
        match model::id_value(&guild["id"]) {
            Some(200) => {
                guild["stickers"] = json!([
                    {"id": "9201", "name": "wave", "tags": "greeting,hello",
                     "description": "A wave", "format_type": 1, "type": 2, "available": true},
                    {"id": "9202", "name": "dance", "format_type": 4, "available": false}
                ]);
                for role in guild["roles"].as_array_mut().unwrap() {
                    if model::id_value(&role["id"]) == Some(201) {
                        role["permissions"] = json!("2048");
                    }
                }
            }
            Some(100) => {
                guild["stickers"] = json!([
                    {"id": "9101", "name": "party", "tags": "fun,celebration", "format_type": 3}
                ]);
            }
            _ => {}
        }
    }
    serde_json::from_value(payload).unwrap()
}

#[test]
fn sticker_sections_order_current_guild_first_and_skip_unavailable() {
    let store = Store::from_ready(stickers_ready(false));
    let sections = store.sticker_sections(Some(200), 2002);
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].guild_id, 200);
    assert_eq!(sections[0].name, "Second Guild Name Extra");
    assert_eq!(
        sections[0].stickers.len(),
        1,
        "available == Some(false) stickers are skipped"
    );
    assert_eq!(sections[0].stickers[0].id, 9201);
    assert_eq!(sections[0].stickers[0].tags, "greeting,hello");
    assert_eq!(
        sections[0].stickers[0].description.as_deref(),
        Some("A wave")
    );
    assert_eq!(sections[0].stickers[0].format_type, 1);
    assert!(
        sections[0].stickers[0].usable,
        "current guild needs no nitro"
    );
    assert_eq!(sections[1].guild_id, 100);
    assert_eq!(sections[1].stickers[0].id, 9101);
    assert!(!sections[1].stickers[0].usable, "other guild without nitro");

    let sections = store.sticker_sections(Some(100), 2002);
    assert_eq!(sections[0].guild_id, 100, "current guild leads the order");
    assert!(sections[0].stickers[0].usable);
    assert_eq!(sections[1].guild_id, 200);
    assert!(!sections[1].stickers[0].usable);
}

#[test]
fn external_stickers_need_nitro_and_the_channel_permission() {
    let mut store = Store::from_ready(stickers_ready(true));
    let sections = store.sticker_sections(Some(200), 2002);
    assert!(!sections[1].stickers[0].usable, "nitro alone is not enough");

    assert_eq!(
        role_event(
            &mut store,
            json!({"guild_id": "200", "role": {"id": "201", "permissions":
                (2048u64 | (1u64 << 37)).to_string(), "position": 1}})
        ),
        Change::Structure
    );
    let sections = store.sticker_sections(Some(200), 2002);
    assert!(sections[1].stickers[0].usable);
}

#[test]
fn dm_stickers_need_only_nitro_for_other_guilds() {
    let store = Store::from_ready(stickers_ready(true));
    let sections = store.sticker_sections(None, 900);
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].guild_id, 200);
    assert!(sections[0].stickers[0].usable);
    assert!(sections[1].stickers[0].usable);

    let store = Store::from_ready(stickers_ready(false));
    let sections = store.sticker_sections(None, 900);
    assert!(!sections[0].stickers[0].usable);
    assert!(!sections[1].stickers[0].usable);
}

#[test]
fn sticker_matches_searches_name_and_tags_case_insensitively() {
    let sticker = PickerSticker {
        id: 1,
        name: "Wave Hello".into(),
        tags: "greeting, Party Time".into(),
        description: None,
        format_type: 1,
        usable: true,
    };
    for query in [
        "wave", "WAVE", "hello", "HELL", "greet", "party", "TIME", "",
    ] {
        assert!(sticker_matches(&sticker, query), "{query}");
    }
    for query in ["dance", "goodbye"] {
        assert!(!sticker_matches(&sticker, query), "{query}");
    }
}
