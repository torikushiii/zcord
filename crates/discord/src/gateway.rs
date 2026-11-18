use super::commands::{Command, Commands, Kind, Requests, voice_state};
use super::voice::VoiceManager;
use super::{Control, Event, FatalError, Inner, Secret, model, properties, settings_proto};
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
use zstd::stream::raw::{InBuffer, Operation, OutBuffer};

mod dispatch;
mod etf;
mod qos;
use dispatch::dispatch;
use qos::{Activity, Qos};

const QUERY: &str = "encoding=etf&v=9&compress=zstd-stream";
const GATEWAY: &str = "wss://gateway.discord.gg/?encoding=etf&v=9&compress=zstd-stream";
const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
const HELLO_TIMEOUT: Duration = Duration::from_secs(30);
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REAUTHS: u32 = 3;
const RESUME_WINDOW: Duration = Duration::from_secs(3 * 60);
const MAX_FAILURES_BEFORE_RESET: u32 = 4;
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct CommandReceiver {
    receiver: tokio::sync::mpsc::UnboundedReceiver<(u64, Command)>,
    generation: Arc<AtomicU64>,
    presence: Arc<std::sync::Mutex<super::presence::Presence>>,
    presence_after: Instant,
    requests: Requests,
    qos: Qos,
    time_spent_session: Option<(String, u64)>,
}

impl CommandReceiver {
    fn new(
        receiver: tokio::sync::mpsc::UnboundedReceiver<(u64, Command)>,
        generation: Arc<AtomicU64>,
        presence: Arc<std::sync::Mutex<super::presence::Presence>>,
    ) -> Self {
        Self {
            receiver,
            generation,
            presence,
            presence_after: Instant::now(),
            requests: Requests::default(),
            qos: Qos::default(),
            time_spent_session: None,
        }
    }

    fn presence_payload(&self) -> Option<Value> {
        self.presence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .payload(now_ms())
    }

    fn presence_command(&self) -> Option<Command> {
        let presence = self
            .presence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(super::commands::presence(
            presence.payload(now_ms())?,
            presence.revision,
        ))
    }

    fn activity_woke(&mut self, state: &State) -> bool {
        let focused = self
            .presence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .focused();
        self.qos
            .woke_by(Activity::new(focused, state.voice_connected))
    }

    fn heartbeat(&mut self, state: &State) -> Value {
        self.activity_woke(state);
        self.qos.heartbeat(state.sequence)
    }

    fn auth(&self, token: &Secret, state: &mut State) -> Value {
        if state.can_resume(Instant::now()) {
            json!({"op": 6, "d": {
                "token": token.expose(), "session_id": state.session_id, "seq": state.sequence
            }})
        } else {
            state.clear();
            let focused = self
                .presence
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .focused();
            let mut identify =
                properties::identify(token.expose(), focused || state.voice_connected);
            if let Some(mut presence) = self.presence_payload() {
                identify["d"]["presence"] = presence["d"].take();
            }
            identify
        }
    }

    fn time_spent(&mut self) -> Value {
        let (session_id, initialized) = properties::heartbeat_session();
        let payload = json!({"op": 41, "d": {
            "initialization_timestamp": initialized, "session_id": session_id,
            "client_launch_id": properties::launch_id()
        }});
        self.time_spent_session = Some((session_id, initialized));
        payload
    }

    fn rotated_time_spent(&mut self, state: &State) -> Option<Value> {
        let (known, _) = self.time_spent_session.as_ref()?;
        if *known == properties::heartbeat_session().0 {
            return None;
        }
        let focused = self
            .presence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .focused();
        if !focused && !state.voice_connected {
            return None;
        }
        Some(self.time_spent())
    }

    fn ready(&self) {
        self.generation.fetch_or(1, Ordering::Release);
    }

    fn disconnected(&mut self) {
        let _ = self
            .generation
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
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

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

struct Inflater(zstd::stream::raw::Decoder<'static>);

impl Inflater {
    fn new() -> Option<Self> {
        zstd::stream::raw::Decoder::new().ok().map(Self)
    }

    fn push(&mut self, bytes: &[u8]) -> Result<Vec<u8>, ()> {
        let mut input = InBuffer::around(bytes);
        let mut output = Vec::with_capacity(bytes.len().saturating_mul(4).max(4096));
        loop {
            if output.len() == output.capacity() {
                output.reserve(output.len());
            }
            let (read, written) = (input.pos(), output.len());
            let mut buffer = OutBuffer::around_pos(&mut output, written);
            self.0.run(&mut input, &mut buffer).map_err(|_| ())?;
            let full = buffer.pos() == buffer.capacity();
            let progressed = input.pos() > read || buffer.pos() > written;
            if output.len() > MAX_PAYLOAD {
                return Err(());
            }
            if input.pos() == bytes.len() && !full {
                return Ok(output);
            }
            if !progressed && !full {
                return Err(());
            }
        }
    }
}

struct Backoff {
    current: Duration,
    failures: u32,
}

impl Backoff {
    const MIN: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(60);

    fn fail(&mut self) -> Duration {
        self.failures += 1;
        let growth = self
            .current
            .mul_f64(2.0 * rand::thread_rng().gen_range(0.0..1.0));
        self.current = (self.current + growth).min(Self::MAX);
        self.current
    }

    fn succeed(&mut self) {
        *self = Self::default();
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            current: Self::MIN,
            failures: 0,
        }
    }
}

#[derive(Default)]
struct State {
    sequence: u64,
    session_id: Option<String>,
    user_id: Option<u64>,
    voice_connected: bool,
    resume_url: Option<String>,
    last_ack: Option<Instant>,
    settings_version: Option<u64>,
    backoff: Backoff,
}
impl State {
    fn clear(&mut self) {
        self.sequence = 0;
        self.session_id = None;
        self.voice_connected = false;
    }

    fn can_resume(&self, now: Instant) -> bool {
        self.session_id.is_some()
            && self
                .last_ack
                .is_none_or(|ack| now.saturating_duration_since(ack) <= RESUME_WINDOW)
    }

    fn forward_settings_update(&mut self, update: &model::SettingsUpdate) -> bool {
        let version = settings_version(&update.settings.proto);
        if !update.partial
            && self
                .settings_version
                .is_some_and(|current| version.is_none_or(|next| next < current))
        {
            return false;
        }
        self.settings_version = self.settings_version.max(version);
        true
    }
}

fn settings_version(encoded: &str) -> Option<u64> {
    use base64::{Engine, engine::general_purpose::STANDARD};

    let proto = STANDARD.decode(encoded).ok()?;
    super::settings_proto::decode(&proto).ok()?;
    super::settings_proto::data_version(&proto).ok().flatten()
}

enum Outcome {
    Retry { reset: bool, delay: Duration },
    Reconnect,
    Fatal(FatalError),
    Stop,
}
fn retry() -> Outcome {
    Outcome::Retry {
        reset: false,
        delay: Duration::ZERO,
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
        },
        4008 => Outcome::Retry {
            reset: false,
            delay: Duration::from_secs(60),
        },
        _ => retry(),
    }
}

pub(super) async fn run(
    mut token: Secret,
    sender: UnboundedSender<Event>,
    mut shutdown: watch::Receiver<Control>,
    owner: Weak<Inner>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<(u64, Command)>,
    generation: Arc<AtomicU64>,
    voice: VoiceManager,
    presence: Arc<std::sync::Mutex<super::presence::Presence>>,
) {
    let mut commands = CommandReceiver::new(receiver, generation, presence);
    if owner.upgrade().is_some_and(|inner| inner.rest.is_err()) {
        let _ = sender.unbounded_send(Event::Fatal(FatalError::Other(
            "Could not initialize REST client".into(),
        )));
        voice.shutdown();
        close_session(&owner, Control::Shutdown);
        return;
    }
    let mut state = State::default();
    let mut ignored = HashSet::new();
    loop {
        if *shutdown.borrow() != Control::Running {
            break;
        }
        let outcome = attempt(
            &mut token,
            &owner,
            &sender,
            &mut shutdown,
            &mut state,
            &mut ignored,
            &mut commands,
            &voice,
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
                voice.shutdown();
                close_session(&owner, control);
                let _ = sender.unbounded_send(Event::Fatal(error));
                return;
            }
            Outcome::Reconnect => {
                if sender.unbounded_send(Event::Reconnecting).is_err() {
                    close_session(&owner, Control::Shutdown);
                    return;
                }
            }
            Outcome::Retry { reset, delay } => {
                if reset {
                    state.clear();
                }
                if sender.unbounded_send(Event::Reconnecting).is_err() {
                    close_session(&owner, Control::Shutdown);
                    return;
                }
                let delay = state.backoff.fail().max(delay);
                if state.backoff.failures > MAX_FAILURES_BEFORE_RESET {
                    state.clear();
                }
                tokio::select! { _ = shutdown.changed() => break, _ = sleep(delay) => {} }
            }
        }
    }
    voice.shutdown();
    if *shutdown.borrow() == Control::InvalidToken {
        let _ = sender.unbounded_send(Event::Fatal(FatalError::InvalidToken));
    }
    close_session(&owner, Control::Shutdown);
}

fn sync_voice_recording(commands: &CommandReceiver, voice: &VoiceManager) -> Option<Value> {
    let allow = commands
        .presence
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .proto()
        .and_then(|proto| settings_proto::decode(proto).ok())
        .and_then(|settings| settings.allow_voice_recording)
        .unwrap_or(true);
    voice.set_allow_voice_recording(allow)
}

/// The old token stops working once READY carries `auth_token`.
/// https://docs.discord.food/gateway/gateway-events#ready
fn rotate_token(token: &mut Secret, owner: &Weak<Inner>, refreshed: &Secret) -> bool {
    if let Some(inner) = owner.upgrade()
        && let Ok(rest) = &inner.rest
        && rest.set_token(refreshed.expose()).is_err()
    {
        log::warn!("Ignoring an unusable rotated auth token");
        return false;
    }
    *token = refreshed.clone();
    log::info!("Gateway rotated the auth token");
    true
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
    url.set_query(Some(QUERY));
    url.set_fragment(None);
    Some(url.into())
}

fn gateway_request(url: &str) -> Option<Request> {
    let mut request = url.into_client_request().ok()?;
    for (name, value) in [
        ("user-agent", properties::user_agent()),
        ("origin", "https://discord.com"),
        ("accept-encoding", "gzip, deflate, br, zstd"),
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
    token: &mut Secret,
    owner: &Weak<Inner>,
    sender: &UnboundedSender<Event>,
    shutdown: &mut watch::Receiver<Control>,
    state: &mut State,
    ignored: &mut HashSet<String>,
    commands: &mut CommandReceiver,
    voice: &VoiceManager,
) -> Outcome {
    let url = state.resume_url.as_deref().unwrap_or(GATEWAY);
    let Some(request) = gateway_request(url) else {
        state.resume_url = None;
        return retry();
    };
    let connection = tokio::select! {
        biased;
        _ = shutdown.changed() => return Outcome::Stop,
        result = timeout(Duration::from_secs(30), connect_async(request)) => result,
    };
    let Ok(Ok((mut socket, _))) = connection else {
        state.resume_url = None;
        return retry();
    };
    let outcome = drive(
        &mut socket,
        token,
        owner,
        sender,
        shutdown,
        state,
        ignored,
        commands,
        voice,
    )
    .await;
    commands.disconnected();
    // 1000/1001 invalidate sessions; use an application close code when intending to resume.
    let code = if matches!(outcome, Outcome::Retry { .. } | Outcome::Reconnect) {
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
        socket.send(Message::Binary(etf::encode(&value))),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())
}

async fn drive(
    socket: &mut Socket,
    token: &mut Secret,
    owner: &Weak<Inner>,
    sender: &UnboundedSender<Event>,
    shutdown: &mut watch::Receiver<Control>,
    state: &mut State,
    ignored: &mut HashSet<String>,
    commands: &mut CommandReceiver,
    voice: &VoiceManager,
) -> Outcome {
    let mut outgoing = Commands::new();
    outgoing.presence_after = commands.presence_after;
    let Some(mut inflater) = Inflater::new() else {
        return retry();
    };
    let hello_deadline = Instant::now() + HELLO_TIMEOUT;
    let mut interval: Option<Duration> = None;
    let mut next_heartbeat = Instant::now();
    let mut beating = false;
    let mut awaiting_ack = false;
    let mut healthy = false;
    if send(socket, commands.auth(token, state)).await.is_err() {
        return retry();
    }
    outgoing.record_send(Instant::now());
    let mut ready_deadline = Instant::now() + READY_TIMEOUT;
    let mut reauths = 0;
    loop {
        let next_command = outgoing.next_send();
        let incoming = tokio::select! {
            biased;
            _ = shutdown.changed() => return Outcome::Stop,
            _ = sleep_until(hello_deadline), if interval.is_none() => {
                log::warn!("Gateway did not say hello in time");
                state.resume_url = None;
                return retry();
            }
            _ = sleep_until(ready_deadline), if !healthy => {
                log::warn!("Gateway did not finish logging in in time");
                state.resume_url = None;
                return retry();
            }
            _ = sleep_until(next_heartbeat), if interval.is_some() => {
                if awaiting_ack {
                    log::warn!("Gateway heartbeat acknowledgement timed out");
                    return retry();
                }
                if healthy
                    && let Some(time_spent) = commands.rotated_time_spent(state)
                    && send(socket, time_spent).await.is_err()
                {
                    return retry();
                }
                if send(socket, commands.heartbeat(state)).await.is_err() { return retry(); }
                beating = true;
                awaiting_ack = true;
                next_heartbeat = Instant::now() + interval.unwrap_or_default();
                continue;
            }
            command = commands.receiver.recv() => {
                let Some((generation, command)) = command else { return Outcome::Stop };
                let current = healthy && generation == commands.generation.load(Ordering::Acquire);
                if !current {
                    outgoing.push(false, command);
                } else if matches!(command.kind, Kind::Activity) {
                    if commands.activity_woke(state)
                        && send(socket, commands.heartbeat(state)).await.is_err()
                    {
                        return retry();
                    }
                } else if let Some(command) = commands.requests.admit(command, Instant::now()) {
                    outgoing.push(true, command);
                }
                continue;
            }
            _ = sleep_until(next_command.unwrap_or_else(|| Instant::now() + Duration::from_secs(60))), if next_command.is_some() && healthy => {
                if let Some(mut command) = outgoing.pop() {
                    commands.presence_after = outgoing.presence_after;
                    if let Some(revision) = command.presence_revision {
                        let (current, payload) = {
                            let presence = commands.presence.lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            (presence.revision, presence.payload(now_ms()))
                        };
                        let Some(payload) = payload else {
                            command.complete(Err(super::Error::SettingsOutdated));
                            continue;
                        };
                        if current != revision {
                            let Some(stale) = outgoing.hand_off(command) else { continue };
                            command = stale;
                            command.presence_revision = Some(current);
                        }
                        command.payload = payload;
                    }
                    let result = send(socket, std::mem::take(&mut command.payload)).await;
                    command.complete(result.map_err(|_| super::Error::Network));
                    if result.is_err() { return retry(); }
                }
                continue;
            }
            incoming = socket.next() => incoming,
        };
        let Some(Ok(incoming)) = incoming else {
            state.resume_url = None;
            return retry();
        };
        let bytes = match incoming {
            Message::Binary(bytes) => match inflater.push(&bytes) {
                Ok(bytes) => bytes,
                Err(()) => return retry(),
            },
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
        let Some(payload) = etf::decode(&bytes)
            .ok()
            .and_then(|value| serde_json::from_value::<Payload>(value).ok())
        else {
            log::warn!("Malformed gateway envelope");
            return retry();
        };
        if let Some(sequence) = payload.s {
            state.sequence = sequence;
        }
        match payload.op {
            10 => {
                let Some(ms) = payload
                    .d
                    .get("heartbeat_interval")
                    .and_then(Value::as_u64)
                    .filter(|n| *n > 0 && *n <= 300_000)
                else {
                    return retry();
                };
                let every = Duration::from_millis(ms);
                interval = Some(every);
                beating = false;
                awaiting_ack = false;
                next_heartbeat =
                    Instant::now() + every.mul_f64(rand::thread_rng().gen_range(0.0..1.0));
            }
            11 => {
                awaiting_ack = false;
                state.last_ack = Some(Instant::now());
            }
            1 => {
                if send(socket, commands.heartbeat(state)).await.is_err() {
                    return retry();
                }
                if beating && let Some(every) = interval {
                    next_heartbeat = Instant::now() + every;
                }
            }
            7 => return Outcome::Reconnect,
            9 => {
                if payload.d.as_bool() != Some(true) {
                    state.clear();
                }
                reauths += 1;
                if reauths > MAX_REAUTHS {
                    log::warn!("Gateway kept invalidating the session");
                    return retry();
                }
                commands.disconnected();
                outgoing = Commands::new();
                outgoing.presence_after = commands.presence_after;
                healthy = false;
                if sender.unbounded_send(Event::Reconnecting).is_err() {
                    return Outcome::Stop;
                }
                if send(socket, commands.auth(token, state)).await.is_err() {
                    return retry();
                }
                outgoing.record_send(Instant::now());
                ready_deadline = Instant::now() + READY_TIMEOUT;
            }
            0 => {
                let kind = payload.t.as_deref().unwrap_or("");
                if kind == "READY" {
                    match serde_json::from_value::<model::Ready>(payload.d) {
                        Ok(mut ready) => {
                            if let Some(refreshed) = &ready.auth_token
                                && !rotate_token(token, owner, refreshed)
                            {
                                ready.auth_token = None;
                            }
                            if let Some(id) = &ready.installation_id
                                && let Some(inner) = owner.upgrade()
                                && let Ok(rest) = &inner.rest
                            {
                                rest.set_installation_id(id);
                            }
                            state.backoff.succeed();
                            state.last_ack = Some(Instant::now());
                            state.session_id = Some(ready.session_id.clone());
                            state.user_id = Some(ready.user.id);
                            state.voice_connected = false;
                            state.resume_url =
                                ready.resume_gateway_url.as_deref().and_then(resume_url);
                            state.settings_version = ready
                                .user_settings_proto
                                .as_deref()
                                .and_then(settings_version);
                            healthy = true;
                            reauths = 0;
                            commands.ready();
                            {
                                let mut presence = commands
                                    .presence
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                presence.set_animated_emojis(
                                    ready
                                        .guilds
                                        .iter()
                                        .flat_map(|guild| guild.emojis.iter().flatten())
                                        .filter(|emoji| emoji.animated)
                                        .map(|emoji| emoji.id),
                                );
                                presence.replace(ready.user_settings_proto.as_deref());
                            }
                            if let Some(command) = commands.presence_command() {
                                outgoing.push(true, command);
                            }
                            if let Some(command) = commands.requests.ready() {
                                outgoing.push(true, command);
                            }
                            sync_voice_recording(commands, voice);
                            voice.gateway_ready(ready.user.id, &ready.session_id, true);
                            if let Some(payload) = voice.take_pending_op4() {
                                outgoing.push(true, voice_state(payload));
                            }
                            log::info!(
                                "Gateway ready: {} guilds, {} users",
                                ready.guilds.len(),
                                ready.users.len()
                            );
                            // Delivered before any write that can fail: a retry after that
                            // resumes, and RESUMED alone never seeds the app or saves the token.
                            if sender
                                .unbounded_send(Event::Ready(Box::new(ready)))
                                .is_err()
                            {
                                return Outcome::Stop;
                            }
                            let time_spent = commands.time_spent();
                            if send(socket, time_spent).await.is_err()
                                || send(socket, commands.heartbeat(state)).await.is_err()
                            {
                                return retry();
                            }
                            outgoing.record_send(Instant::now());
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
                    state.backoff.succeed();
                    state.last_ack = Some(Instant::now());
                    healthy = true;
                    reauths = 0;
                    commands.ready();
                    if let Some(command) = commands.presence_command() {
                        outgoing.push(true, command);
                    }
                    if let Some(command) = commands.requests.resumed() {
                        outgoing.push(true, command);
                    }
                    if let Some(payload) = voice.resumed_op4() {
                        outgoing.push(true, voice_state(payload));
                    }
                    if let Some(command) = voice.resumed_unpause() {
                        outgoing.push(true, command);
                    }
                    if sender.unbounded_send(Event::Resumed).is_err() {
                        return Outcome::Stop;
                    }
                } else if kind == "VOICE_STATE_UPDATE" {
                    let session_id = payload
                        .d
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    match serde_json::from_value::<model::VoiceState>(payload.d.clone()) {
                        Ok(voice_update) => {
                            if Some(voice_update.user_id) == state.user_id
                                && session_id.is_some()
                                && session_id == state.session_id
                            {
                                state.voice_connected = voice_update.channel_id.is_some();
                                if commands.activity_woke(state)
                                    && healthy
                                    && send(socket, commands.heartbeat(state)).await.is_err()
                                {
                                    return retry();
                                }
                            }
                            voice.voice_state(&voice_update, session_id);
                            if sender
                                .unbounded_send(Event::VoiceStateUpdate(Box::new(voice_update)))
                                .is_err()
                            {
                                return Outcome::Stop;
                            }
                        }
                        Err(error) => log::warn!("Skipping malformed voice state: {error}"),
                    }
                } else if kind == "VOICE_SERVER_UPDATE" {
                    let guild_id = payload.d.get("guild_id").and_then(model::id_value);
                    let token = payload.d.get("token").and_then(Value::as_str);
                    if let Some(token) = token {
                        let endpoint = payload
                            .d
                            .get("endpoint")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        if let Some(guild_id) = guild_id {
                            voice.voice_server(guild_id, Secret(token.to_owned()), endpoint);
                        } else {
                            let channel_id = payload.d.get("channel_id").and_then(model::id_value);
                            voice.dm_voice_server(channel_id, Secret(token.to_owned()), endpoint);
                        }
                    }
                } else if kind == "STREAM_CREATE" {
                    let key = payload.d.get("stream_key").and_then(Value::as_str);
                    let rtc_server_id = payload.d.get("rtc_server_id").and_then(model::id_value);
                    if let (Some(key), Some(rtc_server_id)) = (key, rtc_server_id)
                        && key.len() <= 128
                    {
                        let rtc_channel_id =
                            payload.d.get("rtc_channel_id").and_then(model::id_value);
                        voice.stream_created(key, rtc_server_id, rtc_channel_id);
                    }
                } else if kind == "STREAM_SERVER_UPDATE" {
                    let key = payload.d.get("stream_key").and_then(Value::as_str);
                    let token = payload.d.get("token").and_then(Value::as_str);
                    if let (Some(key), Some(token)) = (key, token)
                        && key.len() <= 128
                    {
                        let endpoint = payload.d.get("endpoint").and_then(Value::as_str);
                        voice.stream_server(key, token, endpoint);
                    }
                } else if kind == "STREAM_UPDATE" {
                    let key = payload.d.get("stream_key").and_then(Value::as_str);
                    let viewers = payload
                        .d
                        .get("viewer_ids")
                        .and_then(Value::as_array)
                        .map(|ids| ids.iter().filter_map(model::id_value).collect())
                        .unwrap_or_default();
                    if let Some(key) = key.filter(|key| key.len() <= 128) {
                        voice.stream_updated(key, viewers);
                    }
                } else if kind == "STREAM_DELETE" {
                    let key = payload.d.get("stream_key").and_then(Value::as_str);
                    if let Some(key) = key.filter(|key| key.len() <= 128) {
                        let reason = payload
                            .d
                            .get("reason")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        voice.stream_deleted(key, reason);
                    }
                } else {
                    match kind {
                        "GUILD_CREATE" | "GUILD_DELETE" => {
                            if let Some(guild_id) = payload.d.get("id").and_then(model::id_value) {
                                commands.requests.guild_changed(guild_id);
                            }
                        }
                        "CALL_CREATE" | "CALL_UPDATE" | "CALL_DELETE" => {
                            if let Some(channel_id) =
                                payload.d.get("channel_id").and_then(model::id_value)
                            {
                                commands.requests.call_seen(channel_id);
                            }
                        }
                        _ => {}
                    }
                    let Some(event) = dispatch(kind, payload.d, ignored) else {
                        continue;
                    };
                    if let Event::SettingsUpdate(update) = &event
                        && update.settings.kind == 1
                    {
                        let changed = commands
                            .presence
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .update(&update.settings.proto, update.partial);
                        if matches!(changed, Ok(true))
                            && let Some(command) = commands.presence_command()
                        {
                            outgoing.push(true, command);
                        }
                        if let Some(payload) = sync_voice_recording(commands, voice) {
                            outgoing.push(true, voice_state(payload));
                        }
                        if !state.forward_settings_update(update) {
                            continue;
                        }
                    }
                    if sender.unbounded_send(event).is_err() {
                        return Outcome::Stop;
                    }
                }
            }
            _ => {}
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

#[cfg(test)]
mod tests;
