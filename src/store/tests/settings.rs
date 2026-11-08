use super::*;

#[test]
fn confirmed_self_status_maps_all_choices_without_changing_custom_status_or_games() {
    use crate::discord::SelfStatus;
    let mut payload = ready();
    payload.user_settings_proto = Some(settings_fixtures::status_fixture(
        Some("online"),
        Some(("working", 42, "party", 0)),
    ));
    let mut store = Store::from_ready(payload);
    store.apply(Event::SessionsReplace(model::parse_entries(json!([{
        "session_id":"all","status":"online","activities":[{"type":0,"name":"Another client's game"}]
    }]))));
    let custom = store.user(1).unwrap().custom_status.clone();
    let game = store.user(1).unwrap().game.clone();
    for (choice, expected) in [
        (SelfStatus::Online, Status::Online),
        (SelfStatus::Idle, Status::Idle),
        (SelfStatus::DoNotDisturb, Status::DoNotDisturb),
        (SelfStatus::Invisible, Status::Offline),
    ] {
        store.set_self_status(choice);
        let user = store.user(1).unwrap();
        assert!(user.status == expected);
        assert!(
            user.custom_status
                .as_ref()
                .zip(custom.as_ref())
                .is_some_and(|(now, before)| {
                    now.text == before.text && now.expires_at == before.expires_at
                })
        );
        assert_eq!(user.game, game);
    }
}

#[test]
fn rail_folders_ready_join_delete_and_settings_updates() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut payload: serde_json::Value = serde_json::from_str(include_str!(
        "../../../crates/discord/src/fixtures/ready.json"
    ))
    .unwrap();
    payload["user_settings_proto"] = settings_fixtures::fixture(true).into();
    let mut ready: model::Ready = serde_json::from_value(payload).unwrap();
    ready
        .guilds
        .push(serde_json::from_value(json!({"id":"300","name":"New","owner_id":"1"})).unwrap());
    let mut store = Store::from_ready(ready);
    assert_eq!(
        store.rail,
        vec![
            RailEntry::Guild(300),
            RailEntry::Guild(100),
            RailEntry::Folder(Folder {
                id: 42,
                name: Some("Gaming".into()),
                color: Some(0x5865F2),
                guild_ids: vec![200],
            })
        ]
    );
    assert_eq!(store.guild_by_id(100).unwrap().name, "First");
    let guild = serde_json::from_value(json!({"id":"400","name":"Joined","owner_id":"1"})).unwrap();
    store.apply(Event::GuildUpsert(Box::new(guild)));
    assert_eq!(store.rail[0], RailEntry::Guild(400));
    let before = format!("{:?}", store.rail);
    assert_eq!(
        store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
            settings: model::SettingsProto {
                kind: 1,
                proto: STANDARD.encode([8, 1])
            },
            partial: true,
        }))),
        Change::None
    );
    assert_eq!(format!("{:?}", store.rail), before);
    assert_eq!(
        store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
            settings: model::SettingsProto {
                kind: 1,
                proto: STANDARD.encode([114, 0])
            },
            partial: true,
        }))),
        Change::Structure
    );
    assert_eq!(
        store.rail,
        vec![
            RailEntry::Guild(400),
            RailEntry::Guild(200),
            RailEntry::Guild(100),
            RailEntry::Guild(300)
        ]
    );
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto {
            kind: 1,
            proto: settings_fixtures::fixture(false),
        },
        partial: false,
    })));
    store.apply(Event::GuildDelete { id: 200 });
    assert!(store.guild_by_id(200).is_none());
    assert_eq!(
        store.rail,
        vec![
            RailEntry::Guild(400),
            RailEntry::Guild(300),
            RailEntry::Guild(100)
        ]
    );
    let seed = Store::seed();
    assert!(
        matches!(&seed.rail[1], RailEntry::Folder(folder) if folder.name.as_deref() == Some("Gaming") && folder.guild_ids == [2, 3])
    );
}

#[test]
fn settings_own_status_partial_updates_do_not_reorder_folders() {
    let mut store = Store::from_ready(ready());
    let rail = format!("{:?}", store.rail);
    for (value, expected) in [
        ("idle", Status::Idle),
        ("invisible", Status::Offline),
        ("dnd", Status::DoNotDisturb),
        ("unknown", Status::Online),
    ] {
        let proto =
            settings_fixtures::status_fixture(Some(value), Some(("working", 0, "\u{1f3ae}", 0)));
        assert_eq!(
            store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
                settings: model::SettingsProto { kind: 1, proto },
                partial: true
            }))),
            Change::Members { guild_id: None }
        );
        assert!(store.user(1).unwrap().status == expected);
        assert_eq!(
            store.user(1).unwrap().activity.as_deref(),
            Some("\u{1f3ae} working")
        );
        assert_eq!(format!("{:?}", store.rail), rail);
    }
    let proto = settings_fixtures::status_fixture(Some("idle"), None);
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto { kind: 1, proto },
        partial: true,
    })));
    assert_eq!(
        store.user(1).unwrap().activity.as_deref(),
        Some("\u{1f3ae} working")
    );
    let proto = settings_fixtures::status_fixture(None, Some(("expired", 0, "", 1)));
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto { kind: 1, proto },
        partial: true,
    })));
    assert!(store.user(1).unwrap().activity.is_none());
    assert!(store.user(1).unwrap().status == Status::Idle);
}

#[test]
fn settings_custom_status_maps_custom_and_unicode_emoji() {
    let mut store = Store::from_ready(ready());
    let proto = settings_fixtures::status_fixture(Some("online"), Some(("", 42, "party", 0)));
    assert_eq!(
        store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
            settings: model::SettingsProto { kind: 1, proto },
            partial: true,
        }))),
        Change::Members { guild_id: None }
    );
    let status = store.user(1).unwrap().custom_status.as_ref().unwrap();
    assert!(status.text.is_none());
    assert!(matches!(
        &status.emoji,
        Some(StatusEmoji::Custom {
            id: 42,
            name,
            animated: false
        }) if name == "party"
    ));
    assert!(status.expires_at.is_none());

    let proto =
        settings_fixtures::status_fixture(Some("online"), Some(("gaming", 0, "\u{1f3ae}", 0)));
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto { kind: 1, proto },
        partial: true,
    })));
    let status = store.user(1).unwrap().custom_status.as_ref().unwrap();
    assert_eq!(status.text.as_deref(), Some("gaming"));
    assert!(matches!(
        &status.emoji,
        Some(StatusEmoji::Unicode(emoji)) if emoji == "\u{1f3ae}"
    ));

    use std::time::{Duration, UNIX_EPOCH};
    let status = CustomStatus {
        text: Some("later".into()),
        emoji: None,
        expires_at: Some(UNIX_EPOCH + Duration::from_secs(1)),
    };
    assert!(!status.is_expired(UNIX_EPOCH));
    assert!(status.is_expired(UNIX_EPOCH + Duration::from_secs(2)));
}

#[test]
fn ready_and_sessions_keep_configured_own_custom_status() {
    use std::time::{Duration, UNIX_EPOCH};

    const EXPIRES_AT_MS: u64 = 4_102_444_800_000;
    let mut payload = ready();
    payload.user_settings_proto = Some(settings_fixtures::status_fixture(
        Some("idle"),
        Some(("working", 42, "party", EXPIRES_AT_MS)),
    ));
    payload.sessions = model::parse_entries(json!([{
        "session_id": "all",
        "status": "idle",
        "activities": []
    }]));
    let mut store = Store::from_ready(ready());
    store.apply(Event::Ready(Box::new(payload)));

    let expires_at = Some(UNIX_EPOCH + Duration::from_millis(EXPIRES_AT_MS));
    let user = store.user(1).unwrap();
    assert!(user.status == Status::Idle);
    assert_eq!(user.activity.as_deref(), Some("working"));
    let status = user.custom_status.as_ref().unwrap();
    assert_eq!(status.text.as_deref(), Some("working"));
    assert!(matches!(
        &status.emoji,
        Some(StatusEmoji::Custom {
            id: 42,
            name,
            animated: false
        }) if name == "party"
    ));
    assert_eq!(status.expires_at, expires_at);

    // A game-only session update keeps the configured status while updating the game.
    store.apply(Event::SessionsReplace(model::parse_entries(json!([{
        "session_id": "all",
        "status": "online",
        "activities": [{"type": 0, "name": "Visual Studio Code"}]
    }]))));
    let user = store.user(1).unwrap();
    assert!(user.status == Status::Online);
    assert_eq!(user.game.as_ref().unwrap().name, "Visual Studio Code");
    assert_eq!(user.activity.as_deref(), Some("working"));
    assert!(!user.playing);
    assert_eq!(user.custom_status.as_ref().unwrap().expires_at, expires_at);

    // Listening-only session updates keep it too.
    store.apply(Event::SessionsReplace(model::parse_entries(json!([{
        "session_id": "all",
        "activities": [{"type": 2, "name": "Spotify"}]
    }]))));
    let user = store.user(1).unwrap();
    assert_eq!(user.listening.as_ref().unwrap().name, "Spotify");
    assert!(user.game.is_none());
    assert_eq!(user.custom_status.as_ref().unwrap().expires_at, expires_at);

    // Empty activities clear game/listening but not the settings-owned status.
    store.apply(Event::SessionsReplace(model::parse_entries(json!([{
        "session_id": "all",
        "activities": []
    }]))));
    let user = store.user(1).unwrap();
    assert!(user.game.is_none());
    assert!(user.listening.is_none());
    assert_eq!(user.custom_status.as_ref().unwrap().expires_at, expires_at);
    assert_eq!(
        user.custom_status.as_ref().unwrap().text.as_deref(),
        Some("working")
    );
    assert!(matches!(
        &user.custom_status.as_ref().unwrap().emoji,
        Some(StatusEmoji::Custom { id: 42, .. })
    ));
    assert_eq!(user.activity.as_deref(), Some("working"));
}

#[test]
fn authoritative_settings_clear_own_custom_status_across_sessions() {
    let mut store = Store::from_ready(ready());
    let set =
        settings_fixtures::status_fixture(Some("online"), Some(("working", 0, "\u{1f3ae}", 0)));
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto {
            kind: 1,
            proto: set,
        },
        partial: true,
    })));
    assert!(store.user(1).unwrap().custom_status.is_some());

    // A full settings response without the custom-status field clears it.
    let clear = settings_fixtures::status_fixture(Some("idle"), None);
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto {
            kind: 1,
            proto: clear,
        },
        partial: false,
    })));
    assert!(store.user(1).unwrap().custom_status.is_none());
    store.apply(Event::SessionsReplace(model::parse_entries(json!([{
        "session_id": "all",
        "status": "idle",
        "activities": [{"type": 0, "name": "Visual Studio Code"}]
    }]))));
    let user = store.user(1).unwrap();
    assert!(user.custom_status.is_none());
    assert_eq!(user.game.as_ref().unwrap().name, "Visual Studio Code");

    // A partial update that omits the custom-status field preserves the known value...
    let set = settings_fixtures::status_fixture(Some("online"), Some(("back", 0, "\u{1f3ae}", 0)));
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto {
            kind: 1,
            proto: set,
        },
        partial: true,
    })));
    let omit = settings_fixtures::status_fixture(Some("dnd"), None);
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto {
            kind: 1,
            proto: omit,
        },
        partial: true,
    })));
    assert!(store.user(1).unwrap().custom_status.is_some());
    assert_eq!(
        store
            .user(1)
            .unwrap()
            .custom_status
            .as_ref()
            .and_then(|s| s.text.as_deref()),
        Some("back")
    );

    // ...while an explicit empty/expired custom-status value clears it again.
    let clear = settings_fixtures::status_fixture(None, Some(("expired", 0, "", 1)));
    store.apply(Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto {
            kind: 1,
            proto: clear,
        },
        partial: true,
    })));
    assert!(store.user(1).unwrap().custom_status.is_none());
    store.apply(Event::SessionsReplace(model::parse_entries(json!([{
        "session_id": "all",
        "activities": []
    }]))));
    assert!(store.user(1).unwrap().custom_status.is_none());
}

#[test]
fn session_custom_status_is_used_when_no_settings_status_is_known() {
    let mut store = Store::from_ready(ready());
    store.apply(Event::SessionsReplace(model::parse_entries(json!([{
        "session_id": "all",
        "activities": [{"type": 4, "state": "hello"}]
    }]))));
    assert_eq!(
        store
            .user(1)
            .unwrap()
            .custom_status
            .as_ref()
            .and_then(|s| s.text.as_deref()),
        Some("hello")
    );
}

#[test]
fn expired_configured_custom_status_is_not_restored_by_sessions_or_echoes() {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    let mut store = Store::from_ready(ready());
    let expired = crate::discord::settings_proto::StatusSettings {
        status: Some("online".into()),
        custom_status_present: true,
        show_current_game: None,
        custom_status: Some(crate::discord::settings_proto::CustomStatus {
            text: "old".into(),
            emoji_id: 0,
            emoji_name: String::new(),
            expires_at_ms: 1,
        }),
    };
    store.apply_own_status(Some(&expired), false);
    let expires_at = Some(UNIX_EPOCH + Duration::from_millis(1));
    assert_eq!(
        store
            .user(1)
            .unwrap()
            .custom_status
            .as_ref()
            .unwrap()
            .expires_at,
        expires_at
    );

    // A live echo of the same expired status keeps the expiry instead of becoming immortal.
    let echo: model::Presence = serde_json::from_value(json!({
        "user_id":"1",
        "activities":[{"type":4,"state":"old"}]
    }))
    .unwrap();
    store.apply(Event::PresenceUpdate(Box::new(echo)));
    let status = store.user(1).unwrap().custom_status.as_ref().unwrap();
    assert_eq!(status.expires_at, expires_at);
    assert!(status.is_expired(SystemTime::now()));

    // Game-only snapshots must clear it and expose the real activity line.
    let game: model::Presence = serde_json::from_value(json!({
        "user_id":"1",
        "status":"online",
        "activities":[{"type":0,"name":"Visual Studio Code"}]
    }))
    .unwrap();
    store.apply(Event::PresenceUpdate(Box::new(game)));
    let user = store.user(1).unwrap();
    assert!(user.custom_status.is_none());
    assert_eq!(user.game.as_ref().unwrap().name, "Visual Studio Code");
    assert_eq!(user.activity.as_deref(), Some("Visual Studio Code"));
    assert!(user.playing);

    let listening: model::Presence = serde_json::from_value(json!({
        "user_id":"1",
        "activities":[{"type":2,"name":"Spotify"}]
    }))
    .unwrap();
    store.apply(Event::PresenceUpdate(Box::new(listening)));
    let user = store.user(1).unwrap();
    assert!(user.custom_status.is_none());
    assert_eq!(user.listening.as_ref().unwrap().name, "Spotify");
    assert_eq!(user.activity.as_deref(), Some("Listening to Spotify"));
    assert!(!user.playing);

    // Empty snapshots leave no imported custom status/activity behind either.
    let empty: model::Presence = serde_json::from_value(json!({
        "user_id":"1",
        "activities":[]
    }))
    .unwrap();
    store.apply(Event::PresenceUpdate(Box::new(empty)));
    let user = store.user(1).unwrap();
    assert!(user.custom_status.is_none());
    assert!(user.activity.is_none());
    assert!(!user.playing);
}

#[test]
fn identical_live_custom_status_keeps_configured_expiry_and_emoji_animation() {
    use std::time::{Duration, UNIX_EPOCH};

    const EXPIRES_AT_MS: u64 = 4_102_444_800_000;
    let mut store = Store::from_ready(ready());
    let configured = crate::discord::settings_proto::StatusSettings {
        status: Some("online".into()),
        custom_status_present: true,
        show_current_game: None,
        custom_status: Some(crate::discord::settings_proto::CustomStatus {
            text: "working".into(),
            emoji_id: 42,
            emoji_name: "party".into(),
            expires_at_ms: EXPIRES_AT_MS,
        }),
    };
    store.apply_own_status(Some(&configured), false);

    let presence: model::Presence = serde_json::from_value(json!({
        "user_id":"1",
        "activities":[{
            "type":4,
            "state":"working",
            "emoji":{"id":"42","name":"renamed","animated":true}
        }]
    }))
    .unwrap();
    store.apply(Event::PresenceUpdate(Box::new(presence)));
    let status = store.user(1).unwrap().custom_status.as_ref().unwrap();
    assert_eq!(
        status.expires_at,
        Some(UNIX_EPOCH + Duration::from_millis(EXPIRES_AT_MS))
    );
    assert!(matches!(
        &status.emoji,
        Some(StatusEmoji::Custom {
            id: 42,
            name,
            animated: true
        }) if name == "renamed"
    ));

    // A following game-only snapshot keeps the same live emoji metadata and expiry.
    let game: model::Presence = serde_json::from_value(json!({
        "user_id":"1",
        "activities":[{"type":0,"name":"Visual Studio Code"}]
    }))
    .unwrap();
    store.apply(Event::PresenceUpdate(Box::new(game)));
    let user = store.user(1).unwrap();
    let status = user.custom_status.as_ref().unwrap();
    assert_eq!(
        status.expires_at,
        Some(UNIX_EPOCH + Duration::from_millis(EXPIRES_AT_MS))
    );
    assert!(matches!(
        &status.emoji,
        Some(StatusEmoji::Custom {
            id: 42,
            name,
            animated: true
        }) if name == "renamed"
    ));
    assert_eq!(user.game.as_ref().unwrap().name, "Visual Studio Code");
    assert_eq!(user.activity.as_deref(), Some("working"));
    assert!(!user.playing);
}

#[test]
fn different_live_custom_status_supersedes_the_configured_fallback() {
    let mut store = Store::from_ready(ready());
    let configured = crate::discord::settings_proto::StatusSettings {
        status: Some("online".into()),
        custom_status_present: true,
        show_current_game: None,
        custom_status: Some(crate::discord::settings_proto::CustomStatus {
            text: "old".into(),
            emoji_id: 0,
            emoji_name: String::new(),
            expires_at_ms: 4_102_444_800_000,
        }),
    };
    store.apply_own_status(Some(&configured), false);

    let live: model::Presence = serde_json::from_value(json!({
        "user_id":"1",
        "activities":[{"type":4,"state":"new"}]
    }))
    .unwrap();
    store.apply(Event::PresenceUpdate(Box::new(live)));
    let live = store.user(1).unwrap().custom_status.as_ref().unwrap();
    assert_eq!(live.text.as_deref(), Some("new"));
    assert!(live.expires_at.is_none());

    // Once a different live status has superseded the settings value, an absent custom
    // activity must not resurrect the old configured status.
    store.apply(Event::SessionsReplace(model::parse_entries(json!([{
        "session_id": "all",
        "activities": []
    }]))));
    let user = store.user(1).unwrap();
    assert!(user.custom_status.is_none());
    assert!(user.activity.is_none());
}

#[test]
fn open_at_settings_without_new_fields_load() {
    let settings: crate::settings::Settings = serde_json::from_str(
        r#"{"zoom":4,"window":{"x":1.0,"y":2.0,"width":800.0,"height":600.0,"maximized":false}}"#,
    )
    .unwrap();
    assert_eq!(settings.zoom, Some(4));
    assert!(settings.last_channel.is_none());
    assert!(settings.guild_channels.is_empty());

    let saved: crate::settings::Settings = serde_json::from_value(json!({
        "last_channel": {"guild_id": 200, "channel_id": 2002},
        "guild_channels": {"200": 2002}
    }))
    .unwrap();
    assert_eq!(saved.last_channel.unwrap().channel_id, 2002);
    assert_eq!(saved.guild_channels.get(&200), Some(&2002));
    let round_trip: crate::settings::Settings =
        serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    assert!(round_trip == saved);
}

#[test]
fn frecency_full_updates_replace_favorites_and_report_the_change() {
    let mut store = Store::from_ready(ready());
    assert!(store.frecency().is_none());
    assert!(store.favorite_gifs().is_empty());
    let older = favorite_gif("https://klipy.com/gifs/older", 1);
    let newer = favorite_gif("https://klipy.com/gifs/newer", 2);
    let proto = frecency_proto(&[older.clone(), newer.clone()], false);
    assert_eq!(
        store.apply(settings_update(2, &proto, false)),
        Change::FavoriteGifs
    );
    assert_eq!(store.frecency(), Some(proto.as_slice()));
    assert_eq!(store.favorite_gifs(), [newer.clone(), older.clone()]);
    assert!(store.is_favorite_gif(&newer.url));
    assert!(!store.is_favorite_gif("https://klipy.com/gifs/missing"));
    assert_eq!(store.apply(settings_update(2, &proto, false)), Change::None);
}

#[test]
fn frecency_partial_updates_merge_onto_the_known_state() {
    let mut store = Store::from_ready(ready());
    let first = favorite_gif("https://klipy.com/gifs/first", 1);
    assert_eq!(
        store.apply(settings_update(
            2,
            &frecency_proto(&[first.clone()], true),
            false
        )),
        Change::FavoriteGifs
    );
    let versions = field(1, &uint(1, 4));
    assert_eq!(
        store.apply(settings_update(2, &versions, true)),
        Change::None
    );
    assert_eq!(store.favorite_gifs(), [first.clone()]);

    let unchanged = field(2, &[uint(2, 0), field(1, &favorite_entry(&first))].concat());
    assert_eq!(
        store.apply(settings_update(2, &unchanged, true)),
        Change::None
    );
    assert_eq!(store.favorite_gifs(), [first]);

    let second = favorite_gif("https://klipy.com/gifs/second", 5);
    let update = field(2, &field(1, &favorite_entry(&second)));
    assert_eq!(
        store.apply(settings_update(2, &update, true)),
        Change::FavoriteGifs
    );
    assert_eq!(store.favorite_gifs(), [second]);
    let expected = [
        field(1, &uint(1, 4)),
        field(
            2,
            &field(
                1,
                &favorite_entry(&favorite_gif("https://klipy.com/gifs/second", 5)),
            ),
        ),
        field(5, &field(1, &[])),
    ]
    .concat();
    assert_eq!(store.frecency(), Some(expected.as_slice()));
}

#[test]
fn frecency_partial_updates_are_ignored_before_the_state_is_known() {
    let mut store = Store::from_ready(ready());
    let gif = favorite_gif("https://klipy.com/gifs/unknown", 1);
    let update = field(2, &field(1, &favorite_entry(&gif)));
    assert_eq!(store.apply(settings_update(2, &update, true)), Change::None);
    assert!(store.frecency().is_none());
    assert!(store.favorite_gifs().is_empty());
}

#[test]
fn malformed_frecency_updates_clear_known_state() {
    let mut store = Store::from_ready(ready());
    let gif = favorite_gif("https://klipy.com/gifs/existing", 1);
    store.set_frecency(frecency_proto(&[gif.clone()], false));
    assert_eq!(store.favorite_gifs(), [gif.clone()]);
    assert_eq!(
        store.apply(settings_update(2, &[18, 5, 10], false)),
        Change::FavoriteGifs
    );
    assert!(store.frecency().is_none());
    assert!(store.favorite_gifs().is_empty());

    store.set_frecency(frecency_proto(&[gif], false));
    let broken = Event::SettingsUpdate(Box::new(model::SettingsUpdate {
        settings: model::SettingsProto {
            kind: 2,
            proto: "not base64!".into(),
        },
        partial: false,
    }));
    assert_eq!(store.apply(broken), Change::FavoriteGifs);
    assert!(store.frecency().is_none());
    assert!(store.favorite_gifs().is_empty());
}

#[test]
fn account_theme_follows_ready_and_full_and_partial_settings_updates() {
    use crate::discord::settings_proto::AccountTheme;

    let mut payload = ready();
    payload.user_settings_proto = Some(STANDARD.encode(field(13, &uint(1, 1))));
    let mut store = Store::from_ready(payload);
    assert_eq!(store.account_theme(), Some(AccountTheme::Dark));

    assert_eq!(
        store.apply(settings_update(1, &field(13, &uint(1, 2)), true)),
        Change::Theme
    );
    assert_eq!(store.account_theme(), Some(AccountTheme::Light));
    assert_eq!(
        store.apply(settings_update(1, &field(13, &uint(1, 2)), true)),
        Change::None
    );

    // A partial update without the appearance field leaves the theme alone.
    let status = field(11, &field(1, &field(1, b"idle")));
    assert_eq!(
        store.apply(settings_update(1, &status, true)),
        Change::Members { guild_id: None }
    );
    assert_eq!(store.account_theme(), Some(AccountTheme::Light));

    // A full update without the appearance field clears it.
    assert_eq!(
        store.apply(settings_update(1, &status, false)),
        Change::Structure
    );
    assert_eq!(store.account_theme(), None);
}

#[test]
fn text_images_and_favorites_follow_ready_and_whole_message_updates() {
    use settings_proto::{FavoriteChannel, TextImages};

    let channel = FavoriteChannel {
        id: 123,
        nickname: "saved".into(),
        kind: 1,
        ..Default::default()
    };
    let favorites = settings_proto::favorite_channel_patch(&[], 123, Some(channel.clone()))
        .unwrap()
        .unwrap();
    let favorites = settings_proto::favorites_visible_patch(&favorites, true)
        .unwrap()
        .unwrap();
    let mut payload = ready();
    payload.user_settings_proto = Some(
        STANDARD.encode(
            [
                field(6, &[field(11, &[]), field(17, &uint(1, 1))].concat()),
                favorites,
            ]
            .concat(),
        ),
    );
    let mut store = Store::from_ready(payload);
    assert!(!store.text_images().gif_auto_play);
    assert!(store.text_images().message_display_compact);
    assert!(store.favorites().guild_visible);
    assert_eq!(store.favorites().channels, [channel]);

    let text = field(6, &field(14, &[]));
    assert_eq!(
        store.apply(settings_update(1, &text, true)),
        Change::TextImages
    );
    assert!(store.text_images().gif_auto_play);
    assert!(!store.text_images().animate_emoji);
    assert!(!store.text_images().message_display_compact);
    assert!(store.favorites().guild_visible);
    assert_eq!(store.apply(settings_update(1, &text, true)), Change::None);

    assert_eq!(
        store.apply(settings_update(1, &field(15, &[]), true)),
        Change::Favorites
    );
    assert_eq!(store.favorites(), &settings_proto::Favorites::default());
    assert!(!store.text_images().animate_emoji);
    assert_eq!(
        store.apply(settings_update(1, &field(6, &[]), true)),
        Change::TextImages
    );
    assert_eq!(store.text_images(), &TextImages::default());

    store.apply(settings_update(1, &text, true));
    store.apply(settings_update(1, &field(15, &field(3, &uint(1, 1))), true));
    assert_eq!(
        store.apply(settings_update(1, &[], false)),
        Change::Structure
    );
    assert_eq!(store.text_images(), &TextImages::default());
    assert_eq!(store.favorites(), &settings_proto::Favorites::default());
}

#[test]
fn client_theme_changes_and_empty_appearance_notify_theme() {
    use settings_proto::{AccountTheme, ClientTheme, CustomTheme};

    let initial =
        settings_proto::appearance_patch(&[], AccountTheme::Dark, Some(&ClientTheme::Preset(42)))
            .unwrap();
    let mut payload = ready();
    payload.user_settings_proto = Some(STANDARD.encode(&initial));
    let mut store = Store::from_ready(payload);
    assert_eq!(store.client_theme(), Some(&ClientTheme::Preset(42)));
    assert_eq!(
        store.apply(settings_update(1, &initial, true)),
        Change::None
    );
    let custom = ClientTheme::Custom(CustomTheme {
        colors: vec!["#5348ca".into(), "#abcdef".into()],
        stops: vec![0.0, 1.0],
        angle: 45,
        base_mix: 74,
    });
    let next =
        settings_proto::appearance_patch(&initial, AccountTheme::Dark, Some(&custom)).unwrap();
    assert_eq!(store.apply(settings_update(1, &next, true)), Change::Theme);
    assert_eq!(store.client_theme(), Some(&custom));
    assert_eq!(store.account_theme(), Some(AccountTheme::Dark));
    assert_eq!(
        store.apply(settings_update(1, &field(6, &[]), true)),
        Change::None
    );
    assert_eq!(store.client_theme(), Some(&custom));
    assert_eq!(
        store.apply(settings_update(1, &field(13, &[]), true)),
        Change::Theme
    );
    assert_eq!(store.client_theme(), None);
    assert_eq!(store.account_theme(), None);
    store.apply(settings_update(1, &initial, true));
    assert_eq!(
        store.apply(settings_update(1, &[], false)),
        Change::Structure
    );
    assert_eq!(store.client_theme(), None);
}

#[test]
fn settings_change_priority_still_installs_lower_priority_values() {
    use settings_proto::AccountTheme;

    let mut store = Store::from_ready(ready());
    let text = field(6, &field(11, &[]));
    let favorites = field(15, &field(3, &uint(1, 1)));
    let theme = field(13, &uint(1, 2));
    let status = field(11, &field(1, &field(1, b"idle")));
    for (extra, expected) in [
        (Vec::new(), Change::Theme),
        (status, Change::Members { guild_id: None }),
        (field(14, &[]), Change::Structure),
    ] {
        store.apply(settings_update(1, &[], false));
        let update = [text.clone(), favorites.clone(), theme.clone(), extra].concat();
        assert_eq!(store.apply(settings_update(1, &update, true)), expected);
        assert!(!store.text_images().gif_auto_play);
        assert!(store.favorites().guild_visible);
        assert_eq!(store.account_theme(), Some(AccountTheme::Light));
    }
    store.apply(settings_update(1, &[], false));
    assert_eq!(
        store.apply(settings_update(1, &[text, favorites].concat(), true)),
        Change::TextImages
    );
    assert!(store.favorites().guild_visible);
}

#[test]
fn saved_settings_can_be_installed_before_the_gateway_echo() {
    let mut store = Store::from_ready(ready());
    let text = settings_proto::TextImages {
        gif_auto_play: false,
        ..Default::default()
    };
    assert_eq!(store.set_text_images(text.clone()), Change::TextImages);
    assert_eq!(store.text_images(), &text);
    assert_eq!(store.set_text_images(text), Change::None);
    assert_eq!(
        store.apply(settings_update(1, &field(6, &field(11, &[])), true)),
        Change::None
    );
    let favorites = settings_proto::Favorites {
        guild_visible: true,
        ..Default::default()
    };
    assert_eq!(store.set_favorites(favorites.clone()), Change::Favorites);
    assert_eq!(store.favorites(), &favorites);
    assert_eq!(store.set_favorites(favorites), Change::None);
    assert_eq!(
        store.apply(settings_update(1, &field(15, &field(3, &uint(1, 1))), true)),
        Change::None
    );
}
