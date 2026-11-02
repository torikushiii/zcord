mod cache;
mod commands;
mod error;
mod event;
mod gateway;
pub mod gifs;
pub mod model;
mod permissions;
mod presence;
mod properties;
pub mod remote_auth;
mod rest;
mod search;
mod session;
#[cfg(test)]
pub mod settings_fixtures;
pub mod settings_proto;
mod voice;

use futures::channel::mpsc::{self, UnboundedReceiver};
use gifs::{Gif, GifCategories, GifProvider};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    future::Future,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex as AsyncMutex, watch};

pub use permissions::{
    CONNECT, CREATE_PRIVATE_THREADS, CREATE_PUBLIC_THREADS, MANAGE_MESSAGES, MANAGE_THREADS,
    READ_MESSAGE_HISTORY, SEND_MESSAGES, SEND_MESSAGES_IN_THREADS, USE_EXTERNAL_EMOJIS,
    VIEW_CHANNEL, can_view, channel_permissions,
};
pub use presence::{SelfStatus, StatusDuration};
pub use properties::USER_AGENT;
pub use voice::{
    AudioDevice, FrameSlot, MicTest, ScreenSource, ScreenSourceId, StreamFrame, audio_devices,
};

pub(crate) use cache::ProfileDetails;
use cache::{Cache, CacheRequest, insert_bounded, insert_missing, missing_ids};
pub use error::*;
pub use event::*;
pub use model::{PinPage, ThreadPage};
pub use search::{ForumQuery, ForumSort, SearchOutcome, SearchQuery, SearchScope, SearchSort};

pub fn valid_friend_username(username: &str) -> bool {
    (2..=32).contains(&username.len())
        // Discord usernames cannot contain two consecutive periods.
        && !username.contains("..")
        && username
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.'))
}

#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn into_inner(self) -> String {
        self.0
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Running,
    Shutdown,
    InvalidToken,
}

struct Inner {
    rest: Result<rest::Rest, Error>,
    // Tenor/Giphy only: never carries the Discord session headers.
    gif_client: Result<reqwest::Client, Error>,
    cache: Mutex<Cache>,
    requests: Mutex<HashMap<CacheRequest, Weak<AsyncMutex<()>>>>,
    sends: Mutex<HashMap<u64, tokio::sync::oneshot::Receiver<()>>>,
    control: watch::Sender<Control>,
    commands: tokio::sync::mpsc::UnboundedSender<(u64, commands::Command)>,
    generation: Arc<AtomicU64>,
    presence: Arc<Mutex<presence::Presence>>,
    voice: voice::VoiceManager,
}

#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

pub struct OutgoingMessage {
    pub content: String,
    pub nonce: u64,
    pub reply: Option<OutgoingReply>,
    pub attachments: Vec<OutgoingAttachment>,
}

pub struct OutgoingReply {
    pub message_id: u64,
    pub mention: bool,
}

pub struct OutgoingAttachment {
    pub filename: String,
    pub uploaded_filename: String,
    pub content_type: String,
    pub spoiler: bool,
}

#[derive(Clone)]
pub struct UploadFile {
    pub path: PathBuf,
    pub filename: String,
    pub size: u64,
    pub content_type: String,
    pub spoiler: bool,
}

pub struct UploadProgress {
    pub sent: AtomicU64,
    pub total: u64,
    pub cancelled: AtomicBool,
}

impl UploadProgress {
    pub fn new(total: u64) -> Arc<Self> {
        Arc::new(Self {
            sent: AtomicU64::new(0),
            total,
            cancelled: AtomicBool::new(false),
        })
    }
}

#[derive(Clone)]
pub struct UploadTarget {
    pub id: u64,
    pub upload_url: String,
    pub upload_filename: String,
}

impl Session {
    pub fn connect(
        token: String,
        runtime: &tokio::runtime::Handle,
    ) -> (Session, UnboundedReceiver<Event>) {
        let token = Arc::new(Secret(token));
        let (sender, receiver) = mpsc::unbounded();
        let (control, shutdown) = watch::channel(Control::Running);
        let rest = rest::Rest::new(&token.0);
        let gif_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| Error::Network);
        let (commands, command_receiver) = tokio::sync::mpsc::unbounded_channel();
        let generation = Arc::new(AtomicU64::new(0));
        let presence = Arc::new(Mutex::new(presence::Presence::default()));
        let voice = voice::VoiceManager::new(
            runtime,
            sender.clone(),
            commands.clone(),
            generation.clone(),
        );
        let session = Self {
            inner: Arc::new(Inner {
                rest,
                gif_client,
                cache: Mutex::default(),
                requests: Mutex::default(),
                sends: Mutex::default(),
                control,
                commands,
                generation: generation.clone(),
                presence: presence.clone(),
                voice: voice.clone(),
            }),
        };
        let owner = Arc::downgrade(&session.inner);
        runtime.spawn(gateway::run(
            token,
            sender,
            shutdown,
            owner,
            command_receiver,
            generation,
            voice,
            presence,
        ));
        (session, receiver)
    }

    fn gif_client(&self) -> Result<&reqwest::Client, Error> {
        self.inner.gif_client.as_ref().map_err(Clone::clone)
    }

    fn request_lock(&self, key: CacheRequest) -> Arc<AsyncMutex<()>> {
        let mut requests = self
            .inner
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        requests.retain(|_, request| request.strong_count() > 0);
        if let Some(request) = requests.get(&key).and_then(Weak::upgrade) {
            return request;
        }
        let request = Arc::new(AsyncMutex::new(()));
        requests.insert(key, Arc::downgrade(&request));
        request
    }

    fn cache(&self) -> MutexGuard<'_, Cache> {
        self.inner
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    async fn rest<T>(
        &self,
        call: impl AsyncFnOnce(&rest::Rest) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut shutdown = self.inner.control.subscribe();
        if *shutdown.borrow() != Control::Running {
            return Err(Error::Closed);
        }
        let rest = self.inner.rest.as_ref().map_err(Clone::clone)?;
        let result = tokio::select! {
            biased;
            _ = shutdown.changed() => Err(Error::Closed),
            result = call(rest) => result,
        };
        if matches!(result, Err(Error::Http(401))) {
            self.inner.control.send_replace(Control::InvalidToken);
        }
        result
    }

    pub fn shutdown(&self) {
        self.inner.control.send_replace(Control::Shutdown);
    }

    fn command(&self, command: commands::Command) {
        let generation = self.inner.generation.load(Ordering::Acquire);
        if generation % 2 == 0 || *self.inner.control.borrow() != Control::Running {
            log::debug!("Dropping gateway command while disconnected/connecting");
            return;
        }
        let _ = self.inner.commands.send((generation, command));
    }

    pub fn subscribe_channel(&self, guild_id: u64, channel_id: u64, ranges: &[(u32, u32)]) {
        self.command(commands::subscription(guild_id, channel_id, ranges));
    }

    pub fn request_members(&self, guild_id: u64, user_ids: Vec<u64>) {
        for command in commands::members(guild_id, &user_ids) {
            self.command(command);
        }
    }

    pub fn query_members(&self, guild_id: u64, query: &str, limit: u32) {
        self.command(commands::query_members(guild_id, query, limit));
    }

    pub fn request_voice_start_times(&self, guild_id: u64) {
        self.command(commands::voice_start_times(guild_id));
    }

    pub fn join_voice(&self, guild_id: u64, channel_id: u64) {
        let payload = self.inner.voice.join(guild_id, channel_id);
        self.voice_command(payload);
    }

    pub fn leave_voice(&self) {
        if let Some(payload) = self.inner.voice.leave() {
            self.voice_command(payload);
        }
    }

    pub fn watch_stream(&self, user_id: u64) {
        if let Some(payload) = self.inner.voice.watch(user_id) {
            self.voice_command(payload);
        }
    }

    pub fn stop_watching(&self) {
        if let Some(payload) = self.inner.voice.stop_watching() {
            self.voice_command(payload);
        }
    }

    pub fn stream_frames(&self) -> FrameSlot {
        self.inner.voice.stream_frames()
    }

    pub fn screen_sources(&self) -> Vec<ScreenSource> {
        self.inner.voice.screen_sources()
    }

    pub fn screen_thumbnail(&self, source: &ScreenSource) -> Option<StreamFrame> {
        self.inner.voice.screen_thumbnail(source)
    }

    pub async fn stream_preview(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        user_id: u64,
    ) -> Result<String, Error> {
        let key = voice::stream_key(guild_id, channel_id, user_id);
        self.rest(async |rest| rest.stream_preview(&key).await)
            .await
    }

    pub fn start_share(&self, source: ScreenSource) {
        if let Some(payload) = self.inner.voice.start_share(source) {
            self.voice_command(payload);
        }
    }

    pub fn stop_share(&self) {
        if let Some(payload) = self.inner.voice.stop_sharing() {
            self.voice_command(payload);
        }
    }

    pub fn set_self_mute(&self, mute: bool) {
        if let Some(payload) = self.inner.voice.set_mute(mute) {
            self.voice_command(payload);
        }
    }

    pub fn set_self_deaf(&self, deaf: bool) {
        if let Some(payload) = self.inner.voice.set_deaf(deaf) {
            self.voice_command(payload);
        }
    }

    pub fn set_noise_suppression(&self, enabled: bool) {
        self.inner.voice.set_noise_suppression(enabled);
    }

    pub fn set_input_device(&self, name: Option<String>) {
        self.inner.voice.set_input_device(name);
    }

    pub fn set_output_device(&self, name: Option<String>) {
        self.inner.voice.set_output_device(name);
    }

    pub fn set_input_volume(&self, gain: f32) {
        self.inner.voice.set_input_volume(gain);
    }

    pub fn set_output_volume(&self, gain: f32) {
        self.inner.voice.set_output_volume(gain);
    }

    pub fn set_stream_volume(&self, gain: f32) {
        self.inner.voice.set_stream_volume(gain);
    }

    pub fn set_input_sensitivity(&self, threshold_db: Option<f32>) {
        self.inner.voice.set_input_sensitivity(threshold_db);
    }

    pub fn input_level(&self) -> Option<f32> {
        self.inner.voice.input_level()
    }

    pub fn input_gate_open(&self) -> bool {
        self.inner.voice.input_gate_open()
    }

    fn voice_command(&self, payload: serde_json::Value) {
        let generation = self.inner.generation.load(Ordering::Acquire);
        if generation % 2 == 0 || *self.inner.control.borrow() != Control::Running {
            log::debug!("Deferring voice state update until the gateway is ready");
            return;
        }
        let _ = self
            .inner
            .commands
            .send((generation, commands::voice_state(payload)));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileKind {
    Popout,
    Modal,
}

const PROFILE_TTL: Duration = Duration::from_secs(5 * 60);
const FAILURE_TTL: Duration = Duration::from_secs(60);
const CACHE_LIMIT: usize = 256;

#[cfg(test)]
mod tests {
    use super::valid_friend_username;

    #[test]
    fn friend_usernames_match_serein_validation() {
        assert!(!valid_friend_username("a"));
        assert!(valid_friend_username("ab"));
        assert!(valid_friend_username(&"a".repeat(32)));
        assert!(!valid_friend_username(&"a".repeat(33)));
        assert!(valid_friend_username("a.b_c1"));
        assert!(!valid_friend_username("a b"));
        assert!(!valid_friend_username("a-b"));
        assert!(!valid_friend_username(""));
        assert!(!valid_friend_username("a..b"));
        assert!(!valid_friend_username("AB"));
    }
}
