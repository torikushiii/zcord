use super::*;
use crate::{Error, presence::Presence, settings_fixtures};
use base64::{Engine, engine::general_purpose::STANDARD};
use zstd::stream::raw::Encoder;

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

struct Frames(Encoder<'static>);

impl Frames {
    fn new() -> Self {
        Self(Encoder::new(3).unwrap())
    }

    fn compress(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        let mut input = InBuffer::around(bytes);
        while input.pos() < bytes.len() {
            output.reserve(4096);
            let written = output.len();
            self.0
                .run(&mut input, &mut OutBuffer::around_pos(&mut output, written))
                .unwrap();
        }
        loop {
            output.reserve(4096);
            let written = output.len();
            let remaining = self
                .0
                .flush(&mut OutBuffer::around_pos(&mut output, written))
                .unwrap();
            if remaining == 0 {
                return output;
            }
        }
    }

    fn message(&mut self, value: Value) -> Message {
        Message::Binary(self.compress(&etf::encode(&value)))
    }
}

type ServerSocket = WebSocketStream<TcpStream>;

async fn read(socket: &mut ServerSocket) -> Value {
    loop {
        match socket.next().await.unwrap().unwrap() {
            Message::Binary(bytes) => return etf::decode(&bytes).unwrap(),
            Message::Close(_) => panic!("client closed the socket"),
            _ => {}
        }
    }
}

async fn write(socket: &mut ServerSocket, frames: &mut Frames, value: Value) {
    socket.send(frames.message(value)).await.unwrap();
}

async fn listen() -> (std::net::SocketAddr, tokio::net::TcpListener) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    (listener.local_addr().unwrap(), listener)
}

async fn accept(listener: tokio::net::TcpListener) -> (ServerSocket, Frames) {
    let (tcp, _) = listener.accept().await.unwrap();
    tcp.set_nodelay(true).unwrap();
    (
        tokio_tungstenite::accept_async(tcp).await.unwrap(),
        Frames::new(),
    )
}

async fn connect(address: std::net::SocketAddr) -> Socket {
    let (socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
    if let MaybeTlsStream::Plain(tcp) = socket.get_ref() {
        tcp.set_nodelay(true).unwrap();
    }
    socket
}

fn resumable(sequence: u64) -> State {
    State {
        sequence,
        session_id: Some("fixture-session".into()),
        ..State::default()
    }
}

fn receiver() -> (
    tokio::sync::mpsc::UnboundedSender<(u64, Command)>,
    CommandReceiver,
) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let commands = CommandReceiver::new(receiver, Arc::new(AtomicU64::new(0)), Arc::default());
    (sender, commands)
}

#[tokio::test]
async fn ready_commands_use_the_socket_and_pace_member_chunks() {
    let (address, listener) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, mut frames) = accept(listener).await;
        write(
            &mut socket,
            &mut frames,
            json!({"op":10,"d":{"heartbeat_interval":60000}}),
        )
        .await;
        assert_eq!(read(&mut socket).await["op"], 6);
        write(
            &mut socket,
            &mut frames,
            json!({"op":0,"t":"RESUMED","s":2,"d":{}}),
        )
        .await;
        let mut ops = Vec::new();
        let mut last_member: Option<Instant> = None;
        while ops.len() < 4 {
            let value = read(&mut socket).await;
            let op = value["op"].as_u64().unwrap();
            if op == 40 {
                write(&mut socket, &mut frames, json!({"op":11,"d":null})).await;
                continue;
            }
            if op == 8 {
                assert_eq!(value["d"]["guild_id"], json!(["1"]));
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
        write(&mut socket, &mut frames, json!({"op":7,"d":null})).await;
    });
    let mut socket = connect(address).await;
    let (sender, mut events) = futures::channel::mpsc::unbounded();
    let (_control, mut shutdown) = watch::channel(Control::Running);
    let (command_sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let generation = Arc::new(AtomicU64::new(0));
    let epoch = generation.clone();
    let producer = tokio::spawn(async move {
        assert!(matches!(events.next().await, Some(Event::Resumed)));
        let epoch = epoch.load(Ordering::Acquire);
        assert_eq!(epoch, 1);
        let mut outgoing = vec![
            super::super::commands::subscription(1, 2, &[(0, 99)]),
            super::super::commands::subscription(1, 2, &[(0, 99)]),
        ];
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
            &mut Secret("fixture-secret".into()),
            &Weak::new(),
            &sender,
            &mut shutdown,
            &mut resumable(1),
            &mut HashSet::new(),
            &mut CommandReceiver::new(receiver, generation, Arc::default()),
            &VoiceManager::default(),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Outcome::Reconnect));
    server.await.unwrap();
    producer.abort();
    let _ = producer.await;
}

#[test]
fn zstd_stream_keeps_one_context_across_messages() {
    let mut frames = Frames::new();
    let mut inflater = Inflater::new().unwrap();
    let hello = etf::encode(&json!({"op":10,"d":{"heartbeat_interval":41250}}));
    let ack = etf::encode(&json!({"op":11,"d":null}));
    let large: Vec<u8> = (0..300_000u32)
        .flat_map(|n| n.wrapping_mul(2_654_435_761).to_le_bytes())
        .collect();
    for message in [&hello, &ack, &hello, &large] {
        assert_eq!(inflater.push(&frames.compress(message)).unwrap(), *message);
    }
    let later = frames.compress(&ack);
    assert_eq!(inflater.push(&later).unwrap(), ack);
    assert!(Inflater::new().unwrap().push(&later).is_err());
    assert!(Inflater::new().unwrap().push(&[1, 2, 3, 4]).is_err());
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
    assert_eq!(
        resume_url("wss://gateway-us-east1-b.discord.gg").as_deref(),
        Some("wss://gateway-us-east1-b.discord.gg/?encoding=etf&v=9&compress=zstd-stream")
    );
    assert!(resume_url("wss://discord.gg.attacker.example").is_none());
    assert!(resume_url("ws://gateway.discord.gg").is_none());
    assert_eq!(GATEWAY, format!("wss://gateway.discord.gg/?{QUERY}"));
}

#[test]
fn handshake_offers_compression_without_websocket_extensions() {
    let request = gateway_request(GATEWAY).unwrap();
    let headers = request.headers();
    assert_eq!(headers["accept-encoding"], "gzip, deflate, br, zstd");
    assert_eq!(headers["origin"], "https://discord.com");
    assert!(headers.get("sec-websocket-extensions").is_none());
}

#[test]
fn sessions_resume_only_within_three_minutes_of_the_last_ack() {
    let now = Instant::now();
    let mut state = State::default();
    assert!(!state.can_resume(now));
    state.session_id = Some("fixture-session".into());
    assert!(state.can_resume(now));
    state.last_ack = Some(now);
    assert!(state.can_resume(now + RESUME_WINDOW));
    assert!(!state.can_resume(now + RESUME_WINDOW + Duration::from_millis(1)));
    state.sequence = 9;
    state.voice_connected = true;
    state.resume_url = Some("wss://gateway-us-east1-b.discord.gg".into());
    state.clear();
    assert_eq!(state.sequence, 0);
    assert!(state.session_id.is_none());
    assert!(!state.voice_connected);
    assert!(state.resume_url.is_some());
}

#[test]
fn reidentify_carries_the_current_presence_and_auth_does_not_refresh_the_ack_clock() {
    let (_sender, commands) = receiver();
    let token = Secret("fixture-secret".into());
    let mut state = State::default();
    let first = commands.auth(&token, &mut state);
    assert_eq!(first["op"], 2);
    assert!(state.last_ack.is_none());
    commands
        .presence
        .lock()
        .unwrap()
        .replace(Some(&settings_fixtures::status_fixture(Some("dnd"), None)));
    let again = commands.auth(&token, &mut state);
    assert_eq!(
        again["d"]["presence"],
        json!({"status":"dnd","since":0,"activities":[],"afk":false})
    );
    state.session_id = Some("fixture-session".into());
    state.sequence = 7;
    state.last_ack = Some(Instant::now() - RESUME_WINDOW - Duration::from_secs(1));
    assert_eq!(commands.auth(&token, &mut state)["op"], 2);
    assert_eq!(state.sequence, 0);
    state.session_id = Some("fixture-session".into());
    state.sequence = 7;
    assert_eq!(commands.auth(&token, &mut state)["op"], 2);
    state.session_id = Some("fixture-session".into());
    state.sequence = 7;
    state.last_ack = Some(Instant::now());
    let resume = commands.auth(&token, &mut state);
    assert_eq!(
        resume,
        json!({"op":6,"d":{"token":"fixture-secret","session_id":"fixture-session","seq":7}})
    );
}

#[test]
fn backoff_grows_with_jitter_caps_and_resets() {
    let mut backoff = Backoff::default();
    let mut previous = Backoff::MIN;
    for failures in 1..=20 {
        let delay = backoff.fail();
        assert!(delay >= previous && delay <= Backoff::MAX);
        assert!(delay <= previous * 3);
        assert_eq!(backoff.failures, failures);
        previous = delay;
    }
    backoff.succeed();
    assert_eq!(backoff.current, Backoff::MIN);
    assert_eq!(backoff.failures, 0);
}

#[tokio::test]
async fn resume_replays_dispatch_and_reidentifies_on_the_same_socket() {
    let (address, listener) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, mut frames) = accept(listener).await;
        let auth = read(&mut socket).await;
        assert_eq!(auth["op"], 6);
        assert_eq!(auth["d"]["seq"], 42);
        assert_eq!(auth["d"]["session_id"], "fixture-session");
        for value in [
            json!({"op":10,"d":{"heartbeat_interval":100000}}),
            json!({"op":0,"s":43,"t":"MESSAGE_UPDATE","d":{
                "id":1_175_185_049_000_000_062u64,"channel_id":"2002","content":"updated"
            }}),
            json!({"op":0,"s":44,"t":"RESUMED","d":{}}),
            json!({"op":1,"d":null}),
        ] {
            write(&mut socket, &mut frames, value).await;
        }
        let heartbeat = read(&mut socket).await;
        assert_eq!(heartbeat["op"], 40);
        assert_eq!(heartbeat["d"]["seq"], 44);
        write(&mut socket, &mut frames, json!({"op":11,"d":null})).await;
        write(&mut socket, &mut frames, json!({"op":9,"d":false})).await;
        let identify = read(&mut socket).await;
        assert_eq!(identify["op"], 2);
        assert_eq!(identify["d"]["token"], "fixture-secret");
        write(&mut socket, &mut frames, json!({"op":7,"d":null})).await;
    });
    let mut socket = connect(address).await;
    let (sender, mut events) = futures::channel::mpsc::unbounded();
    let (_control, mut shutdown) = watch::channel(Control::Running);
    let mut state = resumable(42);
    let result = timeout(
        Duration::from_secs(3),
        drive(
            &mut socket,
            &mut Secret("fixture-secret".into()),
            &Weak::new(),
            &sender,
            &mut shutdown,
            &mut state,
            &mut HashSet::new(),
            &mut receiver().1,
            &VoiceManager::default(),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Outcome::Reconnect));
    assert_eq!(state.sequence, 0);
    assert!(state.session_id.is_none());
    let Some(Event::MessageUpdate(update)) = events.next().await else {
        panic!("expected the replayed message update");
    };
    assert_eq!(update.id, 1_175_185_049_000_000_062);
    assert!(matches!(events.next().await, Some(Event::Resumed)));
    assert!(matches!(events.next().await, Some(Event::Reconnecting)));
    server.await.unwrap();
}

#[tokio::test]
async fn identify_goes_out_before_hello_and_missing_acks_reconnect() {
    let (address, listener) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, mut frames) = accept(listener).await;
        assert_eq!(read(&mut socket).await["op"], 2);
        write(
            &mut socket,
            &mut frames,
            json!({"op":10,"d":{"heartbeat_interval":30}}),
        )
        .await;
        assert_eq!(
            read(&mut socket).await,
            json!({"op":40,"d":{"seq":0,"qos":{
                "active":true,"ver":31,"reasons":["foregrounded"]
            }}})
        );
        let _ = socket.next().await;
    });
    let mut socket = connect(address).await;
    let (sender, _events) = futures::channel::mpsc::unbounded();
    let (_control, mut shutdown) = watch::channel(Control::Running);
    let result = timeout(
        Duration::from_secs(3),
        drive(
            &mut socket,
            &mut Secret("fixture-secret".into()),
            &Weak::new(),
            &sender,
            &mut shutdown,
            &mut State::default(),
            &mut HashSet::new(),
            &mut receiver().1,
            &VoiceManager::default(),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Outcome::Retry { reset: false, .. }));
    drop(socket);
    server.await.unwrap();
}

#[tokio::test]
async fn ready_reports_the_time_spent_session_then_beats() {
    let (address, listener) = listen().await;
    let server = tokio::spawn(async move {
        let (mut socket, mut frames) = accept(listener).await;
        assert_eq!(read(&mut socket).await["op"], 2);
        let mut ready: Value =
            serde_json::from_str(include_str!("../fixtures/ready.json")).unwrap();
        ready["user"]["id"] = json!(1_175_185_049_000_000_001u64);
        for value in [
            json!({"op":10,"d":{"heartbeat_interval":100000}}),
            json!({"op":0,"s":1,"t":"READY","d":ready}),
        ] {
            write(&mut socket, &mut frames, value).await;
        }
        let mut time_spent = read(&mut socket).await;
        while time_spent["op"] != 41 {
            time_spent = read(&mut socket).await;
        }
        let properties = properties::client_properties();
        assert_eq!(
            time_spent["d"]["session_id"],
            properties["client_heartbeat_session_id"]
        );
        assert_eq!(
            time_spent["d"]["client_launch_id"],
            properties["client_launch_id"]
        );
        assert!(
            time_spent["d"]["initialization_timestamp"]
                .as_u64()
                .unwrap()
                > 0
        );
        let heartbeat = read(&mut socket).await;
        assert_eq!(heartbeat["op"], 40);
        assert_eq!(heartbeat["d"]["seq"], 1);
        write(&mut socket, &mut frames, json!({"op":7,"d":null})).await;
    });
    let mut socket = connect(address).await;
    let (sender, mut events) = futures::channel::mpsc::unbounded();
    let (_control, mut shutdown) = watch::channel(Control::Running);
    let mut state = State::default();
    let result = timeout(
        Duration::from_secs(3),
        drive(
            &mut socket,
            &mut Secret("fixture-secret".into()),
            &Weak::new(),
            &sender,
            &mut shutdown,
            &mut state,
            &mut HashSet::new(),
            &mut receiver().1,
            &VoiceManager::default(),
        ),
    )
    .await
    .unwrap();
    assert!(matches!(result, Outcome::Reconnect));
    assert_eq!(state.user_id, Some(1_175_185_049_000_000_001));
    assert_eq!(state.session_id.as_deref(), Some("fixture-session"));
    assert!(matches!(events.next().await, Some(Event::Ready(_))));
    server.await.unwrap();
}
