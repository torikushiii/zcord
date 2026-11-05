use super::*;
use serde_json::{Value, json};

fn ready_with_threads(threads: Vec<Value>) -> model::Ready {
    ready_with_permissions(threads, 274_877_910_016)
}

fn ready_with_permissions(threads: Vec<Value>, permissions: u64) -> model::Ready {
    let mut payload: Value = serde_json::from_str(include_str!(
        "../../../crates/discord/src/fixtures/ready.json"
    ))
    .unwrap();
    let guild = &mut payload["guilds"][0];
    for role in guild["roles"].as_array_mut().unwrap() {
        role["permissions"] = json!(permissions.to_string());
    }
    guild["threads"] = json!(threads);
    serde_json::from_value(payload).unwrap()
}

fn now_snowflake() -> u64 {
    snowflake_at(chrono::Utc::now().timestamp_millis())
}

fn snowflake_at(ms: i64) -> u64 {
    ((ms - 1_420_070_400_000).max(0) as u64) << 22
}

fn with_archive(mut thread: Value, minutes: u32, last_message_id: u64) -> Value {
    thread["thread_metadata"]["auto_archive_duration"] = json!(minutes);
    thread["last_message_id"] = json!(last_message_id.to_string());
    thread
}

fn thread(id: &str, parent: &str, archived: bool, archive: &str) -> Value {
    json!({
        "id": id,
        "type": 11,
        "guild_id": "200",
        "parent_id": parent,
        "name": format!("thread-{id}"),
        "owner_id": "1",
        "member_count": 3,
        "member_ids_preview": ["1", "2"],
        "thread_metadata": {
            "archived": archived,
            "archive_timestamp": archive
        }
    })
}

fn message(id: &str, channel: &str, timestamp: &str, content: &str) -> Value {
    json!({
        "id": id,
        "channel_id": channel,
        "author": {"id": "7", "username": "author"},
        "timestamp": timestamp,
        "content": content
    })
}

#[test]
fn pin_timestamp_transitions_and_ack() {
    let mut ready = ready();
    let channel = ready.guilds[0]
        .channels
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|channel| channel.id == 2002)
        .unwrap();
    channel.last_pin_timestamp = Some("2026-09-10T12:00:00Z".into());
    for state in &mut ready.read_state.entries {
        if state.id == 2002 {
            state.last_pin_timestamp = Some("2026-09-10T12:00:00Z".into());
        }
    }
    let mut store = Store::from_ready(ready);
    assert!(!store.pins_unread(2002));

    store.source.as_mut().unwrap().guilds[0]
        .channels
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|channel| channel.id == 2002)
        .unwrap()
        .last_pin_timestamp = Some("2026-09-11T12:00:00Z".into());
    assert!(store.pins_unread(2002));
    store.mark_pins_read(2002);
    assert!(!store.pins_unread(2002));
    assert_eq!(
        store
            .source
            .as_ref()
            .unwrap()
            .read_state
            .entries
            .iter()
            .find(|state| state.id == 2002)
            .unwrap()
            .last_pin_timestamp
            .as_deref(),
        Some("2026-09-11T12:00:00Z")
    );
}

#[test]
fn active_threads_are_newest_first_and_skip_archived() {
    let ready = ready_with_threads(vec![
        thread("3001", "2002", false, "2026-09-10T10:00:00Z"),
        thread("3002", "2002", true, "2026-09-10T12:00:00Z"),
        thread("3003", "2002", false, "2026-09-10T09:00:00Z"),
        thread("3004", "2003", false, "2026-09-10T11:00:00Z"),
    ]);
    let mut store = Store::from_ready(ready);
    let sync: model::ThreadListSync = serde_json::from_value(json!({
        "guild_id": "200",
        "threads": [thread("3001", "2002", false, "2026-09-10T10:00:00Z")],
        "most_recent_messages": [
            message("5001", "3001", "2026-09-10T09:30:00Z", "first"),
            message("5002", "3001", "2026-09-10T11:00:00Z", "second")
        ],
        "members": []
    }))
    .unwrap();
    store.apply(Event::ThreadListSync(Box::new(sync)));

    let rows = store.active_threads(2002);
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![3001, 3003]
    );
    assert_eq!(rows[0].recent.as_ref().unwrap().1, "second");
    assert!(rows[0].joined);
    assert_eq!(rows[0].member_count, 3);
    assert_eq!(rows[0].member_ids, vec![1, 2]);
}

#[test]
fn remember_thread_adds_a_selectable_channel() {
    let mut store = Store::from_ready(ready());
    let thread: model::Channel =
        serde_json::from_value(thread("3009", "2002", true, "2026-09-10T12:00:00Z")).unwrap();
    assert!(store.guild_by_id(200).unwrap().channel(3009).is_none());
    store.remember_thread(thread);
    assert!(store.guild_by_id(200).unwrap().channel(3009).is_some());
    assert!(store.active_threads(2002).is_empty());
}

#[test]
fn listed_message_maps_users_without_entering_channel_history() {
    let mut store = Store::from_ready(ready());
    let message: model::Message = serde_json::from_value(message(
        "7001",
        "2002",
        "2026-09-10T12:00:00Z",
        "**listed**",
    ))
    .unwrap();
    let mapped = store.map_listed_message(200, message);
    assert_eq!(mapped.id, 7001);
    assert_eq!(mapped.content, "**listed**");
    assert!(store.user(7).is_some());
    assert!(
        store
            .network_channel(2002)
            .is_none_or(|channel| channel.messages.is_empty())
    );
}

#[test]
fn thread_member_events_update_join_state_and_count() {
    let mut store = Store::from_ready(ready_with_threads(vec![thread(
        "3001",
        "2002",
        false,
        "2026-09-10T12:00:00Z",
    )]));
    let update: model::ThreadMembersUpdate = serde_json::from_value(json!({
        "id": "3001",
        "guild_id": "200",
        "member_count": 5,
        "added_members": [{"id": "3001", "user_id": "1"}],
        "removed_member_ids": []
    }))
    .unwrap();
    store.apply(Event::ThreadMembersUpdate(Box::new(update)));
    let row = &store.active_threads(2002)[0];
    assert!(row.joined);
    assert_eq!(row.member_count, 5);

    let update: model::ThreadMembersUpdate = serde_json::from_value(json!({
        "id": "3001",
        "guild_id": "200",
        "member_count": 4,
        "added_members": [],
        "removed_member_ids": ["1"]
    }))
    .unwrap();
    store.apply(Event::ThreadMembersUpdate(Box::new(update)));
    let row = &store.active_threads(2002)[0];
    assert!(!row.joined);
    assert_eq!(row.member_count, 4);

    let update: model::ThreadMemberUpdate = serde_json::from_value(json!({
        "id": "3001",
        "guild_id": "200",
        "user_id": "1",
        "member_count": 5
    }))
    .unwrap();
    store.apply(Event::ThreadMemberUpdate(Box::new(update)));
    assert!(store.active_threads(2002)[0].joined);
}

#[test]
fn channel_pins_update_changes_unread_state() {
    let mut store = Store::from_ready(ready());
    let update: model::ChannelPinsUpdate = serde_json::from_value(json!({
        "guild_id": "200",
        "channel_id": "2002",
        "last_pin_timestamp": "2026-09-10T12:00:00Z"
    }))
    .unwrap();
    store.apply(Event::ChannelPinsUpdate(update));
    assert!(store.pins_unread(2002));
    store.mark_pins_read(2002);
    assert!(!store.pins_unread(2002));
}

#[test]
fn sidebar_threads_filters_quiet_archived_and_keeps_selected() {
    let recent = now_snowflake();
    let old = snowflake_at(chrono::Utc::now().timestamp_millis() - 3 * 60 * 60 * 1000);
    let mut selected = thread("3003", "2002", false, "2026-09-10T12:00:00Z");
    selected["member_ids_preview"] = json!([]);
    let threads = vec![
        with_archive(
            thread("3001", "2002", false, "2026-09-10T12:00:00Z"),
            60,
            recent,
        ),
        thread("3002", "2002", true, "2026-09-10T12:00:00Z"),
        selected,
        with_archive(
            thread("3004", "2002", false, "2026-09-10T12:00:00Z"),
            60,
            old,
        ),
        with_archive(
            thread("3005", "2002", false, "2026-09-10T12:00:00Z"),
            60,
            recent,
        ),
    ];
    let mut store = Store::from_ready(ready_with_threads(threads));
    let sync: model::ThreadListSync = serde_json::from_value(json!({
        "guild_id": "200",
        "threads": [],
        "most_recent_messages": [],
        "members": [
            {"id": "3001", "user_id": "1", "join_timestamp": "2026-09-01T12:00:00Z"},
            {"id": "3005", "user_id": "1", "join_timestamp": "2026-09-02T12:00:00Z"}
        ]
    }))
    .unwrap();
    store.apply(Event::ThreadListSync(Box::new(sync)));

    let rows = store.sidebar_threads(2002, 3003);
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![3003, 3005, 3001]
    );
    assert!(!rows[0].joined);
    assert!(rows[1].joined && rows[2].joined);
    assert!(rows.iter().all(|row| !row.archived));
}

#[test]
fn message_thread_chip_follows_updates_and_delete() {
    let recent = now_snowflake();
    let mut source = with_archive(
        thread("3001", "2002", false, "2026-09-10T12:00:00Z"),
        60,
        recent,
    );
    source["last_message_id"] = json!("5000");
    source["name"] = json!("old name");
    source["message_count"] = json!(2);
    let mut store = Store::from_ready(ready_with_threads(vec![source.clone()]));
    store.set_messages(2002, Vec::new());
    let mut source_for_message = source.clone();
    source_for_message["name"] = json!("old name");
    source_for_message["message_count"] = json!(2);
    let message: model::Message = serde_json::from_value(json!({
        "id": "5100",
        "channel_id": "2002",
        "author": {"id": "7", "username": "author"},
        "timestamp": "2026-09-10T12:00:00Z",
        "content": "starter",
        "thread": source_for_message
    }))
    .unwrap();
    store.apply(Event::MessageCreate(Box::new(message)));
    let chip = store.network_channel(2002).unwrap().messages[0]
        .thread
        .as_ref()
        .unwrap();
    assert_eq!(chip.name, "old name");
    assert_eq!(chip.message_count, 2);

    let update: model::MessageUpdate = serde_json::from_value(json!({
        "id": "5100", "channel_id": "2002", "content": "edited"
    }))
    .unwrap();
    store.apply(Event::MessageUpdate(Box::new(update)));
    assert_eq!(
        store.network_channel(2002).unwrap().messages[0]
            .thread
            .as_ref()
            .unwrap()
            .name,
        "old name"
    );

    let mut renamed = source.clone();
    renamed["name"] = json!("new name");
    renamed["message_count"] = json!(3);
    let channel: model::Channel = serde_json::from_value(renamed).unwrap();
    store.apply(Event::ChannelUpsert(Box::new(channel)));
    assert_eq!(
        store.network_channel(2002).unwrap().messages[0]
            .thread
            .as_ref()
            .unwrap()
            .name,
        "new name"
    );
    assert_eq!(
        store.network_channel(2002).unwrap().messages[0]
            .thread
            .as_ref()
            .unwrap()
            .message_count,
        3
    );

    let reply: model::Message = serde_json::from_value(json!({
        "id": "5200",
        "channel_id": "3001",
        "guild_id": "200",
        "author": {"id": "8", "username": "reply"},
        "timestamp": "2026-09-10T12:01:00Z",
        "content": "reply"
    }))
    .unwrap();
    store.apply(Event::MessageCreate(Box::new(reply)));
    assert_eq!(
        store.network_channel(2002).unwrap().messages[0]
            .thread
            .as_ref()
            .unwrap()
            .message_count,
        4
    );

    store.apply(Event::ChannelDelete { id: 3001 });
    assert!(
        store.network_channel(2002).unwrap().messages[0]
            .thread
            .is_none()
    );
}

#[test]
fn can_send_threads_uses_thread_permissions_and_locking() {
    let mut unlocked = thread("3001", "2002", false, "2026-09-10T12:00:00Z");
    unlocked["thread_metadata"]["locked"] = json!(false);
    let mut locked = thread("3002", "2002", false, "2026-09-10T12:00:00Z");
    locked["thread_metadata"]["locked"] = json!(true);
    let mut archived = thread("3003", "2002", true, "2026-09-10T12:00:00Z");
    archived["thread_metadata"]["locked"] = json!(false);

    let store = Store::from_ready(ready_with_threads(vec![
        unlocked.clone(),
        locked.clone(),
        archived.clone(),
    ]));
    assert!(store.can_send(3001));
    assert!(!store.can_send(3002));
    assert!(store.can_send(3003));

    let store = Store::from_ready(ready_with_permissions(
        vec![unlocked, locked, archived],
        274_877_910_016 | (1 << 34),
    ));
    assert!(store.can_send(3002));
}

#[test]
fn thread_creation_and_management_permissions() {
    let mut store = Store::from_ready(ready_with_permissions(
        vec![thread("3001", "2002", false, "2026-09-10T12:00:00Z")],
        274_877_910_016,
    ));
    assert!(!store.can_create_thread(2002, false));
    assert!(!store.can_create_thread(2002, true));
    assert!(store.can_manage_thread(3001));

    store.source.as_mut().unwrap().guilds[0]
        .threads
        .as_mut()
        .unwrap()
        .iter_mut()
        .find(|thread| thread.id == 3001)
        .unwrap()
        .owner_id = Some(99);
    assert!(!store.can_manage_thread(3001));

    let mut store = Store::from_ready(ready_with_permissions(
        vec![thread("3001", "2002", false, "2026-09-10T12:00:00Z")],
        274_877_910_016 | (1 << 35) | (1 << 34),
    ));
    assert!(store.can_create_thread(2002, false));
    assert!(!store.can_create_thread(2002, true));
    assert!(store.can_manage_thread(3001));
}

#[test]
fn thread_row_exposes_locked_state() {
    let mut locked = thread("3001", "2002", false, "2026-09-10T12:00:00Z");
    locked["thread_metadata"]["locked"] = json!(true);
    let store = Store::from_ready(ready_with_threads(vec![locked]));
    let row = store.thread_row_by_id(3001).unwrap();
    assert!(row.locked);
    assert!(!store.can_send(3001));
}

#[test]
fn thread_created_notice_points_at_its_thread() {
    let mut source = with_archive(
        thread("3001", "2002", false, "2026-09-10T12:00:00Z"),
        60,
        now_snowflake(),
    );
    source["name"] = json!("Plans");
    source["message_count"] = json!(3);
    let mut store = Store::from_ready(ready_with_threads(vec![source]));
    store.set_messages(2002, Vec::new());
    let message: model::Message = serde_json::from_value(json!({
        "id": "5100",
        "channel_id": "2002",
        "type": 18,
        "author": {"id": "7", "username": "author"},
        "timestamp": "2026-09-10T12:00:00Z",
        "content": "Plans",
        "message_reference": {"channel_id": "3001"}
    }))
    .unwrap();
    store.apply(Event::MessageCreate(Box::new(message)));
    let chip = store.network_channel(2002).unwrap().messages[0]
        .thread
        .as_ref()
        .unwrap();
    assert_eq!(chip.id, 3001);
    assert_eq!(chip.message_count, 3);
}
