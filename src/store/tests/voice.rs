use super::*;
use crate::discord::{Event, model};
use std::time::{Duration, UNIX_EPOCH};

fn session_start(channel_id: u64, start: Option<u64>) -> Event {
    Event::VoiceChannelStartTimeUpdate(model::VoiceChannelStartTimeUpdate {
        guild_id: 200,
        id: channel_id,
        voice_start_time: Some(start),
    })
}

fn state(guild: u64, channel: Option<u64>, user: u64) -> model::VoiceState {
    serde_json::from_value(json!({
        "guild_id": guild.to_string(),
        "channel_id": channel.map(|id| id.to_string()),
        "user_id": user.to_string()
    }))
    .unwrap()
}

fn ready_with_voice() -> model::Ready {
    let mut ready = ready();
    ready.users.push(model::User {
        id: 3,
        username: "zed".into(),
        global_name: Some("Zed".into()),
        ..Default::default()
    });
    let guild = &mut ready.guilds[0];
    guild.fill_voice_state_guilds();
    guild.voice_states = vec![
        state(200, Some(2005), 2),
        state(200, Some(2005), 1),
        state(200, Some(2006), 3),
    ];
    ready
}

fn names(store: &Store, channel: u64) -> Vec<u64> {
    store
        .voice_members(200, channel)
        .into_iter()
        .map(|member| member.user_id)
        .collect()
}

#[test]
fn voice_members_join_move_leave_and_sort() {
    let mut store = Store::from_ready(ready_with_voice());
    assert_eq!(names(&store, 2005), [2, 1]);
    assert_eq!(names(&store, 2006), [3]);
    let change = store.apply(Event::VoiceStateUpdate(Box::new(state(200, Some(2006), 2))));
    assert_eq!(change, Change::Structure);
    assert_eq!(names(&store, 2005), [1]);
    assert_eq!(names(&store, 2006), [2, 3]);

    let change = store.apply(Event::VoiceStateUpdate(Box::new(state(200, None, 1))));
    assert_eq!(change, Change::Structure);
    assert!(store.voice_members(200, 2005).is_empty());
}

#[test]
fn voice_state_member_is_merged_and_guild_delete_clears() {
    let mut store = Store::from_ready(ready_with_voice());
    let update: model::VoiceState = serde_json::from_value(json!({
        "guild_id": "200", "channel_id": "2006", "user_id": "5",
        "self_mute": true, "self_deaf": false,
        "member": {"user": {"id": "5", "username": "newbie", "global_name": "New Bie"},
                   "nick": "New", "roles": ["201"]}
    }))
    .unwrap();
    store.apply(Event::VoiceStateUpdate(Box::new(update)));
    assert_eq!(store.member_name(200, 5), Some("New"));
    assert_eq!(
        store.user(5).map(|user| user.name.as_str()),
        Some("New Bie")
    );
    assert_eq!(store.voice_members(200, 2006)[0].user_id, 5);

    store.apply(Event::GuildDelete { id: 200 });
    assert!(store.voice_members(200, 2006).is_empty());
}

#[test]
fn ready_supplemental_voice_states_rebuild_rows_and_keep_unknown_ids() {
    let mut payload = social_fixture("supplemental");
    payload["guilds"][0]["voice_states"] = json!([
        {"channel_id": "2005", "user_id": "2", "self_mute": true},
        {"channel_id": "2005", "user_id": "100"},
        {"channel_id": "2005", "user_id": "99"}
    ]);
    let update: model::ReadySupplemental = serde_json::from_value(payload).unwrap();
    let mut store = Store::from_ready(ready());
    let change = store.apply(Event::ReadySupplemental(Box::new(update)));
    assert_eq!(change, Change::Structure);
    let members = store.voice_members(200, 2005);
    assert_eq!(
        members
            .iter()
            .map(|member| member.user_id)
            .collect::<Vec<_>>(),
        [100, 99, 2]
    );
    assert!(members[2].self_mute);
}

#[test]
fn ready_supplemental_users_supply_missing_voice_names() {
    let mut payload = social_fixture("supplemental");
    payload["guilds"][0]["voice_states"] = json!([{"channel_id": "2005", "user_id": "99"}]);
    payload["users"] = json!([{"id": "99", "username": "new", "global_name": "New Voice"}]);
    let update: model::ReadySupplemental = serde_json::from_value(payload).unwrap();
    let mut store = Store::from_ready(ready());
    store.apply(Event::ReadySupplemental(Box::new(update)));
    assert_eq!(store.member_name(200, 99), Some("New Voice"));
    assert_eq!(store.voice_members(200, 2005)[0].user_id, 99);
}

#[test]
fn guild_voice_activity_prefers_streaming() {
    let mut store = Store::from_ready(ready_with_voice());
    assert_eq!(store.guild_voice_activity(200), Some(VoiceActivity::Voice));

    let mut streaming = state(200, Some(2006), 3);
    streaming.self_stream = true;
    store.apply(Event::VoiceStateUpdate(Box::new(streaming)));
    assert_eq!(
        store.guild_voice_activity(200),
        Some(VoiceActivity::Streaming)
    );

    for user in [1, 2, 3] {
        store.apply(Event::VoiceStateUpdate(Box::new(state(200, None, user))));
    }
    assert_eq!(store.guild_voice_activity(200), None);
}

#[test]
fn guild_voice_preview_combines_channels_and_tracks_streaming_and_leaves() {
    let mut store = Store::from_ready(ready_with_voice());
    let members = store.guild_voice_members(200);
    assert_eq!(
        members
            .iter()
            .map(|member| member.user_id)
            .collect::<Vec<_>>(),
        [2, 1, 3]
    );
    assert!(members.iter().all(|member| !member.streaming));
    assert!(store.guild_voice_members(100).is_empty());
    assert!(store.guild_voice_members(u64::MAX).is_empty());

    let mut streaming = state(200, Some(2006), 3);
    streaming.self_stream = true;
    store.apply(Event::VoiceStateUpdate(Box::new(streaming)));
    store.apply(Event::VoiceStateUpdate(Box::new(state(200, Some(2006), 2))));
    let members = store.guild_voice_members(200);
    assert_eq!(
        members.len(),
        3,
        "moving channels does not duplicate a participant"
    );
    assert_eq!(
        members
            .iter()
            .filter(|member| member.streaming)
            .map(|member| member.user_id)
            .collect::<Vec<_>>(),
        [3]
    );

    store.apply(Event::VoiceStateUpdate(Box::new(state(200, None, 3))));
    let members = store.guild_voice_members(200);
    assert_eq!(members.len(), 2);
    assert!(members.iter().all(|member| !member.streaming));
    store.apply(Event::GuildDelete { id: 200 });
    assert!(store.guild_voice_members(200).is_empty());
}

#[test]
fn guild_voice_preview_ignores_disconnected_snapshots_and_keeps_unknown_users() {
    let mut ready = ready_with_voice();
    let guild = ready
        .guilds
        .iter_mut()
        .find(|guild| guild.id == 200)
        .unwrap();
    let mut disconnected = state(200, None, 4);
    disconnected.self_stream = true;
    guild.voice_states.push(disconnected);
    guild.voice_states.push(state(200, Some(2005), 99));
    let store = Store::from_ready(ready);
    let members = store.guild_voice_members(200);
    assert_eq!(members.len(), 4);
    assert!(members.iter().any(|member| member.user_id == 99));
    assert!(
        members
            .iter()
            .all(|member| member.user_id != 4 && !member.streaming)
    );
}

#[test]
fn voice_session_snapshot_and_updates_preserve_missing_fields() {
    let mut store = Store::from_ready(ready_with_voice());
    let snapshot = json!({"guild_id":"200","channels":[
        {"id":"2005","voice_start_time":1770000000},
        {"id":"2006","voice_start_time":1770000010}
    ]});
    assert_eq!(
        store.apply(Event::ChannelInfo(
            serde_json::from_value(snapshot.clone()).unwrap()
        )),
        Change::VoiceTimers
    );
    assert_eq!(
        store.voice_started_at(200, 2005),
        Some(UNIX_EPOCH + Duration::from_secs(1770000000))
    );
    assert_eq!(
        store.apply(Event::ChannelInfo(
            serde_json::from_value(snapshot).unwrap()
        )),
        Change::None
    );

    let status_only = json!({"guild_id":"200","channels":[{"id":"2005","status":"Chatting"}]});
    assert_eq!(
        store.apply(Event::ChannelInfo(
            serde_json::from_value(status_only).unwrap(),
        )),
        Change::None
    );
    assert!(store.voice_started_at(200, 2005).is_some());
    assert_eq!(store.apply(session_start(2005, None)), Change::VoiceTimers);
    assert_eq!(store.voice_started_at(200, 2005), None);
    assert!(store.voice_started_at(200, 2006).is_some());
    assert_eq!(
        store.apply(session_start(2005, Some(1770000100))),
        Change::VoiceTimers
    );
    let omitted = json!({"guild_id":"200","id":"2005"});
    assert_eq!(
        store.apply(Event::VoiceChannelStartTimeUpdate(
            serde_json::from_value(omitted).unwrap()
        )),
        Change::None
    );
    assert_eq!(
        store.voice_started_at(200, 2005),
        Some(UNIX_EPOCH + Duration::from_secs(1770000100))
    );
}

#[test]
fn voice_session_survives_rejoining_and_resets_when_last_member_leaves() {
    let mut store = Store::from_ready(ready_with_voice());
    store.apply(session_start(2005, Some(1770000000)));
    let original = store.voice_started_at(200, 2005);

    store.apply(Event::VoiceStateUpdate(Box::new(state(200, None, 1))));
    assert_eq!(store.voice_started_at(200, 2005), original);
    store.apply(Event::VoiceStateUpdate(Box::new(state(200, Some(2005), 1))));
    assert_eq!(store.voice_started_at(200, 2005), original);

    store.apply(Event::VoiceStateUpdate(Box::new(state(200, Some(2006), 1))));
    assert_eq!(store.voice_started_at(200, 2005), original);
    store.apply(Event::VoiceStateUpdate(Box::new(state(200, None, 2))));
    assert_eq!(store.voice_started_at(200, 2005), None);

    store.apply(Event::VoiceStateUpdate(Box::new(state(200, Some(2005), 1))));
    assert_eq!(store.voice_started_at(200, 2005), None);
    store.apply(session_start(2005, Some(1770000200)));
    assert_ne!(store.voice_started_at(200, 2005), original);
}

#[test]
fn voice_session_is_refetched_after_ready_instead_of_using_local_join_time() {
    let snapshot = json!({"guild_id":"200","channels":[
        {"id":"2005","voice_start_time":1770000000}
    ]});
    let mut store = Store::from_ready(ready_with_voice());
    store.apply(Event::ChannelInfo(
        serde_json::from_value(snapshot.clone()).unwrap(),
    ));
    let original = store.voice_started_at(200, 2005);

    store.apply(Event::Ready(Box::new(ready_with_voice())));
    assert_eq!(store.voice_started_at(200, 2005), None);
    store.apply(Event::ChannelInfo(
        serde_json::from_value(snapshot).unwrap(),
    ));
    assert_eq!(store.voice_started_at(200, 2005), original);
}

#[test]
fn deleting_voice_channels_and_guilds_clears_session_timestamps() {
    let mut store = Store::from_ready(ready_with_voice());
    store.apply(session_start(2005, Some(1770000000)));
    store.apply(session_start(2006, Some(1770000000)));
    store.apply(Event::ChannelDelete { id: 2005 });
    assert_eq!(store.voice_started_at(200, 2005), None);
    assert!(store.voice_started_at(200, 2006).is_some());
    store.apply(Event::GuildDelete { id: 200 });
    assert_eq!(store.voice_started_at(200, 2006), None);
    assert_eq!(
        store.apply(session_start(2006, Some(1770000000))),
        Change::None
    );
}
