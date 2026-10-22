use super::{Control, Event, FatalError, Inner, Secret, model, properties};
use flate2::{Decompress, FlushDecompress};
use futures::{SinkExt, StreamExt, channel::mpsc::UnboundedSender};
use rand::Rng;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{
    net::TcpStream,
    sync::watch,
    time::{Instant, sleep, sleep_until, timeout},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, protocol::CloseFrame},
};

const GATEWAY: &str = "wss://gateway.discord.gg/?encoding=json&v=9&compress=zlib-stream";
const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Discord's zlib-stream: one persistent inflater per connection, including across
/// sync-flush message boundaries.
struct Decoder {
    inflater: Decompress,
    pending: Vec<u8>,
}
impl Decoder {
    fn new() -> Self {
        Self {
            inflater: Decompress::new(true),
            pending: Vec::new(),
        }
    }
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, ()> {
        self.pending.extend_from_slice(bytes);
        if self.pending.len() > MAX_PAYLOAD {
            return Err(());
        }
        let mut messages = Vec::new();
        while let Some(end) = self
            .pending
            .windows(4)
            .position(|w| w == [0, 0, 255, 255])
            .map(|n| n + 4)
        {
            let mut output = Vec::new();
            let mut consumed = 0;
            loop {
                let mut buffer = [0; 8192];
                let before_in = self.inflater.total_in();
                let before_out = self.inflater.total_out();
                self.inflater
                    .decompress(
                        &self.pending[consumed..end],
                        &mut buffer,
                        FlushDecompress::Sync,
                    )
                    .map_err(|_| ())?;
                let read = (self.inflater.total_in() - before_in) as usize;
                let written = (self.inflater.total_out() - before_out) as usize;
                consumed += read;
                output.extend_from_slice(&buffer[..written]);
                if output.len() > MAX_PAYLOAD {
                    return Err(());
                }
                if consumed == end && written < buffer.len() {
                    break;
                }
                if read == 0 && written == 0 {
                    return Err(());
                }
            }
            self.pending.drain(..end);
            messages.push(output);
        }
        Ok(messages)
    }
}

#[derive(Default)]
struct State {
    sequence: Option<u64>,
    session_id: Option<String>,
    resume_url: Option<String>,
}
impl State {
    fn clear(&mut self) {
        *self = Self::default();
    }
    fn can_resume(&self) -> bool {
        self.sequence.is_some() && self.session_id.is_some()
    }
}

enum Outcome {
    Retry {
        reset: bool,
        delay: Duration,
        healthy: bool,
    },
    Fatal(FatalError),
    Stop,
}
fn retry() -> Outcome {
    Outcome::Retry {
        reset: false,
        delay: Duration::ZERO,
        healthy: false,
    }
}

fn close_outcome(code: u16) -> Outcome {
    match code {
        4004 => Outcome::Fatal(FatalError::InvalidToken),
        4010..=4016 => Outcome::Fatal(FatalError::Other(format!(
            "Gateway closed with code {code}"
        ))),
        1000 | 1001 | 4003 | 4006 | 4007 | 4009 => Outcome::Retry {
            reset: true,
            delay: Duration::ZERO,
            healthy: false,
        },
        4008 => Outcome::Retry {
            reset: false,
            delay: Duration::from_secs(60),
            healthy: false,
        },
        _ => retry(),
    }
}

pub(super) async fn run(
    token: Arc<Secret>,
    sender: UnboundedSender<Event>,
    mut shutdown: watch::Receiver<Control>,
    owner: Weak<Inner>,
) {
    if owner.upgrade().is_some_and(|inner| inner.rest.is_err()) {
        let _ = sender.unbounded_send(Event::Fatal(FatalError::Other(
            "Could not initialize REST client".into(),
        )));
        close_session(&owner, Control::Shutdown);
        return;
    }
    let mut state = State::default();
    let mut ignored = HashSet::new();
    let mut backoff = 1u64;
    loop {
        if *shutdown.borrow() != Control::Running {
            break;
        }
        let outcome = attempt(&token.0, &sender, &mut shutdown, &mut state, &mut ignored).await;
        match outcome {
            Outcome::Stop => break,
            Outcome::Fatal(error) => {
                let control = if matches!(error, FatalError::InvalidToken) {
                    Control::InvalidToken
                } else {
                    Control::Shutdown
                };
                close_session(&owner, control);
                let _ = sender.unbounded_send(Event::Fatal(error));
                return;
            }
            Outcome::Retry {
                reset,
                delay,
                healthy,
            } => {
                if reset {
                    state.clear();
                }
                if healthy {
                    backoff = 1;
                }
                if sender.unbounded_send(Event::Reconnecting).is_err() {
                    close_session(&owner, Control::Shutdown);
                    return;
                }
                let jitter = rand::thread_rng().gen_range(0..=250);
                let delay = delay
                    .max(Duration::from_millis(backoff * 1000 + jitter))
                    .min(Duration::from_secs(60));
                backoff = (backoff * 2).min(60);
                tokio::select! { _ = shutdown.changed() => break, _ = sleep(delay) => {} }
            }
        }
    }
    if *shutdown.borrow() == Control::InvalidToken {
        let _ = sender.unbounded_send(Event::Fatal(FatalError::InvalidToken));
    }
    close_session(&owner, Control::Shutdown);
}

fn close_session(owner: &Weak<Inner>, control: Control) {
    if let Some(inner) = owner.upgrade() {
        inner.control.send_replace(control);
    }
}

fn resume_url(url: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(url).ok()?;
    let host = url.host_str()?;
    if url.scheme() != "wss"
        || !(host == "discord.gg" || host.ends_with(".discord.gg"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    url.set_query(Some("encoding=json&v=9&compress=zlib-stream"));
    url.set_fragment(None);
    Some(url.into())
}

async fn attempt(
    token: &str,
    sender: &UnboundedSender<Event>,
    shutdown: &mut watch::Receiver<Control>,
    state: &mut State,
    ignored: &mut HashSet<String>,
) -> Outcome {
    let url = if state.can_resume() {
        state.resume_url.as_deref().unwrap_or(GATEWAY)
    } else {
        GATEWAY
    };
    let connection = tokio::select! {
        biased;
        _ = shutdown.changed() => return Outcome::Stop,
        result = timeout(Duration::from_secs(30), connect_async(url)) => result,
    };
    let Ok(Ok((mut socket, _))) = connection else {
        return retry();
    };
    let started = Instant::now();
    let mut outcome = drive(&mut socket, token, sender, shutdown, state, ignored).await;
    if let Outcome::Retry { healthy, .. } = &mut outcome {
        *healthy = started.elapsed() >= Duration::from_secs(60);
    }
    // 1000/1001 invalidate sessions; use an application close code when intending to resume.
    let code = if matches!(outcome, Outcome::Retry { .. }) {
        4000
    } else {
        1000
    };
    let _ = timeout(
        Duration::from_secs(2),
        socket.close(Some(CloseFrame {
            code: code.into(),
            reason: "".into(),
        })),
    )
    .await;
    outcome
}

async fn send(socket: &mut Socket, value: Value) -> Result<(), ()> {
    timeout(
        Duration::from_secs(10),
        socket.send(Message::Text(value.to_string())),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

async fn drive(
    socket: &mut Socket,
    token: &str,
    sender: &UnboundedSender<Event>,
    shutdown: &mut watch::Receiver<Control>,
    state: &mut State,
    ignored: &mut HashSet<String>,
) -> Outcome {
    let mut decoder = Decoder::new();
    let mut identified = false;
    let mut interval = Duration::from_secs(60);
    let mut next_heartbeat = Instant::now() + interval;
    let mut pending_ack: Option<Instant> = None;
    let mut deadline = Instant::now() + Duration::from_secs(30);
    let mut healthy = false;
    loop {
        let incoming = tokio::select! {
            biased;
            _ = shutdown.changed() => return Outcome::Stop,
            _ = sleep_until(deadline), if !healthy => return retry(),
            _ = sleep_until(next_heartbeat), if identified => {
                if pending_ack.is_some_and(|sent| sent.elapsed() >= interval) {
                    log::warn!("Gateway heartbeat acknowledgement timed out");
                    return Outcome::Retry { reset: false, delay: Duration::ZERO, healthy };
                }
                if send(socket, json!({"op": 1, "d": state.sequence})).await.is_err() { return retry(); }
                pending_ack.get_or_insert(Instant::now());
                next_heartbeat = Instant::now() + interval;
                continue;
            }
            incoming = socket.next() => incoming,
        };
        let Some(Ok(incoming)) = incoming else {
            return Outcome::Retry {
                reset: false,
                delay: Duration::ZERO,
                healthy,
            };
        };
        let payloads = match incoming {
            Message::Binary(bytes) => match decoder.push(&bytes) {
                Ok(messages) => messages,
                Err(()) => return retry(),
            },
            Message::Text(text) => vec![text.into_bytes()],
            Message::Close(frame) => {
                return close_outcome(frame.map_or(1006, |frame| frame.code.into()));
            }
            Message::Ping(_) => {
                if socket.flush().await.is_err() {
                    return retry();
                }
                continue;
            }
            _ => continue,
        };
        for bytes in payloads {
            let Ok(payload) = serde_json::from_slice::<Payload>(&bytes) else {
                log::warn!("Malformed gateway envelope");
                return retry();
            };
            if let Some(sequence) = payload.s {
                state.sequence = Some(sequence);
            }
            match payload.op {
                10 => {
                    if identified {
                        return retry();
                    }
                    let Some(ms) = payload
                        .d
                        .get("heartbeat_interval")
                        .and_then(Value::as_u64)
                        .filter(|n| *n > 0 && *n <= 300_000)
                    else {
                        return retry();
                    };
                    interval = Duration::from_millis(ms);
                    let jitter = rand::thread_rng().gen_range(0.0..1.0);
                    next_heartbeat = Instant::now() + interval.mul_f64(jitter);
                    let auth = if state.can_resume() {
                        json!({"op": 6, "d": {"token": token, "session_id": state.session_id, "seq": state.sequence}})
                    } else {
                        properties::identify(token)
                    };
                    identified = true;
                    deadline = Instant::now() + Duration::from_secs(60);
                    if send(socket, auth).await.is_err() {
                        return retry();
                    }
                }
                11 => pending_ack = None,
                1 => {
                    if !identified {
                        return retry();
                    }
                    if send(socket, json!({"op": 1, "d": state.sequence}))
                        .await
                        .is_err()
                    {
                        return retry();
                    }
                    pending_ack.get_or_insert(Instant::now());
                }
                7 => {
                    return Outcome::Retry {
                        reset: false,
                        delay: Duration::ZERO,
                        healthy,
                    };
                }
                9 => {
                    return Outcome::Retry {
                        reset: payload.d.as_bool() != Some(true),
                        delay: Duration::from_secs(rand::thread_rng().gen_range(1..=5)),
                        healthy: false,
                    };
                }
                0 => {
                    let kind = payload.t.as_deref().unwrap_or("");
                    if kind == "READY" {
                        match serde_json::from_value::<model::Ready>(payload.d) {
                            Ok(ready) => {
                                state.session_id = Some(ready.session_id.clone());
                                state.resume_url =
                                    ready.resume_gateway_url.as_deref().and_then(resume_url);
                                healthy = true;
                                log::info!(
                                    "Gateway ready: {} guilds, {} users",
                                    ready.guilds.len(),
                                    ready.users.len()
                                );
                                if sender
                                    .unbounded_send(Event::Ready(Box::new(ready)))
                                    .is_err()
                                {
                                    return Outcome::Stop;
                                }
                            }
                            // Reconnecting would re-identify on every backoff step with
                            // the same unparseable result, and repeated identifies get
                            // accounts flagged.
                            Err(error) => {
                                log::error!("Could not parse READY: {error}");
                                return Outcome::Fatal(FatalError::Other(format!(
                                    "Could not read Discord's startup data ({error})"
                                )));
                            }
                        }
                    } else if kind == "RESUMED" {
                        healthy = true;
                        if sender.unbounded_send(Event::Resumed).is_err() {
                            return Outcome::Stop;
                        }
                    } else if let Some(event) = dispatch(kind, payload.d, ignored) {
                        if sender.unbounded_send(event).is_err() {
                            return Outcome::Stop;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[derive(Deserialize)]
struct Payload {
    op: u8,
    #[serde(default)]
    d: Value,
    #[serde(default)]
    s: Option<u64>,
    #[serde(default)]
    t: Option<String>,
}

fn dispatch(kind: &str, value: Value, ignored: &mut HashSet<String>) -> Option<Event> {
    fn id(value: &Value, field: &str) -> Option<u64> {
        value.get(field).and_then(model::id_value)
    }
    let event =
        match kind {
            "USER_SETTINGS_PROTO_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|settings| Event::SettingsUpdate(Box::new(settings))),
            "MESSAGE_CREATE" => serde_json::from_value(value)
                .ok()
                .map(|m| Event::MessageCreate(Box::new(m))),
            "MESSAGE_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|m| Event::MessageUpdate(Box::new(m))),
            "MESSAGE_DELETE" => id(&value, "channel_id")
                .zip(id(&value, "id"))
                .map(|(channel_id, id)| Event::MessageDelete { channel_id, id }),
            "CHANNEL_CREATE" | "CHANNEL_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|c| Event::ChannelUpsert(Box::new(c))),
            "CHANNEL_DELETE" => id(&value, "id").map(|id| Event::ChannelDelete { id }),
            "GUILD_CREATE" | "GUILD_UPDATE" => serde_json::from_value::<model::Guild>(value)
                .ok()
                .map(|mut guild| {
                    if kind == "GUILD_CREATE" && guild.unavailable.is_none() {
                        guild.unavailable = Some(false);
                    }
                    Event::GuildUpsert(Box::new(guild))
                }),
            "GUILD_DELETE" => id(&value, "id").map(|id| Event::GuildDelete { id }),
            "MESSAGE_ACK" => {
                if value.get("ack_type").and_then(Value::as_u64).unwrap_or(0) != 0 {
                    return None;
                }
                id(&value, "channel_id").zip(id(&value, "message_id")).map(
                    |(channel_id, message_id)| Event::MessageAck {
                        channel_id,
                        message_id,
                    },
                )
            }
            _ => {
                if kind.len() <= 80
                    && kind.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
                    && ignored.insert(kind.to_owned())
                {
                    log::debug!("Ignoring gateway event {kind}");
                }
                return None;
            }
        };
    if event.is_none() {
        log::warn!("Skipping malformed supported gateway event");
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

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
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
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
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
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
        let mut state = State {
            sequence: Some(42),
            session_id: Some("fixture-session".into()),
            resume_url: None,
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
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(auth["op"], 2);
            let heartbeat: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(heartbeat["op"], 1);
            assert!(heartbeat["d"].is_null());
            let _ = socket.next().await;
        });
        let (mut socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
        let (sender, _events) = futures::channel::mpsc::unbounded();
        let (_control, mut shutdown) = watch::channel(Control::Running);
        let result = timeout(
            Duration::from_secs(3),
            drive(
                &mut socket,
                "fixture-secret",
                &sender,
                &mut shutdown,
                &mut State::default(),
                &mut HashSet::new(),
            ),
        )
        .await
        .unwrap();
        assert!(matches!(result, Outcome::Retry { reset: false, .. }));
        drop(socket);
        server.await.unwrap();
    }
}
