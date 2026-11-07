use super::*;
use crate::discord::Event;
use serde_json::{Value, json};

const NOW: u64 = 1_800_000_000_000;
const AFTER_2030: u64 = 1_900_000_000_000;

fn ready_value() -> Value {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../crates/discord/src/fixtures/ready.json"
    ))
    .unwrap();
    let guild = &mut value["guilds"][0];
    guild["members"] = json!([
        {"user":{"id":"1","username":"owner","global_name":"Local Owner"},"roles":["201"]}
    ]);
    guild["threads"] = json!([
        {"id":"3001","type":11,"guild_id":"200","parent_id":"2003","name":"thread","owner_id":"1",
         "member":{"user_id":"1","join_timestamp":"2024-01-01T00:00:00+00:00"}},
        {"id":"3002","type":11,"guild_id":"200","parent_id":"2003","name":"other","owner_id":"2"}
    ]);
    value["private_channels"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":"901","type":1,"recipient_ids":["2"],"is_message_request":true
        }));
    value
}

fn store_with(entries: Value, guild_default: u64) -> Store {
    let mut value = ready_value();
    value["guilds"][0]["properties"]["default_message_notifications"] = json!(guild_default);
    value["user_guild_settings"] = json!({"entries": entries});
    Store::from_ready(serde_json::from_value(value).unwrap())
}

fn make_store(entries: Value) -> Store {
    store_with(entries, 0)
}

fn store_default() -> Store {
    make_store(json!([]))
}

#[test]
fn guild_mute_query_checks_only_the_guild_and_expires_at_the_boundary() {
    let store = make_store(json!([
        {"guild_id": "200", "muted": true, "mute_config": {"end_time": NOW + 1000}},
        {"guild_id": "300", "muted": true},
        {"guild_id": "400", "muted": true, "mute_config": {"end_time": null}},
        {"guild_id": "500", "muted": false, "mute_config": {"end_time": NOW + 1000}},
        {"guild_id": "600", "channel_overrides": [{"channel_id": "6001", "muted": true}]},
        {"guild_id": null, "muted": true}
    ]));
    assert!(store.guild_muted(200, NOW));
    assert!(store.guild_muted(200, NOW + 999));
    assert!(!store.guild_muted(200, NOW + 1000));
    assert!(!store.guild_muted(200, NOW + 1001));
    assert!(store.guild_muted(300, NOW));
    assert!(store.guild_muted(400, NOW));
    for id in [500, 600, 700] {
        assert!(!store.guild_muted(id, NOW));
    }
}

#[test]
fn guild_notification_query_resolves_settings_then_default_then_all() {
    use model::NotificationLevel::{All, Mentions, Nothing};

    for (wire, expected) in [(0, All), (1, Mentions), (2, Nothing)] {
        let store = store_with(
            json!([{
                "guild_id": "200", "message_notifications": wire,
                "channel_overrides": [{"channel_id": "2003", "message_notifications": 2}]
            }]),
            1,
        );
        assert_eq!(store.guild_notification_level(200), expected);
    }
    for entries in [
        json!([]),
        json!([{"guild_id": "200", "message_notifications": 3}]),
    ] {
        assert_eq!(
            store_with(entries.clone(), 1).guild_notification_level(200),
            Mentions
        );
        assert_eq!(
            store_with(entries.clone(), 0).guild_notification_level(200),
            All
        );
        assert_eq!(store_with(entries, 3).guild_notification_level(200), All);
    }
    assert_eq!(store_default().guild_notification_level(999), All);
}

#[test]
fn guild_notification_flags_default_to_mobile_push_only() {
    let defaults = model::GuildNotificationFlags {
        suppress_everyone: false,
        suppress_roles: false,
        suppress_highlights: false,
        mute_scheduled_events: false,
        mobile_push: true,
    };
    assert_eq!(model::GuildNotificationFlags::default(), defaults);
    assert_eq!(store_default().guild_notification_flags(200), defaults);
    let store = make_store(json!([
        {
            "guild_id": "200",
            "channel_overrides": [{
                "channel_id": "2003", "suppress_everyone": true,
                "suppress_roles": true, "notify_highlights": 1,
                "mute_scheduled_events": true, "mobile_push": false
            }]
        },
        {
            "guild_id": null, "suppress_everyone": true,
            "suppress_roles": true, "notify_highlights": 1,
            "mute_scheduled_events": true, "mobile_push": false
        }
    ]));
    assert_eq!(store.guild_notification_flags(200), defaults);
    assert_eq!(store.guild_notification_flags(999), defaults);
}

#[test]
fn guild_notification_flags_parse_all_fields_and_highlight_levels() {
    for (level, suppressed) in [(0, false), (1, true), (2, false), (99, false)] {
        let store = make_store(json!([{
            "guild_id": "200", "suppress_everyone": true,
            "suppress_roles": true, "notify_highlights": level,
            "mute_scheduled_events": true, "mobile_push": false
        }]));
        assert_eq!(
            store.guild_notification_flags(200),
            model::GuildNotificationFlags {
                suppress_everyone: true,
                suppress_roles: true,
                suppress_highlights: suppressed,
                mute_scheduled_events: true,
                mobile_push: false,
            }
        );
    }
    for mobile_push in [Value::Null, json!("invalid")] {
        let store = make_store(json!([{
            "guild_id": "200", "suppress_everyone": null,
            "suppress_roles": null, "notify_highlights": null,
            "mute_scheduled_events": null, "mobile_push": mobile_push
        }]));
        assert_eq!(
            store.guild_notification_flags(200),
            model::GuildNotificationFlags::default()
        );
    }
}

#[test]
fn guild_notification_flags_follow_gateway_settings_replacement() {
    let mut store = make_store(json!([
        {"guild_id": "200"},
        {"guild_id": "300", "suppress_roles": true},
        {"guild_id": null, "channel_overrides": [{"channel_id": "900", "muted": true}]}
    ]));
    let other = store.guild_notification_flags(300);
    let update = serde_json::from_value(json!({
        "guild_id": "200", "suppress_everyone": true,
        "suppress_roles": true, "notify_highlights": 1,
        "mute_scheduled_events": true, "mobile_push": false
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::UserGuildSettingsUpdate(Box::new(update))),
        Change::Notifications
    );
    assert_eq!(
        store.guild_notification_flags(200),
        model::GuildNotificationFlags {
            suppress_everyone: true,
            suppress_roles: true,
            suppress_highlights: true,
            mute_scheduled_events: true,
            mobile_push: false,
        }
    );
    let update = serde_json::from_value(json!({
        "guild_id": "200", "suppress_everyone": false,
        "suppress_roles": false, "notify_highlights": 2,
        "mute_scheduled_events": false, "mobile_push": true
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::UserGuildSettingsUpdate(Box::new(update))),
        Change::Notifications
    );
    assert_eq!(
        store.guild_notification_flags(200),
        model::GuildNotificationFlags::default()
    );
    assert_eq!(store.guild_notification_flags(300), other);
    assert!(store.dm_muted(900, NOW));
}

fn message_draft(channel_id: u64, author_id: u64, content: &str) -> Value {
    json!({
        "id":"5000",
        "channel_id":channel_id.to_string(),
        "author":{"id":author_id.to_string(),"username":"writer","global_name":"A Writer"},
        "timestamp":"2026-10-22T00:00:00Z",
        "content":content
    })
}

fn message(channel_id: u64, author_id: u64, content: &str) -> model::Message {
    serde_json::from_value(message_draft(channel_id, author_id, content)).unwrap()
}

fn mention_user(message: &mut model::Message, user_id: u64) {
    message
        .mentions
        .push(serde_json::from_value(json!({"id":user_id.to_string()})).unwrap());
}

#[test]
fn ready_reads_plain_and_entries_user_guild_settings_with_defaults() {
    let end = chrono::DateTime::parse_from_rfc3339("2030-01-01T00:00:00.000000+00:00")
        .unwrap()
        .timestamp_millis() as u64;
    let entries = json!([
        {
            "guild_id":"200",
            "muted":true,
            "mute_config":{"end_time":"2030-01-01T00:00:00.000000+00:00"},
            "message_notifications":2,
            "suppress_everyone":true,
            "suppress_roles":true,
            "notify_highlights":1,
            "mute_scheduled_events":true,
            "mobile_push":false,
            "channel_overrides":[{
                "channel_id":"2003","muted":true,"message_notifications":0
            }]
        },
        {"guild_id":null,"message_notifications":0},
        {"guild_id":"201"}
    ]);
    for payload in [entries.clone(), json!({"entries": entries.clone()})] {
        let mut value = ready_value();
        value["user_guild_settings"] = payload;
        let ready: model::Ready = serde_json::from_value(value).unwrap();
        assert_eq!(ready.user_guild_settings.len(), 3);
        let guild = &ready.user_guild_settings[0];
        assert_eq!(guild.guild_id, Some(200));
        assert!(guild.muted);
        assert_eq!(guild.mute_config.as_ref().unwrap().end_time, Some(end));
        assert_eq!(
            guild.message_notifications,
            model::NotificationLevel::Nothing
        );
        assert!(guild.suppress_everyone);
        assert!(guild.suppress_roles);
        assert_eq!(guild.notify_highlights, 1);
        assert!(guild.mute_scheduled_events);
        assert_eq!(guild.mobile_push, Some(false));
        assert_eq!(guild.channel_overrides[0].channel_id, Some(2003));
        assert!(guild.channel_overrides[0].muted);
        assert_eq!(
            guild.channel_overrides[0].message_notifications,
            model::NotificationLevel::All
        );
        let dm = &ready.user_guild_settings[1];
        assert_eq!(dm.guild_id, None);
        assert_eq!(dm.message_notifications, model::NotificationLevel::All);
        let sparse = &ready.user_guild_settings[2];
        assert_eq!(sparse.guild_id, Some(201));
        assert_eq!(
            sparse.message_notifications,
            model::NotificationLevel::Inherit
        );
        assert!(!sparse.muted);
        assert!(sparse.channel_overrides.is_empty());
        assert_eq!(
            ready.guild_notification_defaults.get(&200),
            Some(&model::NotificationLevel::All)
        );
    }
}

#[test]
fn dms_alert_with_author_and_flattened_body() {
    let store = store_default();
    let alert = store
        .message_alert(&message(900, 2, "**hello**"), NOW)
        .expect("DM alerts");
    assert_eq!(alert.guild_id, None);
    assert_eq!(alert.channel_id, 900);
    assert_eq!(alert.message_id, 5000);
    assert_eq!(alert.title, "A Writer");
    assert_eq!(alert.body, "hello");
}

#[test]
fn dm_mutes_expire_and_message_requests_are_silent() {
    let store = make_store(json!([{
        "guild_id":null,
        "channel_overrides":[{
            "channel_id":"900","muted":true,
            "mute_config":{"end_time":"2030-01-01T00:00:00Z"}
        }]
    }]));
    assert!(
        store
            .message_alert(&message(900, 2, "hello"), NOW)
            .is_none()
    );
    assert!(
        store
            .message_alert(&message(900, 2, "hello"), AFTER_2030)
            .is_some()
    );

    let expired = make_store(json!([{
        "guild_id":null,
        "channel_overrides":[{
            "channel_id":"900","muted":true,
            "mute_config":{"end_time":"2020-01-01T00:00:00Z"}
        }]
    }]));
    assert!(
        expired
            .message_alert(&message(900, 2, "hello"), NOW)
            .is_some()
    );

    let indefinite = make_store(json!([{
        "guild_id":null,
        "channel_overrides":[{"channel_id":"900","muted":true}]
    }]));
    assert!(
        indefinite
            .message_alert(&message(900, 2, "hello"), AFTER_2030)
            .is_none()
    );
    assert!(
        store_default()
            .message_alert(&message(901, 2, "request"), NOW)
            .is_none()
    );
}

#[test]
fn dnd_blocked_own_unknown_and_unviewable_messages_are_silent() {
    let mut store = store_default();
    store.set_self_status(crate::discord::SelfStatus::DoNotDisturb);
    assert!(
        store
            .message_alert(&message(900, 2, "hello"), NOW)
            .is_none()
    );
    store.set_self_status(crate::discord::SelfStatus::Online);
    assert!(
        store
            .message_alert(&message(900, 2, "hello"), NOW)
            .is_some()
    );

    let mut blocked = store_default();
    let relationship: model::Relationship = serde_json::from_value(json!({
        "id":"2","type":2,"user":{"id":"2","username":"writer"}
    }))
    .unwrap();
    blocked.apply(Event::RelationshipAdd(Box::new(relationship)));
    assert!(
        blocked
            .message_alert(&message(900, 2, "hello"), NOW)
            .is_none()
    );

    assert!(
        store_default()
            .message_alert(&message(900, 1, "mine"), NOW)
            .is_none()
    );
    assert!(
        store_default()
            .message_alert(&message(9999, 2, "unknown"), NOW)
            .is_none()
    );
    assert!(
        store_default()
            .message_alert(&message(2001, 2, "hidden"), NOW)
            .is_none()
    );
}

#[test]
fn server_levels_and_mentions_follow_the_settings() {
    assert!(
        store_default()
            .message_alert(&message(2003, 2, "plain"), NOW)
            .is_some()
    );
    let store = make_store(json!([{"guild_id":"200","message_notifications":1}]));
    assert!(
        store
            .message_alert(&message(2003, 2, "plain"), NOW)
            .is_none()
    );
    let mut direct = message(2003, 2, "hello");
    mention_user(&mut direct, 1);
    let alert = store.message_alert(&direct, NOW).expect("mention alerts");
    assert_eq!(alert.guild_id, Some(200));
    assert_eq!(alert.title, "A Writer (#first, Earlier)");
    assert_eq!(alert.body, "hello");

    let mut everyone = message(2003, 2, "everyone");
    everyone.mention_everyone = true;
    assert!(store.message_alert(&everyone, NOW).is_some());

    let nothing = make_store(json!([{"guild_id":"200","message_notifications":2}]));
    assert!(nothing.message_alert(&direct, NOW).is_none());
}

#[test]
fn channel_category_and_parent_levels_inherit_in_order() {
    let channel = make_store(json!([{
        "guild_id":"200","message_notifications":1,
        "channel_overrides":[{"channel_id":"2003","message_notifications":0}]
    }]));
    assert!(
        channel
            .message_alert(&message(2003, 2, "plain"), NOW)
            .is_some()
    );

    let category = make_store(json!([{
        "guild_id":"200","message_notifications":1,
        "channel_overrides":[{"channel_id":"220","message_notifications":0}]
    }]));
    assert!(
        category
            .message_alert(&message(2003, 2, "plain"), NOW)
            .is_some()
    );

    let inherited = make_store(json!([{
        "guild_id":"200","message_notifications":1,
        "channel_overrides":[
            {"channel_id":"2003","message_notifications":3},
            {"channel_id":"220","message_notifications":3}
        ]
    }]));
    assert!(
        inherited
            .message_alert(&message(2003, 2, "plain"), NOW)
            .is_none()
    );

    let category_over_server = make_store(json!([{
        "guild_id":"200","message_notifications":1,
        "channel_overrides":[
            {"channel_id":"2003","message_notifications":3},
            {"channel_id":"220","message_notifications":0}
        ]
    }]));
    assert!(
        category_over_server
            .message_alert(&message(2003, 2, "plain"), NOW)
            .is_some()
    );

    let default_mentions = store_with(json!([{"guild_id":"200","message_notifications":3}]), 1);
    assert!(
        default_mentions
            .message_alert(&message(2003, 2, "plain"), NOW)
            .is_none()
    );
    let mut direct = message(2003, 2, "hello");
    mention_user(&mut direct, 1);
    assert!(default_mentions.message_alert(&direct, NOW).is_some());

    let parent = make_store(json!([{
        "guild_id":"200","message_notifications":1,
        "channel_overrides":[{"channel_id":"2003","message_notifications":0}]
    }]));
    assert!(
        parent
            .message_alert(&message(3001, 2, "plain"), NOW)
            .is_some()
    );
}

#[test]
fn suppress_everyone_and_roles_keep_direct_mentions() {
    let everyone = make_store(json!([{
        "guild_id":"200","message_notifications":1,"suppress_everyone":true
    }]));
    let mut broadcast = message(2003, 2, "everyone");
    broadcast.mention_everyone = true;
    assert!(everyone.message_alert(&broadcast, NOW).is_none());
    let mut direct = message(2003, 2, "hello");
    mention_user(&mut direct, 1);
    assert!(everyone.message_alert(&direct, NOW).is_some());

    let roles = make_store(json!([{
        "guild_id":"200","message_notifications":1,"suppress_roles":true
    }]));
    let mut role = message(2003, 2, "role");
    role.mention_roles.push(201);
    assert!(roles.message_alert(&role, NOW).is_none());
    assert!(roles.message_alert(&direct, NOW).is_some());

    let unsuppressed = make_store(json!([{"guild_id":"200","message_notifications":1}]));
    assert!(unsuppressed.message_alert(&role, NOW).is_some());
}

#[test]
fn server_channel_category_and_thread_parent_mutes_respect_expiry() {
    let server = make_store(json!([{
        "guild_id":"200","message_notifications":0,"muted":true,
        "mute_config":{"end_time":"2030-01-01T00:00:00Z"}
    }]));
    assert!(
        server
            .message_alert(&message(2003, 2, "hello"), NOW)
            .is_none()
    );
    assert!(
        server
            .message_alert(&message(2003, 2, "hello"), AFTER_2030)
            .is_some()
    );

    let expired = make_store(json!([{
        "guild_id":"200","message_notifications":0,"muted":true,
        "mute_config":{"end_time":"2020-01-01T00:00:00Z"}
    }]));
    assert!(
        expired
            .message_alert(&message(2003, 2, "hello"), NOW)
            .is_some()
    );

    let channel = make_store(json!([{
        "guild_id":"200","message_notifications":0,
        "channel_overrides":[{
            "channel_id":"2003","muted":true,
            "mute_config":{"end_time":"2030-01-01T00:00:00Z"}
        }]
    }]));
    assert!(
        channel
            .message_alert(&message(2003, 2, "hello"), NOW)
            .is_none()
    );

    let category = make_store(json!([{
        "guild_id":"200","message_notifications":0,
        "channel_overrides":[{
            "channel_id":"220","muted":true,
            "mute_config":{"end_time":"2030-01-01T00:00:00Z"}
        }]
    }]));
    assert!(
        category
            .message_alert(&message(2003, 2, "hello"), NOW)
            .is_none()
    );

    let parent = make_store(json!([{
        "guild_id":"200","message_notifications":0,
        "channel_overrides":[{
            "channel_id":"2003","muted":true,
            "mute_config":{"end_time":"2030-01-01T00:00:00Z"}
        }]
    }]));
    assert!(
        parent
            .message_alert(&message(3001, 2, "hello"), NOW)
            .is_none()
    );
}

#[test]
fn attachment_empty_and_system_messages_choose_the_body() {
    let store = store_default();
    let mut attachment = message_draft(900, 2, "");
    attachment["attachments"] = json!([{"id":"1","filename":"image-44.png"},{"id":"2"}]);
    let alert = store
        .message_alert(&serde_json::from_value(attachment.clone()).unwrap(), NOW)
        .expect("attachment alerts");
    assert_eq!(alert.body, "Uploaded image-44.png");
    attachment["attachments"] = json!([{"id":"1"}]);
    let unnamed = store
        .message_alert(&serde_json::from_value(attachment).unwrap(), NOW)
        .expect("attachment alerts");
    assert_eq!(unnamed.body, "Uploaded a file");

    let empty = store
        .message_alert(&message(900, 2, ""), NOW)
        .expect("empty normal message alerts");
    assert!(empty.body.is_empty());
    let mut system = message_draft(900, 2, "");
    system["type"] = json!(7);
    assert!(
        store
            .message_alert(&serde_json::from_value(system).unwrap(), NOW)
            .is_none()
    );
    let mut reply = message_draft(900, 2, "");
    reply["type"] = json!(19);
    assert!(
        store
            .message_alert(&serde_json::from_value(reply).unwrap(), NOW)
            .is_some()
    );

    let long = store
        .message_alert(&message(900, 2, &"a".repeat(201)), NOW)
        .unwrap();
    assert_eq!(long.body.len(), 200);

    let boundary = message(900, 2, &format!("{}é", "a".repeat(199)));
    let boundary = store.message_alert(&boundary, NOW).unwrap();
    assert_eq!(boundary.body, "a".repeat(199));
}

#[test]
fn user_guild_settings_update_replaces_one_entry() {
    let mut store = make_store(json!([
        {"guild_id":"200","message_notifications":0},
        {"guild_id":"300","message_notifications":1,"muted":true},
        {"guild_id":null,"channel_overrides":[{"channel_id":"900","muted":true}]}
    ]));
    assert!(
        store
            .message_alert(&message(2003, 2, "hello"), NOW)
            .is_some()
    );
    let update: model::UserGuildSettings = serde_json::from_value(json!({
        "guild_id":"200","message_notifications":2,"muted":true,
        "mute_config":{"end_time":NOW + 1000}
    }))
    .unwrap();
    assert_eq!(
        store.apply(Event::UserGuildSettingsUpdate(Box::new(update))),
        Change::Notifications
    );
    assert_eq!(
        store.guild_notification_level(200),
        model::NotificationLevel::Nothing
    );
    assert!(store.guild_muted(200, NOW));
    assert!(!store.guild_muted(200, NOW + 1000));
    assert_eq!(
        store.guild_notification_level(300),
        model::NotificationLevel::Mentions
    );
    assert!(store.guild_muted(300, NOW));
    assert!(store.dm_muted(900, NOW));
    assert!(
        store
            .message_alert(&message(2003, 2, "hello"), NOW)
            .is_none()
    );
    let unmute = serde_json::from_value(json!({
        "guild_id":"200","message_notifications":0,"muted":false
    }))
    .unwrap();
    store.apply(Event::UserGuildSettingsUpdate(Box::new(unmute)));
    assert!(!store.guild_muted(200, NOW));
    assert_eq!(
        store.guild_notification_level(200),
        model::NotificationLevel::All
    );
    assert!(
        store
            .message_alert(&message(2003, 2, "hello"), NOW)
            .is_some()
    );
}

#[test]
fn threads_you_have_not_joined_only_alert_on_mentions() {
    let store = store_default();
    let other = store.current_user_id + 1;
    let now_ms = 1_800_000_000_000;
    assert!(
        store
            .message_alert(&message(3001, other, "joined"), now_ms)
            .is_some()
    );
    let mut unjoined = message(3002, other, "unjoined");
    assert!(store.message_alert(&unjoined, now_ms).is_none());
    mention_user(&mut unjoined, store.current_user_id);
    assert!(store.message_alert(&unjoined, now_ms).is_some());
}

#[test]
fn channel_overrides_also_parse_as_an_object() {
    let settings: model::UserGuildSettings = serde_json::from_value(json!({
        "guild_id": "200",
        "channel_overrides": {"2001": {"channel_id": "2001", "muted": true}}
    }))
    .unwrap();
    assert_eq!(settings.channel_overrides[0].channel_id, Some(2001));
    assert!(settings.channel_overrides[0].muted);
}

#[test]
fn registering_an_author_keeps_their_presence_details() {
    let mut store = store_default();
    let user: model::User =
        serde_json::from_value(json!({"id": "2", "username": "writer"})).unwrap();
    store.users.get_mut(&2).unwrap().on_mobile = true;
    store.register_author(&user);
    assert!(store.user(2).unwrap().on_mobile);
}

#[test]
fn inbox_unreads_skip_muted_servers_categories_and_channels() {
    let ids = |store: &Store| -> Vec<u64> {
        store
            .inbox_unreads(0)
            .into_iter()
            .map(|unread| unread.channel_id)
            .collect()
    };
    let store = make_store(json!([]));
    assert_eq!(ids(&store), [2002]);
    assert_eq!(store.inbox_unreads(0)[0].last_read, 50);

    for entries in [
        json!([{"guild_id": "200", "muted": true}]),
        json!([{"guild_id": "200", "channel_overrides": [{"channel_id": "2002", "muted": true}]}]),
        json!([{"guild_id": "200", "channel_overrides": [{"channel_id": "220", "muted": true}]}]),
    ] {
        assert!(ids(&make_store(entries)).is_empty());
    }

    let store = make_store(json!([{"guild_id": "200", "channel_overrides": [
        {"channel_id": "2002", "message_notifications": 1}
    ]}]));
    assert_eq!(ids(&store), [2002]);
    assert_eq!(
        store.channel_notification_level(200, 2002),
        model::NotificationLevel::Mentions
    );
    assert_eq!(
        store.channel_notification_level(200, 2003),
        model::NotificationLevel::Inherit
    );
}
