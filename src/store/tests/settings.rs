use super::*;

#[test]
fn rail_folders_ready_join_delete_and_settings_updates() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../../discord/fixtures/ready.json")).unwrap();
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
