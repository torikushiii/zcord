use super::*;
use futures::channel::mpsc;
use rsa::pkcs8::DecodePublicKey;
use std::sync::OnceLock;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

fn test_key() -> &'static RsaPrivateKey {
    static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
    KEY.get_or_init(|| RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap())
}

fn encrypt(public: &RsaPublicKey, plaintext: &[u8]) -> String {
    STANDARD.encode(
        public
            .encrypt(&mut rand::thread_rng(), Oaep::new::<Sha256>(), plaintext)
            .unwrap(),
    )
}

#[test]
fn fingerprint_nonce_and_user_payload_round_trip() {
    let key = test_key();
    let public = RsaPublicKey::from(key);
    let der = public.to_public_key_der().unwrap();
    let fingerprint = fingerprint(der.as_bytes());
    assert_eq!(fingerprint.len(), 43);

    let nonce = b"fixture nonce";
    let encrypted = encrypt(&public, nonce);
    assert_eq!(decrypt(key, &encrypted).unwrap(), nonce);

    let payload = b"123456789012345678:0:0:name:with:colons";
    let encrypted = encrypt(&public, payload);
    let user = parse_user_payload(&decrypt(key, &encrypted).unwrap()).unwrap();
    assert_eq!(
        user,
        RemoteUser {
            id: 123456789012345678,
            discriminator: "0".into(),
            avatar: None,
            username: "name:with:colons".into(),
        }
    );
}

#[test]
fn user_payload_parsing_keeps_avatar_and_rejects_short_payloads() {
    let user = parse_user_payload(b"42:1234:abcdef:user").unwrap();
    assert_eq!(user.id, 42);
    assert_eq!(user.discriminator, "1234");
    assert_eq!(user.avatar.as_deref(), Some("abcdef"));
    assert_eq!(user.username, "user");
    assert!(parse_user_payload(b"42:1234:abcdef").is_none());
    assert!(parse_user_payload(b"not-an-id:0:0:user").is_none());
}

#[test]
fn qr_matrix_is_square_for_remote_auth_url() {
    let text = format!("https://discord.com/ra/{}", "a".repeat(43));
    let matrix = qr_matrix(&text).unwrap();
    assert!(matrix.width >= 21);
    assert_eq!(matrix.dark.len(), matrix.width * matrix.width);
}

#[tokio::test]
async fn local_gateway_emits_qr_then_scanned_user() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
        socket
            .send(Message::Text(
                json!({"op": "hello", "heartbeat_interval": 100_000, "timeout_ms": 120_000})
                    .to_string(),
            ))
            .await
            .unwrap();
        let init: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(init["op"], "init");
        let der = STANDARD
            .decode(init["encoded_public_key"].as_str().unwrap())
            .unwrap();
        let public = RsaPublicKey::from_public_key_der(&der).unwrap();
        let fingerprint = fingerprint(&der);
        let nonce = b"fixture nonce";
        socket
            .send(Message::Text(
                json!({"op": "nonce_proof", "encrypted_nonce": encrypt(&public, nonce)})
                    .to_string(),
            ))
            .await
            .unwrap();
        let proof: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(proof["op"], "nonce_proof");
        assert_eq!(
            URL_SAFE_NO_PAD
                .decode(proof["nonce"].as_str().unwrap())
                .unwrap(),
            nonce
        );
        socket
            .send(Message::Text(
                json!({"op": "pending_remote_init", "fingerprint": fingerprint}).to_string(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Text(
                json!({
                    "op": "pending_ticket",
                    "encrypted_user_payload": encrypt(&public, b"42:0:0:user"),
                })
                .to_string(),
            ))
            .await
            .unwrap();
        fingerprint
    });

    let (sender, mut events) = mpsc::unbounded();
    let (_shutdown, stopped) = watch::channel(false);
    let task = tokio::spawn(run(
        Endpoints {
            gateway: format!("ws://{address}"),
            login: "http://127.0.0.1:9/not-used".to_owned(),
        },
        sender,
        stopped,
    ));

    let qr = timeout(Duration::from_secs(10), events.next())
        .await
        .unwrap()
        .unwrap();
    let fingerprint = server.await.unwrap();
    match qr {
        RemoteAuthEvent::Qr(text) => {
            assert_eq!(text, format!("https://discord.com/ra/{fingerprint}"));
        }
        other => panic!("expected QR event, got {other:?}"),
    }
    let scanned = timeout(Duration::from_secs(10), events.next())
        .await
        .unwrap()
        .unwrap();
    match scanned {
        RemoteAuthEvent::Scanned(user) => {
            assert_eq!(user.username, "user");
            assert_eq!(user.avatar, None);
        }
        other => panic!("expected scanned event, got {other:?}"),
    }
    task.abort();
}

async fn accept_request(listener: &TcpListener) -> (TcpStream, String, Vec<u8>) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    let mut bytes = [0; 1024];
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let count = stream.read(&mut bytes).await.unwrap();
        assert!(count > 0 && request.len() < 16384);
        request.extend_from_slice(&bytes[..count]);
    }
    let head_end = request
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap()
        + 4;
    let head = String::from_utf8(request[..head_end].to_vec()).unwrap();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while request.len() < head_end + length {
        let count = stream.read(&mut bytes).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&bytes[..count]);
    }
    let body = request[head_end..head_end + length].to_vec();
    (stream, head, body)
}

#[tokio::test]
async fn local_login_exchange_returns_the_decrypted_token() {
    let login_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let login = format!(
        "http://{}/api/v9/users/@me/remote-auth/login",
        login_listener.local_addr().unwrap()
    );
    let gateway_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway = gateway_listener.local_addr().unwrap();
    let (key_sender, key_receiver) = oneshot::channel();

    let ws_server = tokio::spawn(fake_login_gateway(gateway_listener, key_sender));

    let http_server = tokio::spawn(async move {
        let public = key_receiver.await.unwrap();
        let (mut stream, head, body) = accept_login(&login_listener).await;
        let lower = head.to_ascii_lowercase();
        assert!(!lower.contains("\r\nauthorization:"));
        assert!(
            lower.contains("\r\nreferer: https://discord.com/login"),
            "{head}"
        );
        assert!(lower.contains("\r\norigin: https://discord.com"));
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"ticket": "fixture-ticket"})
        );
        let token = encrypt(&public, b"fixture-token");
        let body = json!({"encrypted_token": token}).to_string();
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    });

    let (sender, mut events) = mpsc::unbounded();
    let (_shutdown, stopped) = watch::channel(false);
    let task = tokio::spawn(run(
        Endpoints {
            gateway: format!("ws://{gateway}"),
            login,
        },
        sender,
        stopped,
    ));

    assert!(matches!(
        timeout(Duration::from_secs(10), events.next())
            .await
            .unwrap()
            .unwrap(),
        RemoteAuthEvent::Qr(_)
    ));
    assert!(matches!(
        timeout(Duration::from_secs(10), events.next())
            .await
            .unwrap()
            .unwrap(),
        RemoteAuthEvent::Scanned(_)
    ));
    let token = timeout(Duration::from_secs(10), events.next())
        .await
        .unwrap()
        .unwrap();
    match token {
        RemoteAuthEvent::Token(secret) => assert_eq!(secret.0, "fixture-token"),
        other => panic!("expected token event, got {other:?}"),
    }
    assert!(
        timeout(Duration::from_secs(10), events.next())
            .await
            .unwrap()
            .is_none()
    );
    ws_server.await.unwrap();
    http_server.await.unwrap();
    task.abort();
}

async fn fake_login_gateway(listener: TcpListener, key_sender: oneshot::Sender<RsaPublicKey>) {
    let (tcp, _) = listener.accept().await.unwrap();
    let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
    socket
        .send(Message::Text(
            json!({"op": "hello", "heartbeat_interval": 100_000, "timeout_ms": 120_000})
                .to_string(),
        ))
        .await
        .unwrap();
    let init: Value =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    let der = STANDARD
        .decode(init["encoded_public_key"].as_str().unwrap())
        .unwrap();
    let public = RsaPublicKey::from_public_key_der(&der).unwrap();
    let fingerprint = fingerprint(&der);
    socket
        .send(Message::Text(
            json!({"op": "nonce_proof", "encrypted_nonce": encrypt(&public, b"fixture nonce")})
                .to_string(),
        ))
        .await
        .unwrap();
    let _: Value =
        serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    socket
        .send(Message::Text(
            json!({"op": "pending_remote_init", "fingerprint": fingerprint}).to_string(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({
                "op": "pending_ticket",
                "encrypted_user_payload": encrypt(&public, b"42:0:0:user"),
            })
            .to_string(),
        ))
        .await
        .unwrap();
    key_sender.send(public).unwrap();
    socket
        .send(Message::Text(
            json!({"op": "pending_login", "ticket": "fixture-ticket"}).to_string(),
        ))
        .await
        .unwrap();
    while let Some(Ok(_)) = socket.next().await {}
}

async fn accept_login(listener: &TcpListener) -> (TcpStream, String, Vec<u8>) {
    loop {
        let request = accept_request(listener).await;
        if request
            .1
            .starts_with("POST /api/v9/users/@me/remote-auth/login ")
        {
            return request;
        }
    }
}

async fn respond_json(mut stream: TcpStream, status: &str, body: Value) {
    let body = body.to_string();
    stream
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stream.shutdown().await.unwrap();
}

async fn next_event(events: &mut mpsc::UnboundedReceiver<RemoteAuthEvent>) -> RemoteAuthEvent {
    timeout(Duration::from_secs(10), events.next())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn captcha_is_shown_then_the_ticket_is_retried_with_its_solution() {
    let login_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let login = format!(
        "http://{}/api/v9/users/@me/remote-auth/login",
        login_listener.local_addr().unwrap()
    );
    let gateway_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway = gateway_listener.local_addr().unwrap();
    let (key_sender, key_receiver) = oneshot::channel();
    let ws_server = tokio::spawn(fake_login_gateway(gateway_listener, key_sender));
    let http_server = tokio::spawn(async move {
        let public = key_receiver.await.unwrap();
        let (stream, head, _) = accept_login(&login_listener).await;
        assert!(!head.to_ascii_lowercase().contains("\r\nx-captcha-key:"));
        respond_json(
            stream,
            "400 Bad Request",
            json!({
                "captcha_key": ["captcha-required"],
                "captcha_service": "hcaptcha",
                "captcha_sitekey": "fixture-sitekey",
                "captcha_rqtoken": "fixture-rqtoken",
                "captcha_session_id": "fixture-session",
            }),
        )
        .await;
        let (stream, head, body) = accept_login(&login_listener).await;
        let lower = head.to_ascii_lowercase();
        assert!(lower.contains("\r\nx-captcha-key: solved\r\n"), "{head}");
        assert!(lower.contains("\r\nx-captcha-rqtoken: fixture-rqtoken\r\n"));
        assert!(lower.contains("\r\nx-captcha-session-id: fixture-session\r\n"));
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"ticket": "fixture-ticket"})
        );
        respond_json(
            stream,
            "200 OK",
            json!({"encrypted_token": encrypt(&public, b"fixture-token")}),
        )
        .await;
    });

    let (sender, mut events) = mpsc::unbounded();
    let (_shutdown, stopped) = watch::channel(false);
    let task = tokio::spawn(run(
        Endpoints {
            gateway: format!("ws://{gateway}"),
            login,
        },
        sender,
        stopped,
    ));

    assert!(matches!(
        next_event(&mut events).await,
        RemoteAuthEvent::Qr(_)
    ));
    assert!(matches!(
        next_event(&mut events).await,
        RemoteAuthEvent::Scanned(_)
    ));
    let RemoteAuthEvent::Captcha(url) = next_event(&mut events).await else {
        panic!("expected a captcha page");
    };
    let client = reqwest::Client::new();
    let html = client.get(&url).send().await.unwrap().text().await.unwrap();
    assert!(html.contains(r#""fixture-sitekey""#), "{html}");
    client.post(&url).body("solved").send().await.unwrap();
    match next_event(&mut events).await {
        RemoteAuthEvent::Token(secret) => assert_eq!(secret.0, "fixture-token"),
        other => panic!("expected token event, got {other:?}"),
    }
    http_server.await.unwrap();
    task.abort();
    ws_server.abort();
}

#[test]
fn secret_debug_is_redacted() {
    assert_eq!(
        format!("{:?}", Secret("fixture-token".into())),
        "[REDACTED]"
    );
    assert_eq!(
        format!(
            "{:?}",
            RemoteAuthEvent::Token(Secret("fixture-token".into()))
        ),
        "Token([REDACTED])"
    );
}
