use super::signaling::{
    binary_frame, identify_payload, ready_payload, resume_payload, select_protocol_payload,
    speaking_payload,
};
use super::*;
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::protocol::CloseFrame};

fn config() -> Config {
    Config {
        guild_id: 10,
        channel_id: 20,
        user_id: 30,
        session_id: Secret("fixture-session".into()),
        token: Secret("fixture-token".into()),
        server: "fixture.invalid".into(),
        ws_url: "wss://fixture.invalid/?v=9".into(),
    }
}

#[test]
fn private_voice_identify_and_resume_use_channel_as_server_id() {
    let mut config = config();
    config.guild_id = 0;
    let identify = identify_payload(&config);
    let resume = resume_payload(&config, Some(7));
    for payload in [identify, resume] {
        assert_eq!(payload["d"]["server_id"], "20");
        assert_eq!(payload["d"]["channel_id"], "20");
    }
}

#[test]
fn early_ssrc_mapping_survives_the_session_description() {
    let mut connection = Connection::new(config(), AudioFlags::new());
    connection.ssrc_users.insert(4242, 30);
    connection.user_ssrcs.insert(30, 4242);
    let (events, _receiver) = futures::channel::mpsc::unbounded();
    let session = json!({
        "mode": "aead_aes256_gcm_rtpsize",
        "secret_key": vec![0u8; 32],
        "dave_protocol_version": 1,
    });
    assert!(connection.start_session(&session, 1, &events).is_ok());
    assert_eq!(connection.ssrc_users.get(&4242), Some(&30));
    assert_eq!(connection.user_ssrcs.get(&30), Some(&4242));
}

#[test]
fn dave_framing_helpers_match_gateway() {
    assert!(!ready_on_prepare(0, 0, true));
    assert!(ready_on_prepare(1, 0, true));
    assert!(ready_on_prepare(1, 1, false));
    assert!(!ready_on_prepare(1, 1, true));

    let ready = ready_payload(7);
    assert_eq!(ready["op"], 23);
    assert_eq!(ready["d"]["transition_id"], 7);
    assert!(ready["d"].get("protocol_version").is_none());

    assert_eq!(binary_frame(26, &[1, 2]), vec![26, 1, 2]);
    assert_eq!(binary_frame(28, &[3, 4]), vec![28, 3, 4]);

    assert_eq!(
        commit_welcome_body(vec![1, 2], Some(vec![3, 4])),
        vec![1, 2, 3, 4]
    );
    assert_eq!(commit_welcome_body(vec![1, 2], None), vec![1, 2]);
}

#[test]
fn close_codes_never_leave_the_call() {
    for code in [4014, 4021, 4022] {
        assert!(matches!(
            close_outcome(code),
            Outcome::Fatal { leave: false, .. }
        ));
    }
    for code in [1006, 4004, 4006, 4009, 4011, 4015, 4016, 4017] {
        assert!(matches!(close_outcome(code), Outcome::Reconnect));
    }
}

#[test]
fn gateway_payload_shapes() {
    let identify = identify_payload(&config());
    assert_eq!(identify["op"], 0);
    assert_eq!(identify["d"]["server_id"], "10");
    assert_eq!(identify["d"]["channel_id"], "20");
    assert_eq!(identify["d"]["user_id"], "30");
    assert_eq!(identify["d"]["session_id"], "fixture-session");
    assert_eq!(identify["d"]["video"], true);
    assert_eq!(
        identify["d"]["streams"],
        json!([{"type": "video", "rid": "100", "quality": 100}])
    );
    assert_eq!(
        identify["d"]["max_dave_protocol_version"],
        davey::DAVE_PROTOCOL_VERSION
    );

    let resume = resume_payload(&config(), None);
    assert_eq!(resume["op"], 7);
    assert_eq!(resume["d"]["token"], "fixture-token");
    assert_eq!(resume["d"]["seq_ack"], -1);

    let heartbeat = heartbeat_payload(Some(7));
    assert_eq!(heartbeat["op"], 3);
    assert!(heartbeat["d"]["t"].as_u64().unwrap() > 0);
    assert_eq!(heartbeat["d"]["seq_ack"], 7);
    assert_eq!(heartbeat_payload(None)["d"]["seq_ack"], -1);

    let speaking = speaking_payload(99, 1);
    assert_eq!(speaking["op"], 5);
    assert_eq!(speaking["d"]["speaking"], 1);
    assert_eq!(speaking["d"]["ssrc"], 99);
    assert_eq!(speaking["d"]["delay"], 0);

    let select = select_protocol_payload(
        "198.51.100.7",
        50000,
        Mode::Aes256Gcm,
        "d6b92f64-40df-48eb-8bce-7facb043149a",
    );
    assert_eq!(select["op"], 1);
    assert_eq!(select["d"]["protocol"], "udp");
    let data = json!({"address": "198.51.100.7", "port": 50000, "mode": "aead_aes256_gcm_rtpsize"});
    assert_eq!(select["d"]["data"], data);
    for key in ["address", "port", "mode"] {
        assert_eq!(select["d"][key], data[key]);
    }
    assert_eq!(
        select["d"]["rtc_connection_id"],
        "d6b92f64-40df-48eb-8bce-7facb043149a"
    );
    assert_eq!(
        select["d"]["experiments"],
        json!(["fixed_keyframe_interval"])
    );
    let codecs = select["d"]["codecs"].as_array().unwrap();
    assert_eq!(
        codecs[0],
        json!({"name": "opus", "type": "audio", "priority": 1000, "payload_type": 120})
    );
    let video: Vec<_> = codecs[1..]
        .iter()
        .map(|codec| {
            (
                codec["name"].as_str().unwrap(),
                codec["priority"].as_u64().unwrap(),
                codec["payload_type"].as_u64().unwrap(),
                codec["rtx_payload_type"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        video,
        [
            ("H265", 1000, 101, 102),
            ("H264", 2000, 103, 104),
            ("VP8", 3000, 105, 106)
        ]
    );
    assert_eq!(codecs[2]["payload_type"], VIDEO_PAYLOAD);
    assert_eq!(codecs[2]["rtx_payload_type"], RTX_PAYLOAD);
}

#[test]
fn reidentify_closes_drop_the_resumable_session() {
    let mut connection = Connection::new(config(), AudioFlags::new());
    connection.handshake_done = true;
    connection.seq = Some(4);
    assert!(connection.can_resume());
    assert!(REIDENTIFY_CLOSES.contains(&4006));
    connection.forget_session();
    assert!(!connection.can_resume());
}

#[tokio::test]
async fn local_handshake_reaches_select_protocol() {
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_address = ws_listener.local_addr().unwrap();
    let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let udp_address = udp.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (tcp, _) = ws_listener.accept().await.unwrap();
        let mut socket = accept_async(tcp).await.unwrap();
        socket
            .send(Message::Text(
                json!({"op": 8, "d": {"v": 9, "heartbeat_interval": 60000}}).to_string(),
            ))
            .await
            .unwrap();
        let identify: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(identify["op"], 0);
        assert_eq!(identify["d"]["channel_id"], "20");
        assert_eq!(identify["d"]["max_dave_protocol_version"], 1);
        socket
            .send(Message::Text(
                json!({"op": 2, "d": {
                    "ssrc": 4242,
                    "ip": "127.0.0.1",
                    "port": udp_address.port(),
                    "modes": ["aead_aes256_gcm_rtpsize", "aead_xchacha20_poly1305_rtpsize"],
                    "experiments": [],
                    "streams": [],
                }})
                .to_string(),
            ))
            .await
            .unwrap();

        let version: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(version, json!({"op": 16, "d": {}}));

        let mut buffer = [0u8; 128];
        let (size, peer) = udp.recv_from(&mut buffer).await.unwrap();
        assert_eq!(size, 74);
        assert_eq!(&buffer[..2], &[0, 1]);
        assert_eq!(&buffer[4..8], &4242u32.to_be_bytes());
        udp.send_to(&udp::discovery_response("198.51.100.2", 50000), peer)
            .await
            .unwrap();

        let select: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(select["op"], 1);
        assert_eq!(select["d"]["data"]["address"], "198.51.100.2");
        assert_eq!(select["d"]["data"]["port"], 50000);
        assert_eq!(select["d"]["data"]["mode"], "aead_aes256_gcm_rtpsize");
        socket
            .send(Message::Close(Some(CloseFrame {
                code: 4014.into(),
                reason: "fixture".into(),
            })))
            .await
            .unwrap();
    });

    let (mut socket, _) = connect_async(format!("ws://{ws_address}")).await.unwrap();
    let (events, _receiver) = futures::channel::mpsc::unbounded();
    let (_shutdown, mut shutdown) = watch::channel(false);
    let mut connection = Connection::new(
        Config {
            ws_url: format!("ws://{ws_address}"),
            ..config()
        },
        AudioFlags::new(),
    );
    let outcome = timeout(
        Duration::from_secs(10),
        connection.drive(&mut socket, &events, &mut shutdown, false),
    )
    .await
    .unwrap();
    assert!(matches!(outcome, Outcome::Fatal { .. }));
    assert_eq!(connection.peer, Some(udp_address));
    server.await.unwrap();
}
