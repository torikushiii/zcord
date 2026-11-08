use super::*;
use crate::{Error, presence::Presence, settings_fixtures};
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compress, Compression, FlushCompress};

fn preloaded_update(version: Option<u64>, folders: bool, partial: bool) -> model::SettingsUpdate {
    let folders = if folders {
        let mut folder = vec![9];
        folder.extend(200u64.to_le_bytes());
        folder.extend(settings_fixtures::field(2, &settings_fixtures::uint(1, 42)));
        settings_fixtures::field(1, &folder)
    } else {
        Vec::new()
    };
    let mut proto = settings_fixtures::field(14, &folders);
    proto.extend(
        STANDARD
            .decode(settings_fixtures::status_fixture(Some("online"), None))
            .unwrap(),
    );
    if let Some(version) = version {
        proto.extend(settings_fixtures::field(
            1,
            &settings_fixtures::uint(3, version),
        ));
    }
    model::SettingsUpdate {
        settings: model::SettingsProto {
            kind: 1,
            proto: STANDARD.encode(proto),
        },
        partial,
    }
}

fn settings_state() -> (State, Presence) {
    let initial = preloaded_update(Some(10), true, false);
    let state = State {
        settings_version: settings_version(&initial.settings.proto),
        ..State::default()
    };
    let mut presence = Presence::default();
    presence.replace(Some(&initial.settings.proto));
    let rest = preloaded_update(Some(12), true, false);
    presence.update(&rest.settings.proto, false).unwrap();
    (state, presence)
}

// The store's side of these updates is covered by the app's store tests.
#[test]
fn outdated_partial_settings_are_forwarded_without_changing_presence() {
    let (mut state, mut presence) = settings_state();
    let revision = presence.revision;
    let update = preloaded_update(Some(11), false, true);
    assert!(matches!(
        presence.update(&update.settings.proto, update.partial),
        Err(Error::SettingsOutdated)
    ));
    assert_eq!(presence.revision, revision);
    assert!(state.forward_settings_update(&update));
    let older_partial = preloaded_update(Some(9), true, true);
    assert!(state.forward_settings_update(&older_partial));
    assert_eq!(state.settings_version, Some(11));
}

#[test]
fn full_settings_are_compared_with_store_versions_not_rest_versions() {
    let (mut state, mut presence) = settings_state();
    let update = preloaded_update(Some(11), false, false);
    assert!(matches!(
        presence.update(&update.settings.proto, update.partial),
        Err(Error::SettingsOutdated)
    ));
    assert!(state.forward_settings_update(&update));
    for version in [Some(10), None] {
        assert!(!state.forward_settings_update(&preloaded_update(version, true, false)));
        assert_eq!(state.settings_version, Some(11));
    }
    assert!(state.forward_settings_update(&preloaded_update(Some(11), false, false)));
    assert!(state.forward_settings_update(&preloaded_update(Some(13), true, false)));
    assert_eq!(state.settings_version, Some(13));
}

#[tokio::test]
async fn ready_commands_use_the_socket_and_pace_member_chunks() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        tcp.set_nodelay(true).unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        socket
            .send(Message::Text(
                json!({"op":10,"d":{"heartbeat_interval":60000}}).to_string(),
            ))
            .await
            .unwrap();
        let auth: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(auth["op"], 6);
        socket
            .send(Message::Text(
                json!({"op":0,"t":"RESUMED","s":2,"d":{}}).to_string(),
            ))
            .await
            .unwrap();
        let mut ops = Vec::new();
        let mut last_member: Option<Instant> = None;
        while ops.len() < 4 {
            let value: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            let op = value["op"].as_u64().unwrap();
            if op == 1 {
                socket
                    .send(Message::Text(json!({"op":11,"d":null}).to_string()))
                    .await
                    .unwrap();
                continue;
            }
            if op == 8 {
                if let Some(previous) = last_member {
                    assert!(
                        previous.elapsed() >= Duration::from_millis(250),
                        "member receive spacing: {:?}",
                        previous.elapsed()
                    );
                }
                last_member = Some(Instant::now());
            }
            ops.push(op);
        }
        assert_eq!(ops, [37, 8, 8, 8]);
        socket
            .send(Message::Text(json!({"op":7,"d":null}).to_string()))
            .await
            .unwrap();
    });
    let (mut socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
    if let MaybeTlsStream::Plain(tcp) = socket.get_ref() {
        tcp.set_nodelay(true).unwrap();
    }
    let (sender, mut events) = futures::channel::mpsc::unbounded();
    let (_control, mut shutdown) = watch::channel(Control::Running);
    let (command_sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let generation = Arc::new(AtomicU64::new(0));
    let epoch = generation.clone();
    let producer = tokio::spawn(async move {
        assert!(matches!(events.next().await, Some(Event::Resumed)));
        let epoch = epoch.load(Ordering::Acquire);
        assert_eq!(epoch, 1);
        let mut outgoing = vec![super::super::commands::subscription(1, 2, &[(0, 99)])];
        outgoing.extend(super::super::commands::members(
            1,
            &(1..=201).collect::<Vec<_>>(),
        ));
        for command in outgoing {
            assert!(command_sender.send((epoch, command)).is_ok());
        }
        sleep(Duration::from_secs(3)).await;
    });
    let result = timeout(
        Duration::from_secs(2),
        drive(
            &mut socket,
            "fixture-secret",
            &sender,
            &mut shutdown,
            &mut State {
                sequence: Some(1),
                session_id: Some("fixture-session".into()),
                resume_url: None,
                settings_version: None,
            },
            &mut HashSet::new(),
            &mut CommandReceiver {
                receiver,
                generation,
                presence: Arc::default(),
                presence_after: Instant::now(),
            },
            &VoiceManager::default(),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Outcome::Retry { .. }));
    server.await.unwrap();
    producer.abort();
    let _ = producer.await;
}

#[test]
fn zlib_stream_survives_odd_splits_and_multiple_messages() {
    let messages = [
        br#"{"op":10,"d":{"heartbeat_interval":41250}}"#.as_slice(),
        br#"{"op":11,"d":null}"#.as_slice(),
    ];
    let mut compressor = Compress::new(Compression::default(), true);
    let mut compressed = Vec::new();
    for message in messages {
        let mut chunk = [0; 512];
        let before = compressor.total_out();
        compressor
            .compress(message, &mut chunk, FlushCompress::Sync)
            .unwrap();
        compressed.extend_from_slice(&chunk[..(compressor.total_out() - before) as usize]);
    }
    let mut decoder = Decoder::new();
    let mut decoded = Vec::new();
    for chunk in compressed.chunks(3) {
        decoded.extend(decoder.push(chunk).unwrap());
    }
    assert_eq!(decoded, messages);
    assert_eq!(Decoder::new().push(&compressed).unwrap(), messages);
}

#[test]
fn close_codes_and_resume_url_policy() {
    assert!(matches!(
        close_outcome(4004),
        Outcome::Fatal(FatalError::InvalidToken)
    ));
    for code in [4007, 4009] {
        assert!(matches!(
            close_outcome(code),
            Outcome::Retry { reset: true, .. }
        ));
    }
    for code in 4010..=4016 {
        assert!(matches!(close_outcome(code), Outcome::Fatal(_)));
    }
    assert!(resume_url("wss://gateway-us-east1-b.discord.gg").is_some());
    assert!(resume_url("wss://discord.gg.attacker.example").is_none());
    assert!(resume_url("ws://gateway.discord.gg").is_none());
}

#[tokio::test]
async fn resume_replays_dispatch_and_honors_invalid_session() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        socket
            .send(Message::Text(
                json!({"op":10,"d":{"heartbeat_interval":100000}}).to_string(),
            ))
            .await
            .unwrap();
        let auth: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(auth["op"], 6);
        assert_eq!(auth["d"]["seq"], 42);
        assert_eq!(auth["d"]["session_id"], "fixture-session");
        for value in [
            json!({"op":0,"s":43,"t":"MESSAGE_UPDATE","d":{"id":"62","channel_id":"2002","content":"updated"}}),
            json!({"op":0,"s":44,"t":"RESUMED","d":{}}),
            json!({"op":1,"d":null}),
        ] {
            socket.send(Message::Text(value.to_string())).await.unwrap();
        }
        let heartbeat: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(heartbeat["op"], 1);
        assert_eq!(heartbeat["d"], 44);
        socket
            .send(Message::Text(json!({"op":11,"d":null}).to_string()))
            .await
            .unwrap();
        socket
            .send(Message::Text(json!({"op":9,"d":false}).to_string()))
            .await
            .unwrap();
    });
    let (mut socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
    let (sender, mut events) = futures::channel::mpsc::unbounded();
    let (_control, mut shutdown) = watch::channel(Control::Running);
    let (_command_sender, command_receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut state = State {
        sequence: Some(42),
        session_id: Some("fixture-session".into()),
        resume_url: None,
        settings_version: None,
    };
    let result = timeout(
        Duration::from_secs(3),
        drive(
            &mut socket,
            "fixture-secret",
            &sender,
            &mut shutdown,
            &mut state,
            &mut HashSet::new(),
            &mut CommandReceiver {
                receiver: command_receiver,
                generation: Arc::new(AtomicU64::new(0)),
                presence: Arc::default(),
                presence_after: Instant::now(),
            },
            &VoiceManager::default(),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Outcome::Retry { reset: true, .. }));
    assert_eq!(state.sequence, Some(44));
    assert!(matches!(events.next().await, Some(Event::MessageUpdate(_))));
    assert!(matches!(events.next().await, Some(Event::Resumed)));
    server.await.unwrap();
}

#[tokio::test]
async fn heartbeat_without_ack_reconnects() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        socket
            .send(Message::Text(
                json!({"op":10,"d":{"heartbeat_interval":30}}).to_string(),
            ))
            .await
            .unwrap();
        let auth: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(auth["op"], 2);
        let heartbeat: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(heartbeat["op"], 1);
        assert!(heartbeat["d"].is_null());
        let _ = socket.next().await;
    });
    let (mut socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
    let (sender, _events) = futures::channel::mpsc::unbounded();
    let (_control, mut shutdown) = watch::channel(Control::Running);
    let (_command_sender, command_receiver) = tokio::sync::mpsc::unbounded_channel();
    let result = timeout(
        Duration::from_secs(3),
        drive(
            &mut socket,
            "fixture-secret",
            &sender,
            &mut shutdown,
            &mut State::default(),
            &mut HashSet::new(),
            &mut CommandReceiver {
                receiver: command_receiver,
                generation: Arc::new(AtomicU64::new(0)),
                presence: Arc::default(),
                presence_after: Instant::now(),
            },
            &VoiceManager::default(),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Outcome::Retry { reset: false, .. }));
    drop(socket);
    server.await.unwrap();
}
