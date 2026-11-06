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

    pub fn watch(&self, user_id: u64) -> Option<Value> {
        let mut shared = self.lock();
        if user_id == 0 || shared.user_id == Some(user_id) {
            return None;
        }
        let (guild_id, channel_id) = shared.confirmed?;
        shared.session_id.as_ref()?;
        let key = stream::stream_key((guild_id != 0).then_some(guild_id), channel_id, user_id);
        self.end_stream(&mut shared, Some("Watching another stream".into()));
        shared.stream = Some(StreamWatch {
            user_id,
            key: key.clone(),
            server_id: server_id(guild_id, channel_id),
            rtc_server_id: None,
            rtc_channel_id: None,
            token: None,
            endpoint: None,
            shutdown: None,
            ended: Arc::new(AtomicBool::new(false)),
            started: false,
        });
        *shared.frames.lock().unwrap_or_else(PoisonError::into_inner) = None;
        Some(json!({"op": 20, "d": {"stream_key": key}}))
    }

    pub fn stop_watching(&self) -> Option<Value> {
        let mut shared = self.lock();
        let key = shared.stream.as_ref()?.key.clone();
        self.end_stream(&mut shared, None);
        Some(json!({"op": 19, "d": {"stream_key": key}}))
    }

    pub fn stream_created(&self, key: &str, rtc_server_id: u64, rtc_channel_id: Option<u64>) {
        let mut shared = self.lock();
        if shared.share.as_ref().is_some_and(|share| share.key == key) {
            let next = (rtc_server_id, rtc_channel_id.unwrap_or(rtc_server_id));
            if shared.share.as_ref().is_some_and(|share| {
                share
                    .rtc_server_id
                    .is_some_and(|server| (server, share.rtc_channel_id.unwrap_or(server)) != next)
            }) {
                self.end_share(
                    &mut shared,
                    Some("Discord changed the stream connection identity".into()),
                );
                return;
            }
            if let Some(share) = shared.share.as_mut() {
                share.rtc_server_id = Some(rtc_server_id);
                share.rtc_channel_id = rtc_channel_id;
            }
            self.spawn_share(&mut shared);
            return;
        }
        if shared
            .stream
            .as_ref()
            .is_none_or(|stream| stream.key != key)
        {
            return;
        }
        let next = (rtc_server_id, rtc_channel_id.unwrap_or(rtc_server_id));
        if shared.stream.as_ref().is_some_and(|stream| {
            stream
                .rtc_server_id
                .is_some_and(|server| (server, stream.rtc_channel_id.unwrap_or(server)) != next)
        }) {
            self.end_stream(
                &mut shared,
                Some("Discord changed the stream connection identity".into()),
            );
            return;
        }
        if let Some(stream) = shared.stream.as_mut() {
            stream.rtc_server_id = Some(rtc_server_id);
            stream.rtc_channel_id = rtc_channel_id;
        }
        self.spawn_stream(&mut shared);
    }

    pub fn stream_server(&self, key: &str, token: &str, endpoint: Option<&str>) {
        let mut shared = self.lock();
        if shared.share.as_ref().is_some_and(|share| share.key == key) {
            if token.len() > 2048 || endpoint.is_some_and(|endpoint| endpoint.len() > 512) {
                self.end_share(
                    &mut shared,
                    Some("Discord sent oversized stream server data".into()),
                );
                return;
            }
            if let Some(share) = shared.share.as_mut() {
                share.token = Some(Secret(token.to_owned()));
                share.endpoint = endpoint.map(str::to_owned);
            }
            self.spawn_share(&mut shared);
            return;
        }
        if shared
            .stream
            .as_ref()
            .is_none_or(|stream| stream.key != key)
        {
            return;
        }
        if token.len() > 2048 || endpoint.is_some_and(|endpoint| endpoint.len() > 512) {
            self.end_stream(
                &mut shared,
                Some("Discord sent oversized stream server data".into()),
            );
            return;
        }
        if let Some(stream) = shared.stream.as_mut() {
            stream.token = Some(Secret(token.to_owned()));
            stream.endpoint = endpoint.map(str::to_owned);
        }
        self.spawn_stream(&mut shared);
    }

    pub fn stream_deleted(&self, key: &str, reason: Option<String>) {
        let mut shared = self.lock();
        if shared.share.as_ref().is_some_and(|share| share.key == key) {
            self.end_share(&mut shared, reason);
            return;
        }
        if shared
            .stream
            .as_ref()
            .is_none_or(|stream| stream.key != key)
        {
            return;
        }
        self.end_stream(&mut shared, reason);
    }

    pub fn screen_sources(&self) -> Vec<ScreenSource> {
        stream::share::sources()
    }

    pub fn screen_thumbnail(&self, source: &ScreenSource) -> Option<stream::StreamFrame> {
        stream::share::thumbnail(source)
    }

    pub fn screen_icon(&self, source: &ScreenSource) -> Option<stream::StreamFrame> {
        stream::share::icon(source)
    }

    pub fn go_live(&self) -> Option<Value> {
        self.begin_share(None, StreamQuality::STANDARD, false)
    }

    pub fn start_share(
        &self,
        source: ScreenSource,
        quality: StreamQuality,
        audio: bool,
    ) -> Option<Value> {
        self.begin_share(Some(source), quality, audio)
    }

    pub fn stop_sharing(&self) -> Option<Value> {
        let mut shared = self.lock();
        let key = shared.share.as_ref()?.key.clone();
        self.end_share_with(&mut shared, None, false);
        Some(json!({"op": 19, "d": {"stream_key": key}}))
    }

    fn begin_share(
        &self,
        source: Option<ScreenSource>,
        quality: StreamQuality,
        audio: bool,
    ) -> Option<Value> {
        let mut shared = self.lock();
        let (guild_id, channel_id) = shared.confirmed?;
        let user_id = shared.user_id?;
        shared.session_id.as_ref()?;
        let key = stream::stream_key((guild_id != 0).then_some(guild_id), channel_id, user_id);
        self.end_share(&mut shared, Some("Sharing replaced".into()));
        let local = shared.local_preview.clone();
        shared.share = Some(StreamShare {
            key,
            server_id: server_id(guild_id, channel_id),
            live: source.map(|source| stream::share::LiveShare::new(source, quality, audio, local)),
            rtc_server_id: None,
            rtc_channel_id: None,
            token: None,
            endpoint: None,
            shutdown: None,
            ended: Arc::new(AtomicBool::new(false)),
            started: false,
            keyframe: Arc::new(AtomicBool::new(true)),
            bitrate: Arc::new(AtomicU32::new(quality.initial_bitrate())),
            stop: Arc::new(AtomicBool::new(false)),
            reannounce: Arc::new(AtomicBool::new(false)),
            failure: Arc::new(Mutex::new(None)),
        });
        Some(json!({"op": 18, "d": {
            "type": if guild_id == 0 { "call" } else { "guild" },
            "guild_id": if guild_id == 0 { Value::Null } else { json!(guild_id.to_string()) },
            "channel_id": channel_id.to_string(),
            "preferred_region": null,
        }}))
    }

    fn end_share(&self, shared: &mut Shared, reason: Option<String>) {
        self.end_share_with(shared, reason, true);
    }

    fn end_share_with(&self, shared: &mut Shared, reason: Option<String>, notify: bool) {
        let Some(mut share) = shared.share.take() else {
            return;
        };
        share.stop.store(true, Ordering::Release);
        if let Some(shutdown) = share.shutdown.take() {
            let _ = shutdown.send(true);
        }
        *shared
            .local_preview
            .frames
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        if notify
            && let (Some(sender), Some(generation)) = (
                self.commands.clone(),
                self.gateway_generation
                    .as_ref()
                    .map(|generation| generation.load(Ordering::Acquire)),
            )
            && generation % 2 == 1
        {
            let payload = json!({"op": 19, "d": {"stream_key": share.key}});
            let _ = sender.send((generation, commands::voice_state(payload)));
        }
        if !share.ended.swap(true, Ordering::AcqRel) {
            self.emit_share(ShareEvent::Ended { reason });
        }
    }

    fn spawn_share(&self, shared: &mut Shared) {
        let Some(share) = shared.share.as_ref() else {
            return;
        };
        if share.started {
            return;
        }
        let (Some(live), Some(rtc_server_id), Some(token)) =
            (share.live.clone(), share.rtc_server_id, share.token.clone())
        else {
            return;
        };
        let Some(endpoint) = share
            .endpoint
            .clone()
            .or_else(|| shared.servers.get(&share.server_id)?.endpoint.clone())
        else {
            return;
        };
        let (Some(user_id), Some(session_id)) = (shared.user_id, shared.session_id.clone()) else {
            return;
        };
        let (Some(runtime), Some(events)) = (&self.runtime, &self.events) else {
            return;
        };
        let key = share.key.clone();
        let ended = share.ended.clone();
        let keyframe = share.keyframe.clone();
        let bitrate = share.bitrate.clone();
        let stop = share.stop.clone();
        let reannounce = share.reannounce.clone();
        let failure = share.failure.clone();
        let (frame_sender, frame_receiver) = std::sync::mpsc::sync_channel(2);
        let previews: stream::share::PreviewSink = {
            let (events, stream_key) = (events.clone(), key.clone());
            Box::new(move |thumbnail| {
                let stream_key = stream_key.clone();
                emit(
                    &events,
                    VoiceEvent::Share(ShareEvent::Preview {
                        stream_key,
                        thumbnail,
                    }),
                );
            })
        };
        if let Err(reason) = stream::share::spawn(
            live.clone(),
            bitrate.clone(),
            keyframe.clone(),
            stop.clone(),
            frame_sender,
            failure.clone(),
            previews,
        ) {
            self.end_share(shared, Some(reason));
            return;
        }
        // The sound is heard even while muted, so it can be unmuted live. It's optional: if
        // Windows won't share it, the picture still goes out.
        let (audio_sender, audio_receiver) = std::sync::mpsc::sync_channel(8);
        let audio_frames = match stream::share::spawn_audio(audio_sender, stop.clone(), live) {
            Ok(()) => Some(audio_receiver),
            Err(reason) => {
                log::warn!("Streaming without sound: {reason}");
                None
            }
        };
        let (shutdown, cancel) = watch::channel(false);
        let Some(share) = shared.share.as_mut() else {
            return;
        };
        share.shutdown = Some(shutdown);
        share.started = true;
        self.emit_share(ShareEvent::Starting);
        let config = SenderConfig {
            user_id,
            session_id,
            token,
            endpoint,
            rtc_server_id,
            key,
            keyframe,
            bitrate,
            reannounce,
            failure,
            events: events.clone(),
            ended: ended.clone(),
        };
        let manager = self.clone();
        runtime.spawn(async move {
            let result = stream::sender::run(config, frame_receiver, audio_frames, cancel).await;
            stop.store(true, Ordering::Release);
            let mut shared = manager.lock();
            manager.end_share_with(&mut shared, result.err(), true);
        });
    }

    fn emit_share(&self, event: ShareEvent) {
        if let Some(events) = &self.events {
            emit(events, VoiceEvent::Share(event));
        }
    }

    /// Switches the running stream to another source or quality without ending it.
    pub fn change_share(&self, source: Option<ScreenSource>, quality: Option<StreamQuality>) {
        if let Some(live) = self
            .lock()
            .share
            .as_ref()
            .and_then(|share| share.live.as_ref())
        {
            live.change(source, quality);
        }
    }

    pub fn set_share_audio(&self, audio: bool) {
        if let Some(live) = self
            .lock()
            .share
            .as_ref()
            .and_then(|share| share.live.as_ref())
        {
            live.audio.store(audio, Ordering::Release);
        }
    }

    pub fn stream_updated(&self, key: &str, viewers: Vec<u64>) {
        let shared = self.lock();
        let Some(share) = shared.share.as_ref().filter(|share| share.key == key) else {
            return;
        };
        if !viewers.is_empty() {
            share.reannounce.store(true, Ordering::Release);
        }
        self.emit_share(ShareEvent::Viewers(viewers));
    }

    pub fn stream_frames(&self) -> FrameSlot {
        self.lock().frames.clone()
    }

    pub fn share_frames(&self) -> FrameSlot {
        self.lock().local_preview.frames.clone()
    }

    pub fn set_share_preview(&self, bounds: Option<(u32, u32)>) {
        let shared = self.lock();
        *shared
            .local_preview
            .bounds
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = bounds;
        if bounds.is_none() {
            *shared
                .local_preview
                .frames
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = None;
        }
    }

    fn end_stream(&self, shared: &mut Shared, reason: Option<String>) {
        shared.flags.clear_stream_audio();
        let Some(mut stream) = shared.stream.take() else {
            return;
        };
        if let Some(shutdown) = stream.shutdown.take() {
            let _ = shutdown.send(true);
        }
        *shared.frames.lock().unwrap_or_else(PoisonError::into_inner) = None;
        if !stream.ended.swap(true, Ordering::AcqRel) {
            self.emit_stream(StreamEvent::Ended {
                user_id: stream.user_id,
                reason,
            });
        }
    }

    fn spawn_stream(&self, shared: &mut Shared) {
        let Some(stream) = shared.stream.as_ref() else {
            return;
        };
        if stream.started {
            return;
        }
        let (Some(rtc_server_id), Some(token)) = (stream.rtc_server_id, stream.token.clone())
        else {
            return;
        };
        let Some(endpoint) = stream
            .endpoint
            .clone()
            .or_else(|| shared.servers.get(&stream.server_id)?.endpoint.clone())
        else {
            return;
        };
        let (Some(user_id), Some(session_id)) = (shared.user_id, shared.session_id.clone()) else {
            return;
        };
        let (Some(runtime), Some(events)) = (&self.runtime, &self.events) else {
            return;
        };
        let frames = shared.frames.clone();
        let flags = shared.flags.clone();
        let Some(stream) = shared.stream.as_mut() else {
            return;
        };
        let streamer_id = stream.user_id;
        let ended = stream.ended.clone();
        let watch_ended = ended.clone();
        let (shutdown, cancel) = watch::channel(false);
        stream.shutdown = Some(shutdown);
        stream.started = true;
        self.emit_stream(StreamEvent::Connecting {
            user_id: streamer_id,
        });
        let audio_generation = flags.begin_stream_audio();
        let config = ViewerConfig {
            user_id,
            streamer_id,
            session_id,
            token,
            endpoint,
            rtc_server_id,
            frames,
            events: events.clone(),
            ended,
            flags,
            audio_generation,
        };
        let manager = self.clone();
        runtime.spawn(async move {
            let result = stream::viewer::run(config, cancel).await;
            manager.stream_viewer_ended(&watch_ended, result);
        });
    }

    fn stream_viewer_ended(&self, ended: &Arc<AtomicBool>, result: Result<(), String>) {
        let mut shared = self.lock();
        if shared
            .stream
            .as_ref()
            .is_none_or(|stream| !Arc::ptr_eq(&stream.ended, ended))
        {
            return;
        }
        self.end_stream(&mut shared, result.err());
    }

    fn emit_stream(&self, event: StreamEvent) {
        if let Some(events) = &self.events {
            emit(events, VoiceEvent::Stream(event));
        }
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
        if shared.desired.is_none() {
            return None;
        }
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
mod tests {
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
}
