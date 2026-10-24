use super::commands::{Command, Commands};
use super::{Control, Event, FatalError, Inner, Secret, model, properties};
use flate2::{Decompress, FlushDecompress};
use futures::{SinkExt, StreamExt, channel::mpsc::UnboundedSender};
use rand::Rng;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpStream,
    sync::watch,
    time::{Instant, sleep, sleep_until, timeout},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        Message, client::IntoClientRequest, handshake::client::Request, http::HeaderValue,
        protocol::CloseFrame,
    },
};

const GATEWAY: &str = "wss://gateway.discord.gg/?encoding=json&v=9&compress=zlib-stream";
const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct CommandReceiver {
    receiver: tokio::sync::mpsc::UnboundedReceiver<(u64, Command)>,
    generation: Arc<AtomicU64>,
}

impl CommandReceiver {
    fn ready(&self) {
        self.generation.fetch_or(1, Ordering::Release);
    }

    fn disconnected(&mut self) {
        let _ = self
            .generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                Some(if value % 2 == 1 {
                    value.wrapping_add(1)
                } else {
                    value
                })
            });
        while self.receiver.try_recv().is_ok() {
            log::debug!("Dropping gateway command on disconnect");
        }
    }
}

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
    receiver: tokio::sync::mpsc::UnboundedReceiver<(u64, Command)>,
    generation: Arc<AtomicU64>,
) {
    let mut commands = CommandReceiver {
        receiver,
        generation,
    };
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
        let outcome = attempt(
            &token.0,
            &sender,
            &mut shutdown,
            &mut state,
            &mut ignored,
            &mut commands,
        )
        .await;
        commands.disconnected();
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

fn gateway_request(url: &str) -> Option<Request> {
    let mut request = url.into_client_request().ok()?;
    for (name, value) in [
        ("user-agent", properties::USER_AGENT),
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

async fn attempt(
    token: &str,
    sender: &UnboundedSender<Event>,
    shutdown: &mut watch::Receiver<Control>,
    state: &mut State,
    ignored: &mut HashSet<String>,
    commands: &mut CommandReceiver,
) -> Outcome {
    let url = if state.can_resume() {
        state.resume_url.as_deref().unwrap_or(GATEWAY)
    } else {
        GATEWAY
    };
    let Some(request) = gateway_request(url) else {
        return retry();
    };
    let connection = tokio::select! {
        biased;
        _ = shutdown.changed() => return Outcome::Stop,
        result = timeout(Duration::from_secs(30), connect_async(request)) => result,
    };
    let Ok(Ok((mut socket, _))) = connection else {
        return retry();
    };
    let started = Instant::now();
    let mut outcome = drive(
        &mut socket,
        token,
        sender,
        shutdown,
        state,
        ignored,
        commands,
    )
    .await;
    commands.disconnected();
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
    commands: &mut CommandReceiver,
) -> Outcome {
    let mut outgoing = Commands::new();
    let mut decoder = Decoder::new();
    let mut identified = false;
    let mut interval = Duration::from_secs(60);
    let mut next_heartbeat = Instant::now() + interval;
    let mut pending_ack: Option<Instant> = None;
    let mut deadline = Instant::now() + Duration::from_secs(30);
    let mut healthy = false;
    loop {
        let next_command = outgoing.next_send();
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
            command = commands.receiver.recv() => {
                if let Some((generation, command)) = command {
                    outgoing.push(identified && healthy && generation == commands.generation.load(Ordering::Acquire), command);
                } else { return Outcome::Stop; }
                continue;
            }
            _ = sleep_until(next_command.unwrap_or_else(|| Instant::now() + Duration::from_secs(60))), if next_command.is_some() && identified && healthy => {
                if let Some(payload) = outgoing.pop() {
                    if send(socket, payload).await.is_err() { return retry(); }
                }
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
                    outgoing.record_send(Instant::now());
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
                                commands.ready();
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
                        commands.ready();
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
            "READY_SUPPLEMENTAL" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::ReadySupplemental(Box::new(v))),
            "PRESENCE_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::PresenceUpdate(Box::new(v))),
            "GUILD_MEMBERS_CHUNK" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::MembersChunk(Box::new(v))),
            "GUILD_MEMBER_LIST_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::MemberListUpdate(Box::new(v))),
            "GUILD_MEMBER_ADD" | "GUILD_MEMBER_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::MemberUpsert(Box::new(v))),
            "GUILD_MEMBER_REMOVE" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::MemberRemove(Box::new(v))),
            "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|v| Event::RoleUpsert(Box::new(v))),
            "GUILD_ROLE_DELETE" => id(&value, "guild_id")
                .zip(id(&value, "role_id"))
                .map(|(guild_id, role_id)| Event::RoleDelete { guild_id, role_id }),
            "USER_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|user| Event::UserUpdate(Box::new(user))),
            "USER_SETTINGS_PROTO_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|settings| Event::SettingsUpdate(Box::new(settings))),
            "MESSAGE_CREATE" => serde_json::from_value(value)
                .ok()
                .map(|m| Event::MessageCreate(Box::new(m))),
            "MESSAGE_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(|m| Event::MessageUpdate(Box::new(m))),
            "TYPING_START" => serde_json::from_value(value)
                .ok()
                .map(|t| Event::TypingStart(Box::new(t))),
            "MESSAGE_REACTION_ADD" => serde_json::from_value(value)
                .ok()
                .map(Event::MessageReactionAdd),
            "MESSAGE_REACTION_REMOVE" => serde_json::from_value(value)
                .ok()
                .map(Event::MessageReactionRemove),
            "MESSAGE_REACTION_REMOVE_ALL" => serde_json::from_value(value)
                .ok()
                .map(Event::MessageReactionRemoveAll),
            "MESSAGE_REACTION_REMOVE_EMOJI" => serde_json::from_value(value)
                .ok()
                .map(Event::MessageReactionRemoveEmoji),
            "MESSAGE_REACTION_ADD_MANY" => serde_json::from_value(value)
                .ok()
                .map(Event::MessageReactionAddMany),
            "MESSAGE_DELETE" => id(&value, "channel_id")
                .zip(id(&value, "id"))
                .map(|(channel_id, id)| Event::MessageDelete { channel_id, id }),
            "CHANNEL_CREATE" | "CHANNEL_UPDATE" | "THREAD_CREATE" | "THREAD_UPDATE" => {
                serde_json::from_value(value)
                    .ok()
                    .map(|c| Event::ChannelUpsert(Box::new(c)))
            }
            "CHANNEL_DELETE" | "THREAD_DELETE" => {
                id(&value, "id").map(|id| Event::ChannelDelete { id })
            }
            "THREAD_LIST_SYNC" => serde_json::from_value(value)
                .ok()
                .map(|sync| Event::ThreadListSync(Box::new(sync))),
            "GUILD_CREATE" | "GUILD_UPDATE" => serde_json::from_value::<model::Guild>(value)
                .ok()
                .map(|mut guild| {
                    if kind == "GUILD_CREATE" && guild.unavailable.is_none() {
                        guild.unavailable = Some(false);
                    }
                    Event::GuildUpsert(Box::new(guild))
                }),
            "GUILD_EMOJIS_UPDATE" => serde_json::from_value(value)
                .ok()
                .map(Event::GuildEmojisUpdate),
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
    fn typing_start_dispatches_with_member() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "TYPING_START",
            json!({"channel_id":"2","guild_id":"3","user_id":"4","timestamp":1770000000,
                "member":{"user":{"id":"4","username":"typer"},"roles":["5"]}}),
            &mut ignored,
        );
        assert!(matches!(
            event,
            Some(Event::TypingStart(typing))
                if typing.channel_id == 2
                    && typing.guild_id == Some(3)
                    && typing.user_id == 4
                    && typing.member.as_ref().and_then(|m| m.user.as_ref()).is_some_and(|u| u.id == 4)
        ));
        assert!(ignored.is_empty());
    }

    #[test]
    fn user_update_dispatches_nullable_user_fields() {
        let mut ignored = HashSet::new();
        let event = dispatch(
            "USER_UPDATE",
            json!({"id":"1","username":"new_handle","global_name":"New Name","avatar":null}),
            &mut ignored,
        );
        assert!(matches!(
            event,
            Some(Event::UserUpdate(user))
                if user.id == 1
                    && user.username == "new_handle"
                    && matches!(&user.global_name, Some(Some(name)) if name == "New Name")
                    && matches!(user.avatar, Some(None))
        ));
        assert!(ignored.is_empty());
    }

    #[test]
    fn reaction_dispatches_are_receive_only() {
        let mut ignored = HashSet::new();
        let value = json!({"channel_id":"1","message_id":"2","user_id":"3","emoji":{"id":"4","name":"custom","animated":true}});
        assert!(
            matches!(dispatch("MESSAGE_REACTION_ADD", value.clone(), &mut ignored), Some(Event::MessageReactionAdd(event)) if event.user_id == Some(3) && event.emoji.animated)
        );
        assert!(matches!(
            dispatch("MESSAGE_REACTION_REMOVE", value.clone(), &mut ignored),
            Some(Event::MessageReactionRemove(_))
        ));
        assert!(matches!(
            dispatch("MESSAGE_REACTION_REMOVE_EMOJI", value, &mut ignored),
            Some(Event::MessageReactionRemoveEmoji(_))
        ));
        assert!(
            matches!(dispatch("MESSAGE_REACTION_REMOVE_ALL", json!({"channel_id":"1","message_id":"2"}), &mut ignored), Some(Event::MessageReactionRemoveAll(event)) if event.channel_id == 1 && event.message_id == 2)
        );
        let many = json!({"channel_id":"1","message_id":"2","reactions":[false,{"users":["3","bad",4],"emoji":{"name":"ok"}}]});
        assert!(
            matches!(dispatch("MESSAGE_REACTION_ADD_MANY", many, &mut ignored), Some(Event::MessageReactionAddMany(event)) if event.reactions.len() == 1 && event.reactions[0].users == [3,4])
        );
        for kind in [
            "MESSAGE_REACTION_ADD",
            "MESSAGE_REACTION_REMOVE",
            "MESSAGE_REACTION_REMOVE_ALL",
            "MESSAGE_REACTION_REMOVE_EMOJI",
            "MESSAGE_REACTION_ADD_MANY",
        ] {
            assert!(dispatch(kind, json!({"message_id":"bad"}), &mut ignored).is_none());
        }
        assert!(ignored.is_empty());
    }

    #[test]
    fn social_dispatches_and_disconnect_generations() {
        let fixture: Value = serde_json::from_str(include_str!("fixtures/social.json")).unwrap();
        let mut ignored = HashSet::new();
        assert!(matches!(
            dispatch(
                "READY_SUPPLEMENTAL",
                fixture["supplemental"].clone(),
                &mut ignored
            ),
            Some(Event::ReadySupplemental(_))
        ));
        assert!(matches!(
            dispatch(
                "GUILD_MEMBERS_CHUNK",
                fixture["chunk"].clone(),
                &mut ignored
            ),
            Some(Event::MembersChunk(_))
        ));
        assert!(matches!(
            dispatch(
                "GUILD_MEMBER_LIST_UPDATE",
                fixture["list"].clone(),
                &mut ignored
            ),
            Some(Event::MemberListUpdate(_))
        ));
        for event in ["GUILD_MEMBER_ADD", "GUILD_MEMBER_UPDATE"] {
            assert!(matches!(
                dispatch(
                    event,
                    json!({"guild_id":"1","user":{"id":"2"}}),
                    &mut ignored
                ),
                Some(Event::MemberUpsert(_))
            ));
        }
        assert!(matches!(
            dispatch(
                "GUILD_MEMBER_REMOVE",
                json!({"guild_id":"1","user":{"id":"2"}}),
                &mut ignored
            ),
            Some(Event::MemberRemove(_))
        ));
        assert!(matches!(
            dispatch(
                "GUILD_ROLE_CREATE",
                json!({"guild_id":"1","role":{"id":"2"}}),
                &mut ignored
            ),
            Some(Event::RoleUpsert(_))
        ));
        assert!(matches!(
            dispatch(
                "GUILD_ROLE_DELETE",
                json!({"guild_id":"1","role_id":"2"}),
                &mut ignored
            ),
            Some(Event::RoleDelete {
                guild_id: 1,
                role_id: 2
            })
        ));
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let generation = Arc::new(AtomicU64::new(0));
        let mut commands = CommandReceiver {
            receiver,
            generation: generation.clone(),
        };
        commands.ready();
        assert_eq!(generation.load(Ordering::Acquire), 1);
        sender
            .send((1, super::super::commands::subscription(1, 2, &[])))
            .unwrap_or_else(|_| panic!("queue closed"));
        commands.disconnected();
        assert_eq!(generation.load(Ordering::Acquire), 2);
        assert!(commands.receiver.try_recv().is_err());
        commands.ready();
        assert_eq!(generation.load(Ordering::Acquire), 3);
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
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
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
                },
                &mut HashSet::new(),
                &mut CommandReceiver {
                    receiver,
                    generation,
                },
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
        let (_command_sender, command_receiver) = tokio::sync::mpsc::unbounded_channel();
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
                &mut CommandReceiver {
                    receiver: command_receiver,
                    generation: Arc::new(AtomicU64::new(0)),
                },
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
                },
            ),
        )
        .await
        .unwrap();
        assert!(matches!(result, Outcome::Retry { reset: false, .. }));
        drop(socket);
        server.await.unwrap();
    }
}
