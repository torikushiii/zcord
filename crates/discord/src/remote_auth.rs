// https://docs.discord.food/remote-authentication/desktop
mod captcha;

use super::{Secret, properties, rest};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use captcha::{Captcha, Page};
use futures::{
    SinkExt, StreamExt,
    channel::mpsc::{self, UnboundedSender},
};
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey, pkcs8::EncodePublicKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::{
    net::TcpStream,
    sync::watch,
    task::JoinHandle,
    time::{Instant, Interval, MissedTickBehavior, interval, sleep, sleep_until, timeout},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Message, client::IntoClientRequest, handshake::client::Request, http::HeaderValue,
    },
};

const GATEWAY: &str = "wss://remote-auth-gateway.discord.gg/?v=2";
const LOGIN: &str = "https://discord.com/api/v9/users/@me/remote-auth/login";
const MAX_FAILURES: u32 = 5;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const RECEIVER_CHECK: Duration = Duration::from_secs(1);
const CAPTCHA_MESSAGE: &str = "Discord asked for a captcha. Log in with a token instead.";
const CAPTCHA_REFUSED: &str = "Discord didn't take the captcha. Try again.";
const CAPTCHA_EXPIRED: &str = "Nobody solved the captcha in time. Try again.";

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct RemoteAuth {
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl RemoteAuth {
    pub fn start(
        runtime: &tokio::runtime::Handle,
    ) -> (
        RemoteAuth,
        futures::channel::mpsc::UnboundedReceiver<RemoteAuthEvent>,
    ) {
        let (sender, receiver) = mpsc::unbounded();
        let (shutdown, stopped) = watch::channel(false);
        let task = runtime.spawn(run(Endpoints::default(), sender, stopped));
        (RemoteAuth { shutdown, task }, receiver)
    }
}

impl Drop for RemoteAuth {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
        self.task.abort();
    }
}

#[derive(Debug)]
pub enum RemoteAuthEvent {
    Qr(String),
    Scanned(RemoteUser),
    Captcha(String),
    Token(Secret),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteUser {
    pub id: u64,
    pub discriminator: String,
    pub avatar: Option<String>,
    pub username: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QrMatrix {
    pub width: usize,
    pub dark: Vec<bool>,
}

pub fn qr_matrix(text: &str) -> Option<QrMatrix> {
    let code = qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::M).ok()?;
    let width = code.width();
    let dark = code
        .into_colors()
        .into_iter()
        .map(|color| color == qrcode::Color::Dark)
        .collect();
    Some(QrMatrix { width, dark })
}

struct Endpoints {
    gateway: String,
    login: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            gateway: GATEWAY.to_owned(),
            login: LOGIN.to_owned(),
        }
    }
}

enum Outcome {
    Stop,
    Done,
    Fresh,
    Retry { healthy: bool },
    Fatal(String),
}

enum Read {
    Close(Option<u16>),
    Failed,
}

async fn run(
    endpoints: Endpoints,
    sender: UnboundedSender<RemoteAuthEvent>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut failures = 0u32;
    loop {
        if *shutdown.borrow() || sender.is_closed() {
            return;
        }
        match attempt(&endpoints, &sender, &mut shutdown).await {
            Outcome::Stop | Outcome::Done => return,
            Outcome::Fatal(message) => {
                let _ = sender.unbounded_send(RemoteAuthEvent::Failed(message));
                return;
            }
            Outcome::Fresh => failures = 0,
            Outcome::Retry { healthy } => {
                if healthy {
                    failures = 0;
                } else {
                    failures += 1;
                    if failures >= MAX_FAILURES {
                        let _ = sender.unbounded_send(RemoteAuthEvent::Failed(
                            "Could not connect to Discord for QR login.".to_owned(),
                        ));
                        return;
                    }
                }
            }
        }
        if failures > 0 {
            let delay = Duration::from_secs(1 << (failures - 1));
            tokio::select! {
                biased;
                _ = shutdown.changed() => return,
                _ = sleep(delay) => {}
            }
        }
    }
}

async fn attempt(
    endpoints: &Endpoints,
    sender: &UnboundedSender<RemoteAuthEvent>,
    shutdown: &mut watch::Receiver<bool>,
) -> Outcome {
    let Some(request) = gateway_request(&endpoints.gateway) else {
        return Outcome::Retry { healthy: false };
    };
    let connection = tokio::select! {
        biased;
        _ = shutdown.changed() => return Outcome::Stop,
        result = timeout(CONNECT_TIMEOUT, connect_async(request)) => result,
    };
    let Ok(Ok((mut socket, _))) = connection else {
        return Outcome::Retry { healthy: false };
    };

    let hello = tokio::select! {
        biased;
        _ = shutdown.changed() => return Outcome::Stop,
        result = timeout(CONNECT_TIMEOUT, read_payload(&mut socket)) => result,
    };
    let payload = match hello {
        Ok(Ok(payload)) => payload,
        Ok(Err(Read::Close(code))) => return close_outcome(code, false),
        Ok(Err(Read::Failed)) | Err(_) => return Outcome::Retry { healthy: false },
    };
    let Ok(hello) = serde_json::from_str::<Value>(&payload) else {
        return Outcome::Retry { healthy: false };
    };
    if hello.get("op").and_then(Value::as_str) != Some("hello") {
        return Outcome::Retry { healthy: false };
    }
    let Some(heartbeat_ms) = hello
        .get("heartbeat_interval")
        .and_then(Value::as_u64)
        .filter(|ms| *ms > 0 && *ms <= 300_000)
    else {
        return Outcome::Retry { healthy: false };
    };
    let timeout_ms = hello
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .filter(|ms| *ms >= 1_000 && *ms <= 600_000)
        .unwrap_or(180_000);

    let key = match tokio::task::spawn_blocking(generate_key).await {
        Ok(Ok(key)) => key,
        _ => return Outcome::Fatal("Could not create a key for QR login.".to_owned()),
    };
    let public = RsaPublicKey::from(&key);
    let Ok(der) = public.to_public_key_der() else {
        return Outcome::Fatal("Could not encode the key for QR login.".to_owned());
    };
    let der = der.as_bytes();
    let fingerprint = fingerprint(der);
    let init = json!({
        "op": "init",
        "encoded_public_key": STANDARD.encode(der),
    });
    if send(&mut socket, init).await.is_err() {
        return Outcome::Retry { healthy: false };
    }

    let mut heartbeat = interval(Duration::from_millis(heartbeat_ms));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let mut receiver_check = interval(RECEIVER_CHECK);
    receiver_check.tick().await;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut awaiting_ack = false;
    let mut reached_qr = false;

    loop {
        let message = tokio::select! {
            biased;
            _ = shutdown.changed() => return Outcome::Stop,
            _ = receiver_check.tick() => {
                if sender.is_closed() {
                    return Outcome::Stop;
                }
                continue;
            }
            _ = sleep_until(deadline) => return Outcome::Fresh,
            _ = heartbeat.tick() => {
                if awaiting_ack {
                    return Outcome::Retry { healthy: reached_qr };
                }
                if send(&mut socket, json!({"op": "heartbeat"})).await.is_err() {
                    return Outcome::Retry { healthy: reached_qr };
                }
                awaiting_ack = true;
                continue;
            }
            message = socket.next() => message,
        };
        let message = match message {
            Some(Ok(message)) => message,
            Some(Err(_)) | None => {
                return Outcome::Retry {
                    healthy: reached_qr,
                };
            }
        };
        let text = match message {
            Message::Text(text) => text,
            Message::Close(frame) => {
                return close_outcome(frame.map(|frame| frame.code.into()), reached_qr);
            }
            Message::Ping(_) => {
                if socket.flush().await.is_err() {
                    return Outcome::Retry {
                        healthy: reached_qr,
                    };
                }
                continue;
            }
            Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => continue,
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            log::debug!("Ignoring malformed remote auth frame");
            continue;
        };
        let Some(op) = value.get("op").and_then(Value::as_str) else {
            continue;
        };
        match op {
            "hello" => {}
            "nonce_proof" => {
                let Some(nonce) = value.get("encrypted_nonce").and_then(Value::as_str) else {
                    return Outcome::Retry {
                        healthy: reached_qr,
                    };
                };
                let Some(nonce) = decrypt(&key, nonce) else {
                    return Outcome::Retry {
                        healthy: reached_qr,
                    };
                };
                let proof = json!({
                    "op": "nonce_proof",
                    "nonce": URL_SAFE_NO_PAD.encode(nonce),
                });
                if send(&mut socket, proof).await.is_err() {
                    return Outcome::Retry {
                        healthy: reached_qr,
                    };
                }
            }
            "pending_remote_init" => {
                let Some(remote) = value.get("fingerprint").and_then(Value::as_str) else {
                    return Outcome::Retry {
                        healthy: reached_qr,
                    };
                };
                if remote != fingerprint {
                    log::warn!("Remote auth fingerprint did not match the generated key");
                    return Outcome::Retry {
                        healthy: reached_qr,
                    };
                }
                let event = RemoteAuthEvent::Qr(format!("https://discord.com/ra/{fingerprint}"));
                if !emit(sender, event) {
                    return Outcome::Stop;
                }
                reached_qr = true;
            }
            "pending_ticket" => {
                let Some(payload) = value.get("encrypted_user_payload").and_then(Value::as_str)
                else {
                    return Outcome::Retry {
                        healthy: reached_qr,
                    };
                };
                let Some(user) =
                    decrypt(&key, payload).and_then(|bytes| parse_user_payload(&bytes))
                else {
                    return Outcome::Retry {
                        healthy: reached_qr,
                    };
                };
                if !emit(sender, RemoteAuthEvent::Scanned(user)) {
                    return Outcome::Stop;
                }
            }
            "pending_login" => {
                let Some(ticket) = value.get("ticket").and_then(Value::as_str) else {
                    return Outcome::Retry {
                        healthy: reached_qr,
                    };
                };
                let ticket = Ticket {
                    login: &endpoints.login,
                    ticket,
                    key: &key,
                };
                match trade(ticket, &mut socket, &mut heartbeat, sender, shutdown).await {
                    Traded::Token(token) => {
                        if !emit(sender, RemoteAuthEvent::Token(Secret(token))) {
                            return Outcome::Stop;
                        }
                        return Outcome::Done;
                    }
                    Traded::Stop => return Outcome::Stop,
                    Traded::Failed(message) => return Outcome::Fatal(message),
                }
            }
            "cancel" => return Outcome::Fresh,
            "heartbeat_ack" => awaiting_ack = false,
            other => log::debug!("Ignoring remote auth op {other}"),
        }
    }
}

fn close_outcome(code: Option<u16>, healthy: bool) -> Outcome {
    // 4003 is the normal end of a QR session (~2-3 minutes), not an error.
    match code {
        Some(4003) => Outcome::Fresh,
        _ => Outcome::Retry { healthy },
    }
}

fn gateway_request(url: &str) -> Option<Request> {
    let mut request = url.into_client_request().ok()?;
    for (name, value) in [
        ("user-agent", properties::user_agent()),
        ("origin", "https://discord.com"),
        ("accept-language", properties::ACCEPT_LANGUAGE),
        ("cache-control", "no-cache"),
        ("pragma", "no-cache"),
    ] {
        request
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    Some(request)
}

async fn read_payload(socket: &mut Socket) -> Result<String, Read> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => return Ok(text),
            Some(Ok(Message::Close(frame))) => {
                return Err(Read::Close(frame.map(|frame| frame.code.into())));
            }
            Some(Ok(Message::Ping(_))) => {
                if socket.flush().await.is_err() {
                    return Err(Read::Failed);
                }
            }
            Some(Ok(_)) => {}
            Some(Err(_)) | None => return Err(Read::Failed),
        }
    }
}

async fn send(socket: &mut Socket, value: Value) -> Result<(), ()> {
    timeout(SEND_TIMEOUT, socket.send(Message::Text(value.to_string())))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

fn generate_key() -> Result<RsaPrivateKey, rsa::Error> {
    RsaPrivateKey::new(&mut rand::thread_rng(), 2048)
}

fn fingerprint(spki: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(spki))
}

fn decrypt(key: &RsaPrivateKey, encrypted: &str) -> Option<Vec<u8>> {
    let ciphertext = STANDARD.decode(encrypted).ok()?;
    key.decrypt(Oaep::new::<Sha256>(), &ciphertext).ok()
}

fn parse_user_payload(payload: &[u8]) -> Option<RemoteUser> {
    let payload = std::str::from_utf8(payload).ok()?;
    // Usernames may contain ':', so only the first three separators are split.
    let mut parts = payload.splitn(4, ':');
    let id = parts.next()?.parse().ok()?;
    let discriminator = parts.next()?.to_owned();
    let avatar = parts.next()?;
    let username = parts.next()?.to_owned();
    Some(RemoteUser {
        id,
        discriminator,
        avatar: if avatar == "0" {
            None
        } else {
            Some(avatar.to_owned())
        },
        username,
    })
}

struct Ticket<'a> {
    login: &'a str,
    ticket: &'a str,
    key: &'a RsaPrivateKey,
}

enum Traded {
    Token(String),
    Stop,
    Failed(String),
}

enum Exchange {
    Token(String),
    Captcha(Captcha),
}

/// The gateway keeps getting heartbeats while a captcha is being solved.
async fn trade(
    ticket: Ticket<'_>,
    socket: &mut Socket,
    heartbeat: &mut Interval,
    sender: &UnboundedSender<RemoteAuthEvent>,
    shutdown: &mut watch::Receiver<bool>,
) -> Traded {
    let mut solved: Option<(Captcha, String)> = None;
    for shown in 0..=captcha::TRIES {
        let solution = solved
            .as_ref()
            .map(|(captcha, solution)| (captcha, solution.as_str()));
        let captcha = match exchange_ticket(&ticket, solution).await {
            Ok(Exchange::Token(token)) => return Traded::Token(token),
            Ok(Exchange::Captcha(captcha)) => captcha,
            Err(message) => return Traded::Failed(message),
        };
        if shown == captcha::TRIES {
            return Traded::Failed(CAPTCHA_REFUSED.to_owned());
        }
        let Ok(page) = Page::serve(&captcha).await else {
            return Traded::Failed("Couldn't show Discord's captcha.".to_owned());
        };
        if !emit(sender, RemoteAuthEvent::Captcha(page.url.clone())) {
            return Traded::Stop;
        }
        let waiting = timeout(captcha::WAIT, page.solution());
        tokio::pin!(waiting);
        let mut receiver_check = interval(RECEIVER_CHECK);
        let mut open = true;
        let solution = loop {
            tokio::select! {
                biased;
                _ = shutdown.changed() => return Traded::Stop,
                _ = receiver_check.tick() => {
                    if sender.is_closed() {
                        return Traded::Stop;
                    }
                }
                solution = &mut waiting => match solution {
                    Ok(solution) => break solution,
                    Err(_) => return Traded::Failed(CAPTCHA_EXPIRED.to_owned()),
                },
                _ = heartbeat.tick(), if open => {
                    open = send(socket, json!({"op": "heartbeat"})).await.is_ok();
                }
                message = socket.next(), if open => open = matches!(message, Some(Ok(_))),
            }
        };
        solved = Some((captcha, solution));
    }
    unreachable!("the last try returns before showing another captcha")
}

async fn exchange_ticket(
    ticket: &Ticket<'_>,
    solved: Option<(&Captcha, &str)>,
) -> Result<Exchange, String> {
    let headers =
        rest::client_headers().map_err(|_| "Could not prepare the QR login request.".to_owned())?;
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Could not prepare the QR login request.".to_owned())?;
    let mut request = client
        .post(ticket.login)
        .header("x-super-properties", properties::super_properties())
        .header(reqwest::header::REFERER, "https://discord.com/login")
        .header(reqwest::header::ORIGIN, "https://discord.com");
    if let Some((captcha, solution)) = solved {
        for (name, value) in captcha.headers(solution) {
            request = request.header(name, value);
        }
    }
    let response = request
        .json(&json!({ "ticket": ticket.ticket }))
        .send()
        .await
        .map_err(|_| "Could not reach Discord to finish QR login.".to_owned())?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    if status.as_u16() == 400 {
        if let Some(captcha) = Captcha::from_body(&body) {
            return Ok(Exchange::Captcha(captcha));
        }
        if ["captcha_key", "captcha_sitekey"]
            .iter()
            .any(|field| body.get(*field).is_some_and(|value| !value.is_null()))
        {
            return Err(CAPTCHA_MESSAGE.to_owned());
        }
    }
    if !status.is_success() {
        return Err(format!(
            "Discord rejected the QR login (HTTP {}).",
            status.as_u16()
        ));
    }
    let encrypted = body
        .get("encrypted_token")
        .and_then(Value::as_str)
        .ok_or_else(|| "Discord sent an unreadable login response.".to_owned())?;
    decrypt(ticket.key, encrypted)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|token| !token.is_empty())
        .map(Exchange::Token)
        .ok_or_else(|| "Discord sent an unreadable login response.".to_owned())
}

fn emit(sender: &UnboundedSender<RemoteAuthEvent>, event: RemoteAuthEvent) -> bool {
    sender.unbounded_send(event).is_ok()
}

#[cfg(test)]
mod tests;
