use super::*;

#[test]
fn avatar_defaults_and_image_attachments() {
    let mut user = map_user(&ready().user);
    user.id = 5 << 22;
    for discriminator in [None, Some("0".to_owned())] {
        user.discriminator = discriminator;
        assert!(matches!(
            user.avatar_spec(),
            ImageSpec::DefaultAvatar { index: 5 }
        ));
    }
    user.discriminator = Some("1234".into());
    assert!(matches!(
        user.avatar_spec(),
        ImageSpec::DefaultAvatar { index: 4 }
    ));
    user.avatar = Some("a_hash".into());
    assert!(matches!(user.avatar_spec(), ImageSpec::Avatar { .. }));
    let mut store = Store::from_ready(ready());
    assert!(matches!(
        store.guilds[0].icon_spec(),
        Some(ImageSpec::GuildIcon { guild_id: 200, .. })
    ));
    store.set_messages(2002, messages());
    let update = serde_json::from_value(json!({"id":"62","channel_id":"2002","attachments":[{
        "id":"77","filename":"image.GIF","url":"https://cdn.discordapp.com/attachments/1/2/image.gif",
        "proxy_url":"https://media.discordapp.net/attachments/1/2/image.gif", "width":400,"height":200,"size":42
    }, {"id":"bad"}]})).unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    let message = &store.guilds[0].channel(2002).unwrap().messages[1];
    assert_eq!(message.attachments.len(), 1);
    assert!(matches!(
        message.attachments[0].image_spec(),
        Some(ImageSpec::Attachment {
            id: 77,
            width: 400,
            height: 200,
            ..
        })
    ));
    let mut attachment = message.attachments[0].clone();
    attachment.width = None;
    assert!(attachment.image_spec().is_none());
    attachment.width = Some(400);
    attachment.filename = "not-image.pdf".into();
    assert!(attachment.image_spec().is_none());
    attachment.content_type = Some("image/png".into());
    assert!(attachment.image_spec().is_some());
    let update =
        serde_json::from_value(json!({"id":"62","channel_id":"2002","attachments":[]})).unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    assert!(
        store.guilds[0].channel(2002).unwrap().messages[1]
            .attachments
            .is_empty()
    );
}

#[test]
fn supplemental_chunk_names_colors_avatars_and_missing_members() {
    let mut store = Store::from_ready(ready());
    assert!(store.user(1).unwrap().status == Status::Online);
    assert_eq!(store.member_name(200, 2), Some("A Writer"));
    assert_eq!(store.member_name(200, 77), None);
    assert_eq!(store.missing_members(200, [1, 2, 2, 3, 4]), [2, 3, 4]);
    assert!(store.missing_members(200, [2, 3, 4]).is_empty());
    let supplemental = serde_json::from_value(social_fixture("supplemental")).unwrap();
    assert_eq!(
        store.apply(Event::ReadySupplemental(Box::new(supplemental))),
        Change::Members { guild_id: None }
    );
    assert_eq!(store.member_name(200, 2), Some("Writer Nick"));
    assert_eq!(store.member_name(100, 2), Some("Other Nick"));
    assert!(store.user(2).unwrap().status == Status::Idle);
    assert_eq!(store.user(2).unwrap().activity.as_deref(), Some("Example Game"));
    assert!(store.user(2).unwrap().playing);
    let chunk = serde_json::from_value(social_fixture("chunk")).unwrap();
    assert_eq!(
        store.apply(Event::MembersChunk(Box::new(chunk))),
        Change::Members {
            guild_id: Some(200)
        }
    );
    assert_eq!(store.member_name(200, 3), Some("Three"));
    assert!(
        matches!(store.member_avatar_spec(200,3), Some(ImageSpec::GuildAvatar { guild_id:200,user_id:3,hash }) if hash == "guild_hash")
    );
    assert!(store.user(3).unwrap().avatar.is_none());
    assert!(store.user(3).unwrap().status == Status::DoNotDisturb);
    assert_eq!(
        store.user(3).unwrap().activity.as_deref(),
        Some("Streaming Making things")
    );
    assert!(store.missing_members(200, [3, 4]).is_empty());
    for (id, position, color, primary) in [
        (201, 1, 0x112233, 0),
        (202, 3, 0x445566, 0x778899),
        (203, 9, 0, 0),
    ] {
        assert_eq!(
            role_event(
                &mut store,
                json!({"guild_id":"200","role":{"id":id.to_string(),"name":format!("Role {id}"),"permissions":"0","position":position,"color":color,"colors":{"primary_color":primary},"hoist":true}})
            ),
            Change::Structure
        );
    }
    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"2"},"roles":["201","202","203"]})
        ),
        Change::Members {
            guild_id: Some(200)
        }
    );
    assert_eq!(store.member_color(200, 2), Some(0x778899));
    assert_eq!(store.role(200, 202).unwrap().position, 3);
    assert!(store.role(200, 202).unwrap().hoist);
    assert_eq!(store.member_name(200, 2), Some("Writer Nick"));
    member_event(
        &mut store,
        json!({"guild_id":"200","user":{"id":"2"},"nick":null}),
    );
    assert_eq!(store.member_name(200, 2), Some("A Writer"));
    member_event(
        &mut store,
        json!({"guild_id":"200","user":{"id":"8","username":"username"},"roles":[]}),
    );
    assert_eq!(store.member_name(200, 8), Some("username"));
    assert_eq!(
        store.apply(Event::RoleDelete {
            guild_id: 200,
            role_id: 202
        }),
        Change::Structure
    );
    assert_eq!(store.member_color(200, 2), Some(0x112233));
    assert!(store.role(200, 202).is_none());
    store.apply(Event::Ready(Box::new(ready())));
    assert_eq!(store.missing_members(200, [4]), [4]);
}

#[test]
fn own_roles_recompute_permissions_and_member_removal() {
    let mut store = Store::from_ready(ready());
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_some());
    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"1"},"roles":[]})
        ),
        Change::Structure
    );
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_none());
    assert!(
        store.source.as_ref().unwrap().guilds[0]
            .members
            .as_ref()
            .unwrap()[0]
            .roles
            .is_empty()
    );
    assert_eq!(
        member_event(
            &mut store,
            json!({"guild_id":"200","user":{"id":"1"},"roles":["201"]})
        ),
        Change::Structure
    );
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_some());
    assert_eq!(
        store.apply(Event::MemberRemove(Box::new(
            serde_json::from_value(json!({"guild_id":"200","user":{"id":"1"}})).unwrap()
        ))),
        Change::Structure
    );
    assert!(store.guild_by_id(200).unwrap().channel(2002).is_none());
}

#[test]
fn member_list_all_ops_switching_and_item_presence() {
    let mut store = Store::from_ready(ready());
    role_event(
        &mut store,
        json!({"guild_id":"200","role":{"id":"201","name":"Mods","color":42}}),
    );
    let list: model::MemberListUpdate = serde_json::from_value(social_fixture("list")).unwrap();
    assert_eq!(
        store.apply(Event::MemberListUpdate(Box::new(list))),
        Change::Members {
            guild_id: Some(200)
        }
    );
    let rows = &store.member_list(200).unwrap().rows;
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows[0],
        Some(MemberRow::Group {
            name: "Mods".into(),
            count: 1
        })
    );
    assert_eq!(rows[1], Some(MemberRow::Member { user_id: 2 }));
    assert_eq!(store.member_name(200, 2), Some("List Nick"));
    role_event(
        &mut store,
        json!({"guild_id":"200","role":{"id":"201","name":"Renamed Mods","color":43}}),
    );
    assert_eq!(
        store.member_list(200).unwrap().rows[0],
        Some(MemberRow::Group {
            name: "Renamed Mods".into(),
            count: 1
        })
    );
    assert_eq!(
        store.user(2).unwrap().activity.as_deref(),
        Some("\u{1f3ae} grinding")
    );
    let mut update = social_fixture("list");
    update["ops"] = json!([
        {"op":"INSERT","index":1,"item":{"member":{"user":{"id":"8","username":"inserted"},"roles":[]}}},
        {"op":"UPDATE","index":2,"item":{"member":{"user":{"id":"9","username":"updated"},"roles":[]}}},
        {"op":"DELETE","index":3},
        {"op":"INVALIDATE","range":[0,0]}
    ]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update.clone()).unwrap(),
    )));
    assert_eq!(
        store.member_list(200).unwrap().rows,
        vec![
            None,
            Some(MemberRow::Member { user_id: 8 }),
            Some(MemberRow::Member { user_id: 9 }),
            Some(MemberRow::Member { user_id: 3 })
        ]
    );
    update["id"] = "different".into();
    update["ops"] = json!([{"op":"UPDATE","index":1,"item":{"member":{"user":{"id":"77","username":"ignored"}}}}]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update.clone()).unwrap(),
    )));
    assert_eq!(store.member_list(200).unwrap().id, "everyone");
    assert!(store.user(77).is_none());
    update["ops"] = json!([{"op":"SYNC","range":[2,3],"items":[{"group":{"id":"online","count":1}},{"member":{"user":{"id":"77","username":"synced"}}}]}]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update.clone()).unwrap(),
    )));
    let list = store.member_list(200).unwrap();
    assert_eq!(list.id, "different");
    assert_eq!(list.rows[0], None);
    assert_eq!(
        list.rows[2],
        Some(MemberRow::Group {
            name: "Online".into(),
            count: 0
        })
    );
    assert_eq!(list.rows.len(), 4);
    update["groups"] = json!([{"id":"offline","count":1}]);
    update["ops"] =
        json!([{"op":"SYNC","range":[0,0],"items":[{"group":{"id":"unknown-role","count":1}}]}]);
    store.apply(Event::MemberListUpdate(Box::new(
        serde_json::from_value(update).unwrap(),
    )));
    assert_eq!(store.member_list(200).unwrap().rows.len(), 2);
    assert_eq!(
        store.member_list(200).unwrap().rows[0],
        Some(MemberRow::Group {
            name: "Role".into(),
            count: 1
        })
    );
}

#[test]
fn presence_activity_mapping_and_partial_users_preserve_names() {
    let mut store = Store::from_ready(ready());
    for (value, expected) in [
        ("online", Status::Online),
        ("idle", Status::Idle),
        ("dnd", Status::DoNotDisturb),
        ("offline", Status::Offline),
        ("invisible", Status::Offline),
    ] {
        let presence =
            serde_json::from_value(json!({"user":{"id":"2"},"status":value,"activities":[]}))
                .unwrap();
        assert_eq!(
            store.apply(Event::PresenceUpdate(Box::new(presence))),
            Change::Members { guild_id: None }
        );
        assert!(store.user(2).unwrap().status == expected);
        assert_eq!(store.user(2).unwrap().name, "A Writer");
    }
    for (kind, expected) in [
        (0, "Example Game"),
        (1, "Streaming Example Game"),
        (2, "Listening to Example Game"),
        (3, "Watching Example Game"),
        (5, "Competing in Example Game"),
    ] {
        let presence = serde_json::from_value(
            json!({"user_id":"2","guild_id":"200","activities":[{"type":kind,"name":"Example Game"}]}),
        )
        .unwrap();
        store.apply(Event::PresenceUpdate(Box::new(presence)));
        assert_eq!(store.user(2).unwrap().activity.as_deref(), Some(expected));
    }
    let presence = serde_json::from_value(json!({"user_id":"2","activities":[{"type":0,"name":"Game"},{"type":4,"state":"custom","emoji":{"id":"77","name":"custom_emoji"}}]})).unwrap();
    store.apply(Event::PresenceUpdate(Box::new(presence)));
    assert_eq!(store.user(2).unwrap().activity.as_deref(), Some("custom"));
    let mut message = messages().remove(0);
    message.author.id = 2;
    message.author.username = "writer".into();
    store.set_messages(2002, vec![message]);
    assert_eq!(store.user(2).unwrap().activity.as_deref(), Some("custom"));
}

#[test]
fn presence_game_activity_tracks_playing_and_clears() {
    use std::time::{Duration, UNIX_EPOCH};

    let mut store = Store::from_ready(ready());
    let presence: model::Presence = serde_json::from_value(json!({
        "user_id": "2",
        "activities": [{
            "type": 0,
            "name": "Example Game",
            "application_id": "123",
            "timestamps": {"start": 1700000000000u64}
        }]
    }))
    .unwrap();
    store.apply(Event::PresenceUpdate(Box::new(presence)));
    let game = store.user(2).unwrap().game.clone().unwrap();
    assert_eq!(game.name, "Example Game");
    assert_eq!(game.application_id, Some(123));
    assert_eq!(
        game.started_at,
        Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_000))
    );

    // An author refresh must keep the presence-derived game.
    let mut message = messages().remove(0);
    message.author.id = 2;
    message.author.username = "writer".into();
    store.set_messages(2002, vec![message]);
    assert!(store.user(2).unwrap().game.is_some());

    let presence: model::Presence =
        serde_json::from_value(json!({"user_id":"2","activities":[]})).unwrap();
    store.apply(Event::PresenceUpdate(Box::new(presence)));
    assert!(store.user(2).unwrap().game.is_none());
    assert!(!store.user(2).unwrap().playing);
}

#[test]
fn presence_listening_activity_keeps_kind_and_timestamps() {
    use std::time::{Duration, UNIX_EPOCH};

    let mut store = Store::from_ready(ready());
    let presence: model::Presence = serde_json::from_value(json!({
        "user_id": "2",
        "activities": [{
            "type": 2,
            "name": "YouTube Music",
            "timestamps": {
                "start": 1700000000000u64,
                "end": 1700000300000u64
            }
        }]
    }))
    .unwrap();
    store.apply(Event::PresenceUpdate(Box::new(presence)));
    let listening = store.user(2).unwrap().listening.clone().unwrap();
    assert_eq!(listening.kind, 2);
    assert_eq!(
        listening.started_at,
        Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_000))
    );
    assert_eq!(
        listening.ends_at,
        Some(UNIX_EPOCH + Duration::from_millis(1_700_000_300_000))
    );
}

#[test]
fn presence_status_display_type_selects_the_status_field() {
    let mut store = Store::from_ready(ready());
    let mut apply = |activity: serde_json::Value| {
        let presence: model::Presence = serde_json::from_value(json!({
            "user_id": "2",
            "activities": [activity]
        }))
        .unwrap();
        store.apply(Event::PresenceUpdate(Box::new(presence)));
        let user = store.user(2).unwrap();
        (user.activity.clone(), user.activity_kind, user.playing)
    };

    let (activity, kind, playing) = apply(json!({
        "type": 2,
        "name": "YouTube Music",
        "state": "Laur",
        "details": "Colorful Universe",
        "status_display_type": 1
    }));
    assert_eq!(activity.as_deref(), Some("Laur"));
    assert_eq!(kind, Some(2));
    assert!(!playing);

    let (activity, kind, playing) = apply(json!({
        "type": 2,
        "name": "YouTube Music",
        "state": "Laur",
        "details": "Colorful Universe",
        "status_display_type": 2
    }));
    assert_eq!(activity.as_deref(), Some("Colorful Universe"));
    assert_eq!(kind, Some(2));
    assert!(!playing);

    let (activity, kind, playing) = apply(json!({
        "type": 2,
        "name": "YouTube Music"
    }));
    assert_eq!(activity.as_deref(), Some("Listening to YouTube Music"));
    assert_eq!(kind, Some(2));
    assert!(!playing);

    let (activity, kind, playing) = apply(json!({
        "type": 0,
        "name": "Example Game"
    }));
    assert_eq!(activity.as_deref(), Some("Example Game"));
    assert_eq!(kind, Some(0));
    assert!(playing);

    let (activity, kind, playing) = apply(json!({
        "type": 4,
        "state": "custom status"
    }));
    assert_eq!(activity.as_deref(), Some("custom status"));
    assert_eq!(kind, None);
    assert!(!playing);
}

#[test]
fn presence_game_activity_maps_rich_assets_and_text() {
    let mut store = Store::from_ready(ready());
    let mut apply = |activity: serde_json::Value| {
        let presence: model::Presence = serde_json::from_value(json!({
            "user_id": "2",
            "activities": [activity]
        }))
        .unwrap();
        store.apply(Event::PresenceUpdate(Box::new(presence)));
        store.user(2).unwrap().game.clone().unwrap()
    };

    let game = apply(json!({
        "type": 0,
        "name": "Visual Studio Code",
        "application_id": "123",
        "details": "  Editing quick_switcher.rs  ",
        "state": "   ",
        "assets": {
            "large_image": "456",
            "large_text": "Example Game",
            "small_image": "789",
            "small_text": "VS Code"
        }
    }));
    assert_eq!(game.details.as_deref(), Some("Editing quick_switcher.rs"));
    assert!(game.state.is_none());
    let large = game.large_image.unwrap();
    assert_eq!(
        large.url,
        "https://cdn.discordapp.com/app-assets/123/456.png"
    );
    assert_eq!(large.text.as_deref(), Some("Example Game"));
    let small = game.small_image.unwrap();
    assert_eq!(
        small.url,
        "https://cdn.discordapp.com/app-assets/123/789.png"
    );
    assert_eq!(small.text.as_deref(), Some("VS Code"));

    let game = apply(json!({
        "type": 0,
        "name": "Code",
        "details": "   ",
        "assets": {"large_image": "456"}
    }));
    assert!(game.details.is_none());
    assert!(game.large_image.is_none());

    let game = apply(json!({
        "type": 0,
        "name": "Code",
        "assets": {"large_image": "mp:external/abc/https/x.png"}
    }));
    assert_eq!(
        game.large_image.unwrap().url,
        "https://media.discordapp.net/external/abc/https/x.png"
    );

    let game = apply(json!({
        "type": 0,
        "name": "Code",
        "assets": {"large_image": "spotify:f00000000000002a"}
    }));
    assert_eq!(
        game.large_image.unwrap().url,
        "https://i.scdn.co/image/f00000000000002a"
    );

    let game = apply(json!({
        "type": 0,
        "name": "Code",
        "assets": {"large_image": "youtube:xyz"}
    }));
    assert!(game.large_image.is_none());

    let game = apply(json!({
        "type": 0,
        "name": "Code",
        "details": "Editing quick_switcher.rs",
        "assets": "malformed"
    }));
    assert_eq!(game.details.as_deref(), Some("Editing quick_switcher.rs"));
    assert!(game.large_image.is_none() && game.small_image.is_none());
}

#[test]
fn presence_game_activity_maps_platform_and_embedded_flag() {
    let mut store = Store::from_ready(ready());
    let mut apply = |activity: serde_json::Value| {
        let presence: model::Presence = serde_json::from_value(json!({
            "user_id": "2",
            "activities": [activity]
        }))
        .unwrap();
        store.apply(Event::PresenceUpdate(Box::new(presence)));
        store.user(2).unwrap().game.clone().unwrap()
    };

    let game = apply(json!({
        "type": 0,
        "name": "Example Puzzle",
        "application_id": "123",
        "platform": "XBOX",
        "flags": 256
    }));
    assert_eq!(game.platform.as_deref(), Some("xbox"));
    assert!(game.embedded);

    let game = apply(json!({
        "type": 0,
        "name": "Embedded",
        "platform": "embedded"
    }));
    assert_eq!(game.platform.as_deref(), Some("embedded"));
    assert!(game.embedded);

    let game = apply(json!({"type": 0, "name": "Desktop", "platform": "  ", "flags": 0}));
    assert!(game.platform.is_none());
    assert!(!game.embedded);
}

#[test]
fn presence_game_activity_falls_back_to_created_at_and_reads_the_play_flag() {
    use std::time::{Duration, UNIX_EPOCH};

    let mut store = Store::from_ready(ready());
    let mut apply = |activity: serde_json::Value| {
        let presence: model::Presence = serde_json::from_value(json!({
            "user_id": "2",
            "activities": [activity]
        }))
        .unwrap();
        store.apply(Event::PresenceUpdate(Box::new(presence)));
        store.user(2).unwrap().game.clone().unwrap()
    };

    let game = apply(json!({
        "type": 0,
        "name": "Example Idle Game",
        "application_id": "1191168989724672018",
        "created_at": 1704067200000u64,
        "flags": 288
    }));
    assert_eq!(
        game.started_at,
        Some(UNIX_EPOCH + Duration::from_millis(1_704_067_200_000))
    );
    assert!(game.playable && game.embedded);

    let game = apply(json!({
        "type": 0,
        "name": "Both",
        "created_at": 1704067200000u64,
        "timestamps": {"start": 1700000000000u64}
    }));
    assert_eq!(
        game.started_at,
        Some(UNIX_EPOCH + Duration::from_millis(1_700_000_000_000))
    );
    assert!(!game.playable && !game.embedded);
}

#[test]
fn sessions_replace_applies_the_current_users_activity() {
    let mut store = Store::from_ready(ready());
    let current = store.current_user_id;
    let sessions = model::parse_entries(json!([
        {
            "session_id": "all",
            "status": "online",
            "active": true,
            "activities": [{
                "type": 0,
                "name": "Visual Studio Code",
                "application_id": "123",
                "details": "Editing quick_switcher.rs",
                "state": "Workspace: discord [SSH: demo-host]",
                "timestamps": {"start": 1700000000000u64},
                "assets": {"large_image": "456", "large_text": "Example Game"}
            }]
        }
    ]));
    store.apply(Event::SessionsReplace(sessions));
    let game = store.user(current).unwrap().game.clone().unwrap();
    assert_eq!(game.name, "Visual Studio Code");
    assert_eq!(game.details.as_deref(), Some("Editing quick_switcher.rs"));
    assert_eq!(
        game.state.as_deref(),
        Some("Workspace: discord [SSH: demo-host]")
    );
    assert_eq!(
        game.large_image.as_ref().map(|image| image.url.as_str()),
        Some("https://cdn.discordapp.com/app-assets/123/456.png")
    );
    assert_eq!(game.application_id, Some(123));

    let sessions = model::parse_entries(json!([
        {"session_id": "a", "active": false, "activities": [
            {"type": 0, "name": "Inactive"}
        ]},
        {"session_id": "b", "active": true, "activities": [
            {"type": 0, "name": "Active"}
        ]}
    ]));
    store.apply(Event::SessionsReplace(sessions));
    assert_eq!(
        store.user(current).unwrap().game.as_ref().unwrap().name,
        "Active"
    );

    let sessions = model::parse_entries(json!([
        "not a session",
        {"session_id": "all", "activities": [{"type": 0, "name": "Recovered"}]}
    ]));
    assert_eq!(sessions.len(), 1);
    store.apply(Event::SessionsReplace(sessions));
    assert_eq!(
        store.user(current).unwrap().game.as_ref().unwrap().name,
        "Recovered"
    );

    let mut ready = ready();
    ready.sessions = model::parse_entries(json!([
        {"session_id": "all", "activities": [{
            "type": 0,
            "name": "Ready Game",
            "details": "From READY"
        }]}
    ]));
    store.apply(Event::Ready(Box::new(ready)));
    let game = store.user(current).unwrap().game.clone().unwrap();
    assert_eq!(game.name, "Ready Game");
    assert_eq!(game.details.as_deref(), Some("From READY"));
}

#[test]
fn primary_guild_tag_maps_and_survives_users_without_the_field() {
    let mut store = Store::from_ready(ready());
    let mut message = messages().remove(0);
    message.author = serde_json::from_value(json!({"id":"2","username":"writer","primary_guild":{"identity_enabled":true,"identity_guild_id":"9","tag":"BYND","badge":"flame"}})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    let tag = store.user(2).unwrap().tag.clone().unwrap();
    assert_eq!((tag.text.as_str(), tag.guild_id), ("BYND", 9));
    assert!(tag.badge_spec().is_some());
    message.author = serde_json::from_value(json!({"id":"2","username":"writer"})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    assert!(store.user(2).unwrap().tag.is_some());
    message.author = serde_json::from_value(json!({"id":"2","username":"writer","primary_guild":{"identity_enabled":false,"tag":"BYND"}})).unwrap();
    store.set_messages(2002, vec![message]);
    assert!(store.user(2).unwrap().tag.is_none());
}

#[test]
fn decorations_survive_partial_authors_and_guild_nameplates_override() {
    let mut store = Store::from_ready(ready());
    let mut message = messages().remove(0);
    message.author = serde_json::from_value(json!({
        "id": "2",
        "username": "writer",
        "avatar_decoration_data": {"asset": "a_cat", "sku_id": "7"},
        "collectibles": {"nameplate": {"asset": "nameplates/mouse/1/", "palette": "berry"}},
        "display_name_styles": {"font_id": 14, "effect_id": 8, "colors": [1, 2]}
    }))
    .unwrap();
    store.set_messages(2002, vec![message.clone()]);
    let user = store.user(2).unwrap();
    assert_eq!(user.decoration.as_ref().unwrap().asset, "a_cat");
    assert!(matches!(
        user.decoration.as_ref().unwrap().image_spec(true),
        ImageSpec::AvatarDecoration { animated: true, .. }
    ));
    assert_eq!(user.name_style.as_ref().unwrap().colors, [1, 2]);
    assert_eq!(
        store.nameplate(None, 2).unwrap().asset,
        "nameplates/mouse/1/"
    );

    message.author = serde_json::from_value(json!({"id": "2", "username": "writer"})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    message.author = serde_json::from_value(json!({"id": "2", "global_name": "W"})).unwrap();
    store.set_messages(2002, vec![message.clone()]);
    let user = store.user(2).unwrap();
    assert!(user.decoration.is_some() && user.nameplate.is_some() && user.name_style.is_some());

    member_event(
        &mut store,
        json!({
            "guild_id": "200",
            "user": {"id": "2"},
            "collectibles": {"nameplate": {"asset": "nameplates/rain/2/"}}
        }),
    );
    assert_eq!(
        store.nameplate(Some(200), 2).unwrap().asset,
        "nameplates/rain/2/"
    );
    assert_eq!(
        store.nameplate(Some(201), 2).unwrap().asset,
        "nameplates/mouse/1/"
    );
    assert_eq!(
        store.nameplate(None, 2).unwrap().asset,
        "nameplates/mouse/1/"
    );
    member_event(
        &mut store,
        json!({"guild_id": "200", "user": {"id": "2"}, "nick": "N"}),
    );
    assert_eq!(
        store.nameplate(Some(200), 2).unwrap().asset,
        "nameplates/rain/2/"
    );
    member_event(
        &mut store,
        json!({"guild_id": "200", "user": {"id": "2"}, "collectibles": null}),
    );
    assert_eq!(
        store.nameplate(Some(200), 2).unwrap().asset,
        "nameplates/mouse/1/"
    );

    message.author = serde_json::from_value(json!({
        "id": "2",
        "global_name": "W",
        "avatar_decoration_data": null,
        "collectibles": null
    }))
    .unwrap();
    store.set_messages(2002, vec![message.clone()]);
    message.author = serde_json::from_value(json!({
        "id": "2",
        "username": "writer",
        "display_name_styles": null
    }))
    .unwrap();
    store.set_messages(2002, vec![message]);
    let user = store.user(2).unwrap();
    assert!(user.decoration.is_none() && user.nameplate.is_none() && user.name_style.is_none());
    assert!(store.nameplate(Some(200), 2).is_none());
}

#[test]
fn offline_profile_applies_server_cosmetics_without_replacing_global_ones() {
    let mut store = Store::from_ready(ready());
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../discord/fixtures/profile_modal.json")).unwrap();
    value["guild_member"]["nick"] = json!("Server nickname");
    value["guild_member"]["avatar"] = json!("server-avatar");
    value["guild_member"]["avatar_decoration_data"] = json!({"asset": "a_server", "sku_id": "7"});
    value["guild_member"]["display_name_styles"] =
        json!({"font_id": 1, "colors": [0x112233, 0x445566]});
    let profile: model::Profile = serde_json::from_value(value.clone()).unwrap();
    let id = profile.user.user.id;
    store.absorb_profile(Some(200), &profile);
    assert!(store.user(id).unwrap().status == Status::Offline);
    assert_eq!(store.member_name(200, id), Some("Server nickname"));
    assert!(matches!(store.member_avatar_spec(200, id),
        Some(ImageSpec::GuildAvatar { hash, .. }) if hash == "server-avatar"));
    assert_eq!(store.member_decoration(200, id).unwrap().asset, "a_server");
    assert_eq!(
        store.member_name_style(200, id).unwrap().colors,
        [0x112233, 0x445566]
    );
    let global = store
        .user(id)
        .unwrap()
        .decoration
        .as_ref()
        .unwrap()
        .asset
        .clone();
    assert_ne!(global, "a_server");
    assert_eq!(store.member_decoration(201, id).unwrap().asset, global);
    assert_ne!(
        store.member_name_style(201, id).unwrap().colors,
        [0x112233, 0x445566]
    );

    // An unrelated partial update must not erase server-specific cosmetics.
    member_event(
        &mut store,
        json!({"guild_id": "200", "user": {"id": id.to_string()}, "nick": "Changed"}),
    );
    assert_eq!(store.member_decoration(200, id).unwrap().asset, "a_server");
    assert_eq!(
        store.member_name_style(200, id).unwrap().colors,
        [0x112233, 0x445566]
    );

    // Explicitly removing an override restores the global cosmetics.
    value["guild_member"]["avatar"] = serde_json::Value::Null;
    value["guild_member"]["avatar_decoration_data"] = serde_json::Value::Null;
    value["guild_member"]["display_name_styles"] = serde_json::Value::Null;
    store.absorb_profile(Some(200), &serde_json::from_value(value).unwrap());
    assert!(store.member_avatar_spec(200, id).is_none());
    assert_eq!(store.member_decoration(200, id).unwrap().asset, global);
    assert_eq!(
        store.member_name_style(200, id).unwrap().colors,
        store.user(id).unwrap().name_style.as_ref().unwrap().colors
    );
}

#[test]
fn profile_retains_nested_user_cosmetics_when_the_top_level_user_omits_them() {
    let mut store = Store::from_ready(ready());
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../discord/fixtures/profile_modal.json")).unwrap();
    for key in [
        "avatar_decoration_data",
        "collectibles",
        "display_name_styles",
    ] {
        value["user"].as_object_mut().unwrap().remove(key);
    }
    let profile: model::Profile = serde_json::from_value(value.clone()).unwrap();
    let id = profile.user.user.id;
    store.absorb_profile(Some(200), &profile);
    let user = store.user(id).unwrap();
    assert!(user.status == Status::Offline);
    assert!(user.decoration.is_some() && user.nameplate.is_some() && user.name_style.is_some());
    assert!(store.nameplate(Some(200), id).is_some());

    value["user"]["avatar_decoration_data"] = serde_json::Value::Null;
    store.absorb_profile(Some(200), &serde_json::from_value(value).unwrap());
    assert!(store.user(id).unwrap().decoration.is_none());
    assert!(store.member_decoration(200, id).is_none());
}

#[test]
fn presence_custom_status_keeps_custom_emoji_without_text() {
    let mut store = Store::from_ready(ready());
    let presence: model::Presence = serde_json::from_value(json!({
        "user_id":"2",
        "activities":[{"type":4,"emoji":{"id":"42","name":"party","animated":true}}]
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::PresenceUpdate(Box::new(presence))),
        Change::Members { guild_id: None }
    );
    let status = store.user(2).unwrap().custom_status.as_ref().unwrap();
    assert!(status.text.is_none());
    assert!(matches!(
        &status.emoji,
        Some(StatusEmoji::Custom {
            id: 42,
            name,
            animated: true
        }) if name == "party"
    ));
}

#[test]
fn user_update_merges_partial_payload_and_returns_members_change() {
    let mut store = Store::from_ready(ready());
    let avatar = store.user(1).unwrap().avatar.clone();
    let update: model::UserUpdate = serde_json::from_value(json!({
        "id":"1",
        "global_name":"Renamed",
        "avatar_decoration_data":{"asset":"a_new"}
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::UserUpdate(Box::new(update))),
        Change::Members { guild_id: None }
    );
    let user = store.user(1).unwrap();
    assert_eq!(user.username, "owner");
    assert_eq!(user.name, "Renamed");
    assert_eq!(
        user.decoration
            .as_ref()
            .map(|decoration| decoration.asset.as_str()),
        Some("a_new")
    );
    assert_eq!(user.avatar, avatar);

    let update: model::UserUpdate = serde_json::from_value(json!({
        "id":"1",
        "username":"new_owner",
        "global_name":null,
        "avatar_decoration_data":null
    }))
    .unwrap();
    store.apply(Event::UserUpdate(Box::new(update)));
    let user = store.user(1).unwrap();
    assert_eq!(user.username, "new_owner");
    assert_eq!(user.name, "new_owner");
    assert!(user.decoration.is_none());
}

#[test]
fn typing_start_registers_member_without_state_change() {
    let mut store = Store::from_ready(ready());
    let typing: model::TypingStart = serde_json::from_value(json!({
        "channel_id": "2002", "guild_id": "200", "user_id": "77",
        "timestamp": 1770000000,
        "member": {"user": {"id": "77", "username": "typer"}, "roles": []}
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::TypingStart(Box::new(typing))),
        Change::None
    );
    assert_eq!(store.member_name(200, 77), Some("typer"));
}

#[test]
fn a_custom_status_keeps_the_game_for_profiles() {
    let mut store = Store::from_ready(ready());
    let presence: model::Presence = serde_json::from_value(json!({
        "user_id": "2",
        "activities": [
            {"type": 4, "name": "Custom Status", "state": "busy"},
            {"type": 0, "name": "Visual Studio Code"}
        ]
    }))
    .unwrap();
    store.apply(Event::PresenceUpdate(Box::new(presence)));
    let user = store.user(2).unwrap();
    assert_eq!(user.activity.as_deref(), Some("busy"));
    assert!(!user.playing);
    assert_eq!(user.game.as_ref().unwrap().name, "Visual Studio Code");
}
