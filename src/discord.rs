mod cache;
mod commands;
mod error;
mod event;
mod gateway;
pub mod gifs;
pub mod model;
mod permissions;
mod properties;
pub mod remote_auth;
mod rest;
mod session;
mod voice;
#[cfg(test)]
pub mod settings_fixtures;
pub mod settings_proto;

use futures::channel::mpsc::{self, UnboundedReceiver};
use gifs::{Gif, GifCategories, GifProvider};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    future::Future,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex as AsyncMutex, watch};

pub use permissions::{
    CONNECT, MANAGE_MESSAGES, SEND_MESSAGES, USE_EXTERNAL_EMOJIS, VIEW_CHANNEL, can_view,
    channel_permissions,
};
pub use properties::USER_AGENT;

pub(crate) use cache::ProfileDetails;
use cache::{Cache, CacheRequest, insert_bounded, insert_missing, missing_ids};
pub use error::*;
pub use event::*;

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
}

pub struct OutgoingReply {
    pub message_id: u64,
    pub mention: bool,
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

    pub fn join_voice(&self, guild_id: u64, channel_id: u64) {
        let payload = self.inner.voice.join(guild_id, channel_id);
        self.voice_command(payload);
    }

    pub fn leave_voice(&self) {
        if let Some(payload) = self.inner.voice.leave() {
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
