mod streams;

use super::{
    audio::AudioFlags,
    emit,
    gateway::{Config, Connection, Outcome},
    stream::{
        self, FrameSlot, ScreenSource, StreamQuality, sender::SenderConfig, viewer::ViewerConfig,
    },
};
use crate::{Event, Secret, ShareEvent, StreamEvent, VoiceEvent, commands, model};
use futures::channel::mpsc::UnboundedSender;
use rand::Rng;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{sync::watch, time::sleep};

const CONNECT_ATTEMPTS: u32 = 5;
const OP4_FLAGS: u64 = 3;

struct Server {
    token: Secret,
    endpoint: Option<String>,
}

struct Task {
    shutdown: watch::Sender<bool>,
    guild_id: u64,
    channel_id: u64,
    endpoint: String,
}

// Private calls use the channel as the voice server identity, not the guild sentinel.
fn server_id(guild_id: u64, channel_id: u64) -> u64 {
    if guild_id == 0 { channel_id } else { guild_id }
}

struct StreamWatch {
    user_id: u64,
    key: String,
    server_id: u64,
    rtc_server_id: Option<u64>,
    rtc_channel_id: Option<u64>,
    token: Option<Secret>,
    endpoint: Option<String>,
    shutdown: Option<watch::Sender<bool>>,
    ended: Arc<AtomicBool>,
    started: bool,
}

struct StreamShare {
    key: String,
    server_id: u64,
    /// What's shared, which can change while live; `None` for a bare Go Live.
    live: Option<Arc<stream::share::LiveShare>>,
    rtc_server_id: Option<u64>,
    rtc_channel_id: Option<u64>,
    token: Option<Secret>,
    endpoint: Option<String>,
    shutdown: Option<watch::Sender<bool>>,
    ended: Arc<AtomicBool>,
    started: bool,
    keyframe: Arc<AtomicBool>,
    bitrate: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
    reannounce: Arc<AtomicBool>,
    failure: Arc<Mutex<Option<String>>>,
}

struct Shared {
    user_id: Option<u64>,
    mute: bool,
    deaf: bool,
    desired: Option<(u64, u64)>,
    confirmed: Option<(u64, u64)>,
    last_guild: Option<u64>,
    session_id: Option<Secret>,
    gateway_session_id: Option<Secret>,
    servers: HashMap<u64, Server>,
    task: Option<Task>,
    stream: Option<StreamWatch>,
    share: Option<StreamShare>,
    frames: FrameSlot,
    /// Outlives each share, so the app polls one slot for whatever it streams next.
    local_preview: Arc<stream::share::LocalPreview>,
    flags: AudioFlags,
    generation: u64,
    dirty: bool,
    restart: Option<&'static str>,
}

#[derive(Clone)]
pub(crate) struct VoiceManager {
    shared: Arc<Mutex<Shared>>,
    events: Option<UnboundedSender<Event>>,
    runtime: Option<tokio::runtime::Handle>,
    commands: Option<tokio::sync::mpsc::UnboundedSender<(u64, commands::Command)>>,
    gateway_generation: Option<Arc<AtomicU64>>,
}

impl Default for VoiceManager {
    fn default() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                user_id: None,
                mute: false,
                deaf: false,
                desired: None,
                confirmed: None,
                last_guild: None,
                session_id: None,
                gateway_session_id: None,
                servers: HashMap::new(),
                task: None,
                stream: None,
                share: None,
                frames: Arc::new(Mutex::new(None)),
                local_preview: Arc::default(),
                flags: AudioFlags::new(),
                generation: 0,
                dirty: false,
                restart: None,
            })),
            events: None,
            runtime: None,
            commands: None,
            gateway_generation: None,
        }
    }
}

impl VoiceManager {
    pub fn new(
        runtime: &tokio::runtime::Handle,
        events: UnboundedSender<Event>,
        commands: tokio::sync::mpsc::UnboundedSender<(u64, commands::Command)>,
        gateway_generation: Arc<AtomicU64>,
    ) -> Self {
        Self {
            events: Some(events),
            runtime: Some(runtime.clone()),
            commands: Some(commands),
            gateway_generation: Some(gateway_generation),
            ..Self::default()
        }
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn join(&self, guild_id: u64, channel_id: u64) -> Value {
        let mut shared = self.lock();
        let restart = match &shared.task {
            Some(task) if task.guild_id != guild_id || task.channel_id != channel_id => {
                "channel changed"
            }
            Some(_) => "rejoin requested",
            None if shared.desired.is_some() || shared.confirmed.is_some() => "rejoin requested",
            None => "join requested",
        };
        shared.restart = Some(restart);
        shared.servers.remove(&server_id(guild_id, channel_id));
        if let Some((guild, channel)) = shared.desired.or(shared.confirmed) {
            shared.servers.remove(&server_id(guild, channel));
        }
        if let Some(previous) = shared.last_guild {
            shared.servers.remove(&previous);
        }
        shared.desired = Some((guild_id, channel_id));
        shared.confirmed = None;
        shared.last_guild = Some(guild_id);
        shared.dirty = true;
        if let Some(task) = shared.task.take() {
            let _ = task.shutdown.send(true);
        }
        let payload = op4(&shared);
        drop(shared);
        self.emit_connecting(guild_id, channel_id);
        self.spawn_if_ready();
        payload
    }

    pub fn leave(&self) -> Option<Value> {
        let mut shared = self.lock();
        let had_voice =
            shared.desired.is_some() || shared.confirmed.is_some() || shared.task.is_some();
        if let Some((guild, channel)) = shared.desired.or(shared.confirmed) {
            shared.servers.remove(&server_id(guild, channel));
        }
        shared.desired = None;
        shared.confirmed = None;
        shared.session_id = None;
        shared.dirty = true;
        shared.restart = None;
        self.end_stream(&mut shared, Some("Left the call".into()));
        self.end_share(&mut shared, Some("Left the call".into()));
        if let Some(guild_id) = shared.last_guild {
            shared.servers.remove(&guild_id);
        }
        let payload = (shared.last_guild.is_some()).then(|| op4(&shared));
        if let Some(task) = shared.task.take() {
            let _ = task.shutdown.send(true);
        }
        drop(shared);
        if had_voice {
            self.emit(VoiceEvent::Disconnected { reason: None });
        }
        payload
    }

    pub fn set_mute(&self, mute: bool) -> Option<Value> {
        self.set_flags(mute, None)
    }

    pub fn set_deaf(&self, deaf: bool) -> Option<Value> {
        let mute = self.lock().mute;
        self.set_flags(mute, Some(deaf))
    }

    pub fn set_noise_suppression(&self, enabled: bool) {
        self.lock().flags.set_noise_suppression(enabled);
    }

    pub fn set_input_device(&self, name: Option<String>) {
        self.lock().flags.set_input_device(name);
    }

    pub fn set_output_device(&self, name: Option<String>) {
        self.lock().flags.set_output_device(name);
    }

    pub fn set_input_volume(&self, gain: f32) {
        self.lock().flags.set_input_gain(gain);
    }

    pub fn set_output_volume(&self, gain: f32) {
        self.lock().flags.set_output_gain(gain);
    }

    pub fn set_stream_volume(&self, gain: f32) {
        self.lock().flags.set_stream_volume(gain);
    }

    pub fn set_input_sensitivity(&self, threshold: Option<f32>) {
        self.lock().flags.set_sensitivity(threshold);
    }

    pub fn input_level(&self) -> Option<f32> {
        let shared = self.lock();
        shared.desired?;
        shared.flags.input_level()
    }

    pub fn input_gate_open(&self) -> bool {
        let shared = self.lock();
        shared.desired.is_some() && shared.flags.gate_open()
    }

    fn set_flags(&self, mute: bool, deaf: Option<bool>) -> Option<Value> {
        let mut shared = self.lock();
        shared.mute = mute;
        if let Some(deaf) = deaf {
            shared.deaf = deaf;
        }
        shared.flags.set(shared.mute, shared.deaf);
        shared.desired?;
        shared.dirty = true;
        Some(op4(&shared))
    }

    pub fn voice_state(&self, state: &model::VoiceState, session_id: Option<String>) {
        let mut shared = self.lock();
        if shared.user_id != Some(state.user_id) {
            return;
        }
        let was_joined =
            shared.desired.is_some() || shared.confirmed.is_some() || shared.task.is_some();
        if let (Some(ours), Some(theirs)) = (&shared.gateway_session_id, session_id.as_deref())
            && ours.expose() != theirs
        {
            if was_joined {
                shared.desired = None;
                shared.confirmed = None;
                shared.session_id = None;
                shared.dirty = false;
                shared.restart = None;
                self.end_stream(&mut shared, Some("Voice moved to another device".into()));
                self.end_share(&mut shared, Some("Voice moved to another device".into()));
                if let Some(task) = shared.task.take() {
                    let _ = task.shutdown.send(true);
                }
                drop(shared);
                self.emit(VoiceEvent::Disconnected {
                    reason: Some("Voice moved to another device".into()),
                });
            }
            return;
        }
        let guild_id = state.guild_id.unwrap_or(0);
        if guild_id == 0
            && shared
                .desired
                .or(shared.confirmed)
                .is_some_and(|(guild, _)| guild != 0)
        {
            return;
        }
        let Some(channel_id) = state.channel_id else {
            let was_joined = shared.desired.is_some() || shared.confirmed.is_some();
            shared.desired = None;
            shared.confirmed = None;
            shared.session_id = None;
            shared.dirty = false;
            shared.restart = None;
            self.end_stream(&mut shared, Some("Disconnected from voice".into()));
            self.end_share(&mut shared, Some("Disconnected from voice".into()));
            if let Some(task) = shared.task.take() {
                let _ = task.shutdown.send(true);
            }
            drop(shared);
            if was_joined {
                self.emit(VoiceEvent::Disconnected {
                    reason: Some("Disconnected from voice".into()),
                });
            }
            return;
        };
        if shared.desired.is_none() {
            let Some(current) = shared.confirmed else {
                return;
            };
            if current.0 != guild_id {
                return;
            }
            shared.desired = Some((guild_id, channel_id));
        }
        if shared
            .task
            .as_ref()
            .is_some_and(|task| task.guild_id != guild_id || task.channel_id != channel_id)
        {
            shared.restart = Some("channel changed");
        }
        if shared
            .confirmed
            .is_some_and(|previous| previous != (guild_id, channel_id))
        {
            self.end_stream(&mut shared, Some("Changed voice channel".into()));
            self.end_share(&mut shared, Some("Changed voice channel".into()));
        }
        shared.confirmed = Some((guild_id, channel_id));
        shared.session_id = session_id.map(Secret);
        if shared.desired == Some((guild_id, channel_id)) {
            shared.dirty = false;
        }
        drop(shared);
        self.spawn_if_ready();
    }

    pub fn voice_server(&self, guild_id: u64, token: Secret, endpoint: Option<String>) {
        let mut shared = self.lock();
        let Some(server) = shared.servers.get_mut(&guild_id) else {
            shared.servers.insert(guild_id, Server { token, endpoint });
            drop(shared);
            self.spawn_if_ready();
            return;
        };
        let token_changed = server.token.expose() != token.expose();
        server.token = token;
        if endpoint.is_none() {
            server.endpoint = None;
            shared.restart = None;
            if shared
                .task
                .as_ref()
                .is_some_and(|task| server_id(task.guild_id, task.channel_id) == guild_id)
                && let Some(task) = shared.task.take()
            {
                let _ = task.shutdown.send(true);
            }
            return;
        }
        let endpoint_changed = server.endpoint != endpoint;
        server.endpoint = endpoint;
        if let Some(reason) = server_restart_reason(token_changed, endpoint_changed) {
            shared.restart = Some(reason);
            if shared
                .task
                .as_ref()
                .is_some_and(|task| server_id(task.guild_id, task.channel_id) == guild_id)
                && let Some(task) = shared.task.take()
            {
                let _ = task.shutdown.send(true);
            }
        }
        drop(shared);
        self.spawn_if_ready();
    }

    pub fn dm_voice_server(
        &self,
        channel_id: Option<u64>,
        token: Secret,
        endpoint: Option<String>,
    ) {
        let shared = self.lock();
        let Some((0, desired_channel)) = shared.desired else {
            return;
        };
        if channel_id.is_some_and(|id| id != desired_channel) {
            return;
        }
        drop(shared);
        self.voice_server(desired_channel, token, endpoint);
    }

    pub fn gateway_ready(&self, user_id: u64, session_id: &str, fresh: bool) {
        let mut shared = self.lock();
        shared.user_id = Some(user_id);
        shared.gateway_session_id = Some(Secret(session_id.to_owned()));
        if fresh {
            self.end_stream(&mut shared, Some("Discord reset the voice session".into()));
            self.end_share(&mut shared, Some("Discord reset the voice session".into()));
        }
        if fresh && (shared.desired.is_some() || shared.confirmed.is_some()) {
            shared.desired = None;
            shared.confirmed = None;
            shared.session_id = None;
            shared.dirty = false;
            shared.restart = None;
            self.end_stream(&mut shared, Some("Discord reset the voice session".into()));
            if let Some(task) = shared.task.take() {
                let _ = task.shutdown.send(true);
            }
            drop(shared);
            self.emit(VoiceEvent::Disconnected {
                reason: Some("Discord reset the voice session".into()),
            });
        }
    }

    pub fn take_pending_op4(&self) -> Option<Value> {
        let shared = self.lock();
        if !shared.dirty {
            return None;
        }
        if shared.desired.is_some() || shared.last_guild.is_some() {
            Some(op4(&shared))
        } else {
            None
        }
    }

    fn give_up(&self, leave: bool) {
        let mut shared = self.lock();
        shared.desired = None;
        shared.confirmed = None;
        shared.session_id = None;
        shared.dirty = false;
        shared.restart = None;
        self.end_stream(&mut shared, Some("Voice disconnected".into()));
        self.end_share(&mut shared, Some("Voice disconnected".into()));
        let payload = (leave && shared.last_guild.is_some()).then(|| {
            shared.dirty = true;
            op4(&shared)
        });
        let sender = self.commands.clone();
        let generation = self
            .gateway_generation
            .as_ref()
            .map(|generation| generation.load(Ordering::Acquire));
        drop(shared);
        if let (Some(sender), Some(generation), Some(payload)) = (sender, generation, payload)
            && generation % 2 == 1
        {
            let _ = sender.send((generation, commands::voice_state(payload)));
        }
    }

    pub fn shutdown(&self) {
        let mut shared = self.lock();
        shared.desired = None;
        shared.confirmed = None;
        shared.session_id = None;
        shared.restart = None;
        self.end_stream(&mut shared, None);
        self.end_share(&mut shared, None);
        if let Some(task) = shared.task.take() {
            let _ = task.shutdown.send(true);
        }
    }

    fn spawn_if_ready(&self) {
        let (Some(runtime), Some(events)) = (&self.runtime, &self.events) else {
            return;
        };
        let mut shared = self.lock();
        let Some((guild_id, channel_id)) = shared.desired else {
            return;
        };
        if shared.confirmed != Some((guild_id, channel_id)) {
            return;
        }
        let Some(session_id) = shared.session_id.clone() else {
            return;
        };
        let Some(user_id) = shared.user_id else {
            return;
        };
        let Some((endpoint, token)) = shared
            .servers
            .get(&server_id(guild_id, channel_id))
            .and_then(|server| {
                server
                    .endpoint
                    .clone()
                    .map(|endpoint| (endpoint, server.token.clone()))
            })
        else {
            return;
        };
        let Some(ws_url) = ws_url(&endpoint) else {
            return;
        };
        let Some(server) = server_name(&endpoint) else {
            return;
        };
        if shared.task.as_ref().is_some_and(|task| {
            task.guild_id == guild_id && task.channel_id == channel_id && task.endpoint == endpoint
        }) {
            return;
        }
        let first = shared.generation == 0;
        let restart = match shared.restart.take() {
            Some(reason) => reason,
            None => match shared.task.as_ref() {
                Some(task) if task.guild_id != guild_id || task.channel_id != channel_id => {
                    "channel changed"
                }
                Some(task) if task.endpoint != endpoint => "server endpoint changed",
                Some(_) => "voice state changed",
                None if first => "join requested",
                None => "rejoin requested",
            },
        };
        if let Some(task) = shared.task.take() {
            let _ = task.shutdown.send(true);
        }
        shared.generation = shared.generation.wrapping_add(1);
        let generation = shared.generation;
        let (shutdown, shutdown_rx) = watch::channel(false);
        shared.task = Some(Task {
            shutdown,
            guild_id,
            channel_id,
            endpoint: endpoint.clone(),
        });
        let flags = shared.flags.clone();
        let config = Config {
            guild_id,
            channel_id,
            user_id,
            session_id,
            token,
            server,
            ws_url,
        };
        drop(shared);
        let action = if first {
            "Voice starting"
        } else {
            "Voice restarting"
        };
        log::info!("{action}: {restart}");
        runtime.spawn(connection_loop(
            config,
            events.clone(),
            shutdown_rx,
            flags,
            self.shared.clone(),
            generation,
            self.clone(),
        ));
    }

    fn emit(&self, event: VoiceEvent) {
        if let Some(events) = &self.events {
            emit(events, event);
        }
    }

    fn emit_connecting(&self, guild_id: u64, channel_id: u64) {
        self.emit(VoiceEvent::Connecting {
            guild_id,
            channel_id,
        });
    }
}

fn op4(shared: &Shared) -> Value {
    let (guild_id, channel_id) = match shared.desired {
        Some((0, channel)) => (Value::Null, json!(channel.to_string())),
        Some((guild, channel)) => (json!(guild.to_string()), json!(channel.to_string())),
        None => (
            shared
                .last_guild
                .filter(|guild| *guild != 0)
                .map(|guild| json!(guild.to_string()))
                .unwrap_or(Value::Null),
            Value::Null,
        ),
    };
    json!({"op": 4, "d": {
        "guild_id": guild_id,
        "channel_id": channel_id,
        "self_mute": shared.mute || shared.deaf,
        "self_deaf": shared.deaf,
        "self_video": false,
        "flags": OP4_FLAGS,
    }})
}

fn server_restart_reason(token_changed: bool, endpoint_changed: bool) -> Option<&'static str> {
    match (token_changed, endpoint_changed) {
        (true, true) => Some("server endpoint and token changed"),
        (true, false) => Some("token changed"),
        (false, true) => Some("server endpoint changed"),
        (false, false) => None,
    }
}

pub(super) fn ws_url(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        return None;
    }
    Some(
        if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
            format!("{endpoint}/?v=9")
        } else {
            format!("wss://{endpoint}/?v=9")
        },
    )
}

pub(super) fn server_name(endpoint: &str) -> Option<String> {
    let endpoint = endpoint.trim();
    let authority = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, authority)| authority);
    let authority = authority.split(['/', '?', '#']).next().unwrap_or_default();
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split_once(']')?.0
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => host,
            _ => authority,
        }
    };
    (!host.is_empty()).then(|| host.to_owned())
}

async fn connection_loop(
    config: Config,
    events: UnboundedSender<Event>,
    mut shutdown: watch::Receiver<bool>,
    flags: AudioFlags,
    shared: Arc<Mutex<Shared>>,
    generation: u64,
    manager: VoiceManager,
) {
    let mut connection = Connection::new(config, flags);
    let mut attempts = 0;
    let mut resume = false;
    loop {
        if *shutdown.borrow() {
            break;
        }
        match connection.attempt(&events, &mut shutdown, resume).await {
            Outcome::Stopped => break,
            Outcome::Fatal { reason, leave } => {
                emit(
                    &events,
                    VoiceEvent::Disconnected {
                        reason: Some(reason),
                    },
                );
                manager.give_up(leave);
                break;
            }
            Outcome::Reconnect => {
                attempts += 1;
                if attempts > CONNECT_ATTEMPTS {
                    emit(
                        &events,
                        VoiceEvent::Disconnected {
                            reason: Some("Could not connect to voice".into()),
                        },
                    );
                    manager.give_up(true);
                    break;
                }
                let reason = match connection.last_close() {
                    Some(code) => format!("reconnect after close {code}"),
                    None => "reconnect after connection loss".into(),
                };
                log::info!("Voice restarting: {reason}");
                emit(
                    &events,
                    VoiceEvent::Connecting {
                        guild_id: connection.guild_id(),
                        channel_id: connection.channel_id(),
                    },
                );
                resume = connection.can_resume();
                let base = 1000u64 << (attempts - 1).min(4);
                let jitter = rand::thread_rng().gen_range(0..=500);
                tokio::select! {
                    _ = shutdown.changed() => break,
                    _ = sleep(Duration::from_millis(base + jitter)) => {}
                }
            }
        }
    }
    let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
    if shared.generation == generation {
        shared.task = None;
    }
}

#[cfg(test)]
mod tests;
