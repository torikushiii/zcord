mod commands;
mod gateway;
pub mod model;
mod permissions;
mod properties;
mod rest;
#[cfg(test)]
pub mod settings_fixtures;
pub mod settings_proto;

use futures::channel::mpsc::{self, UnboundedReceiver};
use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub use permissions::{
    MANAGE_MESSAGES, SEND_MESSAGES, VIEW_CHANNEL, can_view, channel_permissions,
};
pub use properties::USER_AGENT;

struct Secret(String);
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
    cache: Mutex<Cache>,
    control: watch::Sender<Control>,
    commands: tokio::sync::mpsc::UnboundedSender<(u64, commands::Command)>,
    generation: Arc<AtomicU64>,
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
        let (commands, command_receiver) = tokio::sync::mpsc::unbounded_channel();
        let generation = Arc::new(AtomicU64::new(0));
        let session = Self {
            inner: Arc::new(Inner {
                rest,
                cache: Mutex::default(),
                control,
                commands,
                generation: generation.clone(),
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
        ));
        (session, receiver)
    }

    pub async fn fetch_messages(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<model::Message>, Error> {
        self.rest(async |rest| {
            rest.fetch_messages(guild_id, channel_id, before, limit)
                .await
        })
        .await
    }

    pub async fn ack_bulk(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        acks: Vec<(u64, u64)>,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.ack_bulk(guild_id, channel_id, &acks).await)
            .await
    }

    pub async fn ack_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        last_viewed: u32,
    ) -> Result<(), Error> {
        self.rest(async |rest| {
            rest.ack_message(guild_id, channel_id, message_id, last_viewed)
                .await
        })
        .await
    }

    pub async fn send_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message: OutgoingMessage,
    ) -> Result<model::Message, Error> {
        self.rest(async |rest| rest.send_message(guild_id, channel_id, &message).await)
            .await
    }

    pub async fn edit_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        content: String,
    ) -> Result<model::MessageUpdate, Error> {
        self.rest(async |rest| {
            rest.edit_message(guild_id, channel_id, message_id, &content)
                .await
        })
        .await
    }

    pub async fn delete_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.delete_message(guild_id, channel_id, message_id).await)
            .await
    }

    pub async fn trigger_typing(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
    ) -> Result<(), Error> {
        self.rest(async |rest| rest.trigger_typing(guild_id, channel_id).await)
            .await
    }

    pub async fn profile(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
        kind: ProfileKind,
    ) -> Result<Arc<model::Profile>, Error> {
        let key = (user_id, guild_id);
        if let Some(profile) = self.cache().profile(key, kind, Instant::now()) {
            return Ok(profile);
        }
        let profile = self
            .rest(async |rest| {
                rest.fetch_profile(user_id, guild_id, channel_id, kind)
                    .await
            })
            .await?;
        let profile = Arc::new(profile);
        self.cache()
            .insert_profile(key, kind, profile.clone(), Instant::now());
        Ok(profile)
    }

    pub async fn collectible_product(
        &self,
        sku_id: u64,
    ) -> Result<Arc<model::CollectibleProduct>, Error> {
        if let Some(product) = self.cache().products.get(&sku_id) {
            return Ok(product.clone());
        }
        let product = self
            .rest(async |rest| rest.fetch_collectible_product(sku_id).await)
            .await?;
        let product = Arc::new(product);
        self.cache().insert_product(sku_id, product.clone());
        Ok(product)
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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileKind {
    Popout,
    Modal,
}

const PROFILE_TTL: Duration = Duration::from_secs(5 * 60);
const CACHE_LIMIT: usize = 256;

#[derive(Default)]
struct Cache {
    profiles: HashMap<(u64, Option<u64>), CachedProfile>,
    products: HashMap<u64, Arc<model::CollectibleProduct>>,
}

struct CachedProfile {
    kind: ProfileKind,
    fetched: Instant,
    profile: Arc<model::Profile>,
}

impl Cache {
    fn profile(
        &self,
        key: (u64, Option<u64>),
        kind: ProfileKind,
        now: Instant,
    ) -> Option<Arc<model::Profile>> {
        let cached = self.profiles.get(&key)?;
        // A full profile response includes everything the popout shows, so it can stand in.
        let covers = cached.kind == kind || cached.kind == ProfileKind::Modal;
        (covers && now.saturating_duration_since(cached.fetched) < PROFILE_TTL)
            .then(|| cached.profile.clone())
    }

    fn insert_profile(
        &mut self,
        key: (u64, Option<u64>),
        kind: ProfileKind,
        profile: Arc<model::Profile>,
        now: Instant,
    ) {
        if self.profiles.len() >= CACHE_LIMIT && !self.profiles.contains_key(&key) {
            let oldest = self
                .profiles
                .iter()
                .min_by_key(|(_, cached)| cached.fetched)
                .map(|(key, _)| *key);
            if let Some(oldest) = oldest {
                self.profiles.remove(&oldest);
            }
        }
        self.profiles.insert(
            key,
            CachedProfile {
                kind,
                fetched: now,
                profile,
            },
        );
    }

    fn insert_product(&mut self, sku_id: u64, product: Arc<model::CollectibleProduct>) {
        if self.products.len() >= CACHE_LIMIT {
            let any = self.products.keys().next().copied();
            if let Some(any) = any {
                self.products.remove(&any);
            }
        }
        self.products.insert(sku_id, product);
    }
}

pub enum Event {
    ReadySupplemental(Box<model::ReadySupplemental>),
    PresenceUpdate(Box<model::Presence>),
    MembersChunk(Box<model::MembersChunk>),
    MemberListUpdate(Box<model::MemberListUpdate>),
    MemberUpsert(Box<model::MemberUpdate>),
    MemberRemove(Box<model::MemberUpdate>),
    RoleUpsert(Box<model::RoleUpdate>),
    RoleDelete { guild_id: u64, role_id: u64 },
    Ready(Box<model::Ready>),
    SettingsUpdate(Box<model::SettingsUpdate>),
    MessageCreate(Box<model::Message>),
    MessageUpdate(Box<model::MessageUpdate>),
    TypingStart(Box<model::TypingStart>),
    MessageReactionAdd(model::ReactionEvent),
    MessageReactionRemove(model::ReactionEvent),
    MessageReactionRemoveAll(model::ReactionEvent),
    MessageReactionRemoveEmoji(model::ReactionEvent),
    MessageReactionAddMany(model::ReactionManyEvent),
    MessageDelete { channel_id: u64, id: u64 },
    ChannelUpsert(Box<model::Channel>),
    ChannelDelete { id: u64 },
    GuildUpsert(Box<model::Guild>),
    GuildDelete { id: u64 },
    MessageAck { channel_id: u64, message_id: u64 },
    Reconnecting,
    Resumed,
    Fatal(FatalError),
}

#[derive(Debug)]
pub enum FatalError {
    InvalidToken,
    Other(String),
}

#[derive(Clone, Debug)]
pub enum Error {
    RateLimited {
        retry_after: Duration,
    },
    Http(u16),
    Captcha,
    Rejected {
        status: u16,
        code: u64,
        message: String,
    },
    Network,
    Decode,
    InvalidCredential,
    Closed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimited { retry_after } => write!(
                f,
                "Rate limited; retry after {:.1}s",
                retry_after.as_secs_f64()
            ),
            Self::Http(status) => write!(f, "HTTP {status}"),
            Self::Captcha => f.write_str("Discord requested verification"),
            Self::Rejected {
                status,
                code,
                message,
            } => write!(f, "HTTP {status} ({code}): {message}"),
            Self::Network => f.write_str("Network request failed"),
            Self::Decode => f.write_str("Malformed Discord response"),
            Self::InvalidCredential => f.write_str("Invalid credential header"),
            Self::Closed => f.write_str("Session closed"),
        }
    }
}
impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Arc<model::Profile> {
        Arc::new(serde_json::from_str(include_str!("discord/fixtures/profile_modal.json")).unwrap())
    }

    #[test]
    fn profile_cache_lets_full_profiles_serve_popouts_until_expiry() {
        let mut cache = Cache::default();
        let start = Instant::now();
        let key = (1, Some(2));
        cache.insert_profile(key, ProfileKind::Popout, profile(), start);
        assert!(cache.profile(key, ProfileKind::Popout, start).is_some());
        assert!(cache.profile(key, ProfileKind::Modal, start).is_none());
        assert!(
            cache
                .profile((1, None), ProfileKind::Popout, start)
                .is_none()
        );
        cache.insert_profile(key, ProfileKind::Modal, profile(), start);
        assert!(cache.profile(key, ProfileKind::Popout, start).is_some());
        assert!(cache.profile(key, ProfileKind::Modal, start).is_some());
        let expired = start + PROFILE_TTL;
        assert!(cache.profile(key, ProfileKind::Popout, expired).is_none());
    }

    #[test]
    fn profile_cache_evicts_the_oldest_entry_when_full() {
        let mut cache = Cache::default();
        let start = Instant::now();
        let profile = profile();
        for user in 0..CACHE_LIMIT as u64 {
            let fetched = start + Duration::from_secs(user + 1);
            cache.insert_profile((user, None), ProfileKind::Popout, profile.clone(), fetched);
        }
        cache.insert_profile((999, None), ProfileKind::Popout, profile, start);
        assert_eq!(cache.profiles.len(), CACHE_LIMIT);
        assert!(!cache.profiles.contains_key(&(0, None)));
        assert!(cache.profiles.contains_key(&(999, None)));
    }
}
