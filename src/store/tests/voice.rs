use super::*;
use crate::discord::{Event, model};

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
