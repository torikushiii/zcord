use super::*;

#[test]
fn server_name_strips_scheme_path_and_port() {
    let expected = Some("c-sin20-b41c6763.discord.media");
    assert_eq!(
        server_name("c-sin20-b41c6763.discord.media:443").as_deref(),
        expected
    );
    assert_eq!(
        server_name("c-sin20-b41c6763.discord.media").as_deref(),
        expected
    );
    assert_eq!(
        server_name("wss://c-sin20-b41c6763.discord.media:443").as_deref(),
        expected
    );
    assert_eq!(
        server_name("  wss://c-sin20-b41c6763.discord.media/?v=9  ").as_deref(),
        expected
    );
    assert_eq!(server_name(""), None);
}

#[test]
fn voice_server_restarts_only_when_endpoint_or_token_changes() {
    assert_eq!(server_restart_reason(false, false), None);
    assert_eq!(server_restart_reason(true, false), Some("token changed"));
    assert_eq!(
        server_restart_reason(false, true),
        Some("server endpoint changed")
    );
    assert_eq!(
        server_restart_reason(true, true),
        Some("server endpoint and token changed")
    );
}

#[test]
fn op4_join_leave_and_mute_flags() {
    let manager = VoiceManager::default();
    let join = manager.join(10, 20);
    assert_eq!(join["op"], 4);
    assert_eq!(join["d"]["guild_id"], "10");
    assert_eq!(join["d"]["channel_id"], "20");
    assert_eq!(join["d"]["self_mute"], false);
    assert_eq!(join["d"]["self_deaf"], false);
    assert_eq!(join["d"]["self_video"], false);
    assert_eq!(join["d"]["flags"], 3);
    assert_eq!(manager.take_pending_op4(), Some(join));

    let deaf = manager.set_deaf(true).unwrap();
    assert_eq!(deaf["d"]["self_mute"], true);
    assert_eq!(deaf["d"]["self_deaf"], true);
    let unmuted_while_deaf = manager.set_mute(false).unwrap();
    assert_eq!(unmuted_while_deaf["d"]["self_mute"], true);
    let undeafened = manager.set_deaf(false).unwrap();
    assert_eq!(undeafened["d"]["self_mute"], false);
    assert_eq!(undeafened["d"]["self_deaf"], false);

    let leave = manager.leave().unwrap();
    assert_eq!(leave["d"]["guild_id"], "10");
    assert_eq!(leave["d"]["channel_id"], serde_json::Value::Null);
}

#[test]
fn dm_join_confirm_flags_server_and_leave_use_private_identity() {
    let manager = VoiceManager::default();
    manager.gateway_ready(1, "own-session", false);
    let join = manager.join(0, 902);
    assert!(join["d"]["guild_id"].is_null());
    assert_eq!(join["d"]["channel_id"], "902");
    let state = serde_json::from_value(json!({
        "user_id": "1", "channel_id": "902", "guild_id": null
    }))
    .unwrap();
    manager.voice_state(&state, Some("own-session".into()));
    assert_eq!(manager.lock().confirmed, Some((0, 902)));
    manager.dm_voice_server(
        Some(901),
        Secret("stale".into()),
        Some("old.invalid".into()),
    );
    assert!(manager.lock().servers.is_empty());
    manager.dm_voice_server(
        Some(902),
        Secret("dm-token".into()),
        Some("dm.invalid".into()),
    );
    assert!(manager.lock().servers.contains_key(&902));
    let mute = manager.set_mute(true).unwrap();
    assert!(mute["d"]["guild_id"].is_null());
    assert_eq!(mute["d"]["self_mute"], true);
    let deaf = manager.set_deaf(true).unwrap();
    assert!(deaf["d"]["guild_id"].is_null());
    assert_eq!(deaf["d"]["self_deaf"], true);
    let leave = manager.leave().unwrap();
    assert!(leave["d"]["guild_id"].is_null());
    assert!(leave["d"]["channel_id"].is_null());
    assert!(manager.lock().servers.is_empty());
}

#[test]
fn dm_gateway_disconnect_clears_confirmed_connection() {
    let manager = VoiceManager::default();
    manager.gateway_ready(1, "own-session", false);
    manager.join(0, 902);
    let state = serde_json::from_value(json!({"user_id": "1", "channel_id": null})).unwrap();
    manager.voice_state(&state, Some("own-session".into()));
    assert!(manager.lock().desired.is_none());
    assert!(manager.lock().confirmed.is_none());
}

#[test]
fn mute_persists_outside_voice() {
    let manager = VoiceManager::default();
    assert!(manager.set_mute(true).is_none());
    assert!(manager.set_deaf(true).is_none());
    let join = manager.join(10, 20);
    assert_eq!(join["d"]["self_mute"], true);
    assert_eq!(join["d"]["self_deaf"], true);
    manager.leave();
}

#[test]
fn input_gate_is_closed_without_a_call() {
    let manager = VoiceManager::default();
    assert!(!manager.input_gate_open());
    assert_eq!(manager.input_level(), None);
}

#[test]
fn giving_up_queues_leave_op4_but_server_disconnects_do_not() {
    let manager = VoiceManager::default();
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    manager.give_up(true);
    let leave = manager.take_pending_op4().unwrap();
    assert_eq!(leave["op"], 4);
    assert_eq!(leave["d"]["guild_id"], "10");
    assert_eq!(leave["d"]["channel_id"], serde_json::Value::Null);

    let manager = VoiceManager::default();
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    manager.give_up(false);
    assert!(manager.take_pending_op4().is_none());
}

#[tokio::test]
async fn give_up_sends_leave_through_gateway() {
    let (events, _received) = futures::channel::mpsc::unbounded();
    let (commands, mut sent) = tokio::sync::mpsc::unbounded_channel();
    let generation = Arc::new(AtomicU64::new(1));
    let manager = VoiceManager::new(
        &tokio::runtime::Handle::current(),
        events,
        commands,
        generation,
    );
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    manager.give_up(true);
    let (sent_generation, command) = sent.try_recv().unwrap();
    assert_eq!(sent_generation, 1);
    assert_eq!(command.payload["op"], 4);
    assert_eq!(command.payload["d"]["channel_id"], serde_json::Value::Null);
    assert_eq!(command.payload["d"]["guild_id"], "10");
}

#[tokio::test]
async fn another_device_taking_over_displaces_the_voice_session() {
    use futures::StreamExt;

    let (events, mut received) = futures::channel::mpsc::unbounded();
    let (commands, _command_receiver) = tokio::sync::mpsc::unbounded_channel();
    let generation = Arc::new(AtomicU64::new(0));
    let manager = VoiceManager::new(
        &tokio::runtime::Handle::current(),
        events,
        commands,
        generation,
    );
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    assert!(matches!(
        received.next().await,
        Some(Event::Voice(VoiceEvent::Connecting { .. }))
    ));

    let state: model::VoiceState = serde_json::from_value(json!({
        "guild_id": "10", "channel_id": "20", "user_id": "1"
    }))
    .unwrap();
    manager.voice_state(&state, Some("phone-session".into()));
    {
        let shared = manager.lock();
        assert_eq!(shared.desired, None);
        assert_eq!(shared.confirmed, None);
        assert!(!shared.dirty);
    }
    match received.next().await {
        Some(Event::Voice(VoiceEvent::Disconnected {
            reason: Some(reason),
        })) => assert_eq!(reason, "Voice moved to another device"),
        _ => panic!("unexpected event after displacement"),
    }
    assert!(manager.take_pending_op4().is_none());
}

#[tokio::test]
async fn voice_server_token_change_restarts_a_connected_task() {
    let (events, _received) = futures::channel::mpsc::unbounded();
    let (commands, _command_receiver) = tokio::sync::mpsc::unbounded_channel();
    let manager = VoiceManager::new(
        &tokio::runtime::Handle::current(),
        events,
        commands,
        Arc::new(AtomicU64::new(1)),
    );
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    let state: model::VoiceState = serde_json::from_value(json!({
        "guild_id": "10", "channel_id": "20", "user_id": "1"
    }))
    .unwrap();
    manager.voice_state(&state, Some("own-session".into()));
    manager.voice_server(10, Secret("old-token".into()), Some("127.0.0.1:1".into()));
    let old_shutdown = manager
        .lock()
        .task
        .as_ref()
        .expect("voice task")
        .shutdown
        .subscribe();

    manager.voice_server(10, Secret("old-token".into()), Some("127.0.0.1:1".into()));
    assert!(!*old_shutdown.borrow());

    manager.voice_server(10, Secret("new-token".into()), Some("127.0.0.1:1".into()));
    assert!(*old_shutdown.borrow());
    assert!(manager.lock().task.is_some());
}

#[tokio::test]
async fn rejoin_waits_for_a_fresh_voice_server_update() {
    let (events, _received) = futures::channel::mpsc::unbounded();
    let (commands, _command_receiver) = tokio::sync::mpsc::unbounded_channel();
    let manager = VoiceManager::new(
        &tokio::runtime::Handle::current(),
        events,
        commands,
        Arc::new(AtomicU64::new(1)),
    );
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    let state: model::VoiceState = serde_json::from_value(json!({
        "guild_id": "10", "channel_id": "20", "user_id": "1"
    }))
    .unwrap();
    manager.voice_state(&state, Some("own-session".into()));
    manager.voice_server(10, Secret("old-token".into()), Some("127.0.0.1:1".into()));
    assert!(manager.lock().servers.contains_key(&10));
    assert!(manager.lock().task.is_some());

    manager.leave();
    assert!(!manager.lock().servers.contains_key(&10));
    assert!(manager.lock().task.is_none());

    manager.join(10, 20);
    manager.voice_state(&state, Some("own-session".into()));
    assert!(!manager.lock().servers.contains_key(&10));
    assert!(manager.lock().task.is_none());

    manager.voice_server(10, Secret("new-token".into()), Some("127.0.0.1:1".into()));
    assert!(manager.lock().servers.contains_key(&10));
    assert!(manager.lock().task.is_some());
}

#[test]
fn watch_and_stop_payloads_refuse_the_own_user() {
    let manager = VoiceManager::default();
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    let state: model::VoiceState = serde_json::from_value(json!({
        "guild_id": "10", "channel_id": "20", "user_id": "1"
    }))
    .unwrap();
    manager.voice_state(&state, Some("own-session".into()));

    assert_eq!(
        manager.watch(2),
        Some(json!({"op": 20, "d": {"stream_key": "guild:10:20:2"}}))
    );
    assert_eq!(manager.watch(1), None);
    assert_eq!(
        manager.stop_watching(),
        Some(json!({"op": 19, "d": {"stream_key": "guild:10:20:2"}}))
    );
    assert!(manager.stop_watching().is_none());
}

#[test]
fn stream_setup_ignores_another_key_and_leaving_ends_the_watch() {
    let manager = VoiceManager::default();
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    let state: model::VoiceState = serde_json::from_value(json!({
        "guild_id": "10", "channel_id": "20", "user_id": "1"
    }))
    .unwrap();
    manager.voice_state(&state, Some("own-session".into()));
    manager.watch(2);

    manager.stream_created("guild:10:20:3", 30, Some(31));
    assert!(
        manager
            .lock()
            .stream
            .as_ref()
            .unwrap()
            .rtc_server_id
            .is_none()
    );
    manager.stream_created("guild:10:20:2", 30, Some(31));
    assert_eq!(
        manager.lock().stream.as_ref().unwrap().rtc_server_id,
        Some(30)
    );

    manager.leave();
    assert!(manager.lock().stream.is_none());
}

#[test]
fn go_live_and_stop_sharing_use_the_call_key() {
    let manager = VoiceManager::default();
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    let state: model::VoiceState = serde_json::from_value(json!({
        "guild_id": "10", "channel_id": "20", "user_id": "1"
    }))
    .unwrap();
    manager.voice_state(&state, Some("own-session".into()));

    assert_eq!(
        manager.go_live(),
        Some(json!({"op": 18, "d": {
            "type": "guild", "guild_id": "10", "channel_id": "20",
            "preferred_region": null,
        }}))
    );
    assert_eq!(
        manager.stop_sharing(),
        Some(json!({"op": 19, "d": {"stream_key": "guild:10:20:1"}}))
    );
    assert!(manager.stop_sharing().is_none());
}

#[test]
fn share_and_watch_state_are_separate_and_route_by_key() {
    let manager = VoiceManager::default();
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    let state: model::VoiceState = serde_json::from_value(json!({
        "guild_id": "10", "channel_id": "20", "user_id": "1"
    }))
    .unwrap();
    manager.voice_state(&state, Some("own-session".into()));

    assert!(manager.watch(2).is_some());
    assert!(
        manager
            .start_share(
                ScreenSource {
                    id: super::super::stream::ScreenSourceId::Monitor(7),
                    name: "Display".into(),
                    is_window: false,
                },
                StreamQuality::STANDARD,
                false,
            )
            .is_some()
    );
    manager.stream_created("guild:10:20:1", 40, Some(41));
    assert!(
        manager
            .lock()
            .stream
            .as_ref()
            .unwrap()
            .rtc_server_id
            .is_none()
    );
    assert_eq!(
        manager.lock().share.as_ref().unwrap().rtc_server_id,
        Some(40)
    );
    manager.stream_created("guild:10:20:2", 30, Some(31));
    assert_eq!(
        manager.lock().stream.as_ref().unwrap().rtc_server_id,
        Some(30)
    );
    assert!(manager.lock().stream.is_some() && manager.lock().share.is_some());
}

#[test]
fn fresh_gateway_ready_drops_voice_state() {
    let manager = VoiceManager::default();
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    let state: model::VoiceState = serde_json::from_value(json!({
        "guild_id": "10", "channel_id": "20", "user_id": "1"
    }))
    .unwrap();
    manager.voice_state(&state, Some("own-session".into()));
    assert_eq!(manager.lock().confirmed, Some((10, 20)));
    manager.gateway_ready(1, "own-session", true);
    let shared = manager.lock();
    assert_eq!(shared.confirmed, None);
    assert_eq!(shared.desired, None);
}

#[test]
fn stopping_a_watch_clears_queued_stream_audio() {
    let manager = VoiceManager::default();
    manager.gateway_ready(1, "own-session", false);
    manager.join(10, 20);
    let state: model::VoiceState = serde_json::from_value(json!({
        "guild_id": "10", "channel_id": "20", "user_id": "1"
    }))
    .unwrap();
    manager.voice_state(&state, Some("own-session".into()));
    manager.watch(2);
    let flags = manager.lock().flags.clone();
    let generation = flags.begin_stream_audio();
    flags.push_stream_audio(generation, 1, vec![1, 2, 3]);
    assert_eq!(flags.queued_stream_audio(), 1);
    manager.stop_watching();
    assert_eq!(flags.queued_stream_audio(), 0);
}
