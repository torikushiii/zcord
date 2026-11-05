use super::*;

fn home_ready() -> model::Ready {
    serde_json::from_str(include_str!(
        "../../../crates/discord/src/fixtures/home.json"
    ))
    .unwrap()
}

fn call(value: serde_json::Value) -> model::PrivateCall {
    serde_json::from_value(value).unwrap()
}

fn recipient(user_id: u64) -> model::ChannelRecipient {
    serde_json::from_value(json!({
        "channel_id": "902", "user": {"id": user_id.to_string(), "username": "new"}
    }))
    .unwrap()
}

#[test]
fn call_lifecycle_preserves_partial_fields_and_clears_explicit_lists() {
    let mut store = Store::from_ready(home_ready());
    store.apply(Event::CallCreate(call(json!({
        "channel_id": "902", "message_id": "50", "region": "singapore",
        "ringing": ["1"], "voice_states": [{"user_id": "10", "self_mute": true}]
    }))));
    assert_eq!(store.incoming_call(), Some(902));
    let active = store.dm_call(902).unwrap();
    assert_eq!(active.voice_states[0].channel_id, Some(902));
    assert!(active.voice_states[0].self_mute);
    assert!(store.voice_members(0, 902)[0].self_mute);
    store.apply(Event::CallUpdate(call(
        json!({"channel_id": "902", "ringing": []}),
    )));
    let active = store.dm_call(902).unwrap();
    assert_eq!(active.message_id, Some(50));
    assert_eq!(active.region.as_deref(), Some("singapore"));
    assert_eq!(active.voice_states.len(), 1);
    assert_eq!(store.incoming_call(), None);
    store.apply(Event::CallUpdate(call(
        json!({"channel_id": "902", "region": null, "voice_states": []}),
    )));
    assert!(store.dm_call(902).unwrap().voice_states.is_empty());
    assert!(store.dm_call(902).unwrap().region.is_none());
    store.apply(Event::CallDelete { channel_id: 902 });
    assert!(store.dm_call(902).is_none());
}

#[test]
fn ready_calls_seed_private_state_and_unknown_channels_are_ignored() {
    let mut payload: serde_json::Value = serde_json::from_str(include_str!(
        "../../../crates/discord/src/fixtures/home.json"
    ))
    .unwrap();
    payload["calls"] = json!([
        {"channel_id": "900", "ringing": ["1"]},
        {"channel_id": "99999", "ringing": ["1"]}
    ]);
    let ready = serde_json::from_value(payload).unwrap();
    let store = Store::from_ready(ready);
    assert_eq!(store.incoming_call(), Some(900));
    assert!(store.dm_call(99999).is_none());
}

#[test]
fn dm_voice_state_updates_join_move_mute_and_leave_without_touching_guilds() {
    let mut ready = ready();
    ready.private_channels = home_ready().private_channels;
    ready.guilds[0].voice_states.push(
        serde_json::from_value(json!({
            "guild_id": "200", "channel_id": "2002", "user_id": "10"
        }))
        .unwrap(),
    );
    let mut store = Store::from_ready(ready);
    for (channel_id, mute) in [(Some("900"), false), (Some("902"), true)] {
        let state = serde_json::from_value(json!({
            "guild_id": null, "channel_id": channel_id, "user_id": "10", "self_mute": mute,
            "member": {"user": {"id": "10", "username": "updated"}}
        }))
        .unwrap();
        assert_eq!(
            store.apply(Event::VoiceStateUpdate(Box::new(state))),
            Change::Structure
        );
    }
    assert!(store.dm_call(900).unwrap().voice_states.is_empty());
    assert!(store.dm_call(902).unwrap().voice_states[0].self_mute);
    assert_eq!(store.user(10).unwrap().name, "updated");
    let leave = serde_json::from_value(json!({"channel_id": null, "user_id": "10"})).unwrap();
    store.apply(Event::VoiceStateUpdate(Box::new(leave)));
    assert!(store.dm_call(902).unwrap().voice_states.is_empty());
    assert!(store.dm_call(902).is_some());
    assert_eq!(store.voice_members(200, 2002).len(), 1);
}

#[test]
fn call_announcement_preserves_early_voice_state_and_decline_only_clears_own_ring() {
    let mut store = Store::from_ready(home_ready());
    let state = serde_json::from_value(json!({
        "channel_id": "902", "user_id": "10"
    }))
    .unwrap();
    store.apply(Event::VoiceStateUpdate(Box::new(state)));
    store.apply(Event::CallCreate(call(json!({
        "channel_id": "902", "message_id": "50", "ringing": ["1", "12"]
    }))));
    store.stop_ringing_locally(902);
    let active = store.dm_call(902).unwrap();
    assert_eq!(active.voice_states.len(), 1);
    assert_eq!(active.ringing, vec![12]);
    assert_eq!(store.incoming_call(), None);
}

#[test]
fn recipients_update_cache_membership_and_remove_own_group() {
    let mut store = Store::from_ready(home_ready());
    store.apply(Event::ChannelRecipientAdd(recipient(20)));
    store.apply(Event::ChannelRecipientAdd(recipient(20)));
    assert_eq!(
        store.private_channel(902).unwrap().recipient_ids,
        vec![10, 12, 20]
    );
    assert_eq!(store.user(20).unwrap().name, "new");
    store.apply(Event::ChannelRecipientRemove(recipient(10)));
    assert_eq!(
        store.private_channel(902).unwrap().recipient_ids,
        vec![12, 20]
    );
    store.apply(Event::CallCreate(call(
        json!({"channel_id": "902", "ringing": ["1"]}),
    )));
    store.apply(Event::ChannelRecipientRemove(recipient(1)));
    assert!(store.private_channel(902).is_none());
    assert!(!store.dm_list().iter().any(|dm| dm.channel.id == 902));
    assert!(store.channel_name(902).is_none());
    assert!(store.dm_call(902).is_none());
}

#[test]
fn private_channel_updates_merge_and_delete_removes_dm_and_call() {
    let mut store = Store::from_ready(home_ready());
    let update = serde_json::from_value(json!({
        "id": "902", "name": "Renamed", "icon": null, "owner_id": "1"
    }))
    .unwrap();
    store.apply(Event::ChannelUpsert(Box::new(update)));
    let group = store.private_channel(902).unwrap();
    assert_eq!(group.channel.name, "Renamed");
    assert_eq!(group.name.as_deref(), Some("Renamed"));
    assert!(group.icon.is_none());
    assert_eq!(group.owner_id, Some(1));
    assert_eq!(group.recipient_ids, vec![10, 12]);
    store.apply(Event::CallCreate(call(
        json!({"channel_id": "902", "ringing": ["1"]}),
    )));
    store.apply(Event::ChannelDelete { id: 902 });
    assert!(store.private_channel(902).is_none());
    assert!(!store.dm_list().iter().any(|dm| dm.channel.id == 902));
    assert!(store.channel_name(902).is_none());
    assert!(store.dm_call(902).is_none());
    assert_eq!(store.incoming_call(), None);
}
