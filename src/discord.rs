mod commands;
mod gateway;
pub mod gifs;
pub mod model;
mod permissions;
mod properties;
mod rest;
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
    MANAGE_MESSAGES, SEND_MESSAGES, USE_EXTERNAL_EMOJIS, VIEW_CHANNEL, can_view,
    channel_permissions,
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
    // Tenor/Giphy only: never carries the Discord session headers.
    gif_client: Result<reqwest::Client, Error>,
    cache: Mutex<Cache>,
    requests: Mutex<HashMap<CacheRequest, Weak<AsyncMutex<()>>>>,
    sends: Mutex<HashMap<u64, tokio::sync::oneshot::Receiver<()>>>,
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
        let gif_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| Error::Network);
        let (commands, command_receiver) = tokio::sync::mpsc::unbounded_channel();
        let generation = Arc::new(AtomicU64::new(0));
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

    pub async fn gif_search(&self, provider: GifProvider, query: &str) -> Result<Vec<Gif>, Error> {
        match provider {
            GifProvider::Klipy => {
                self.rest(async |rest| rest.fetch_gif_search(query).await)
                    .await
            }
            GifProvider::Tenor => gifs::tenor_search(self.gif_client()?, query).await,
            GifProvider::Giphy => gifs::giphy_search(self.gif_client()?, query).await,
        }
    }

    pub async fn gif_trending(&self, provider: GifProvider) -> Result<Vec<Gif>, Error> {
        match provider {
            GifProvider::Klipy => {
                self.rest(async |rest| rest.fetch_gif_trending().await)
                    .await
            }
            GifProvider::Tenor => gifs::tenor_trending(self.gif_client()?).await,
            GifProvider::Giphy => gifs::giphy_trending(self.gif_client()?).await,
        }
    }

    pub async fn gif_categories(&self, provider: GifProvider) -> Result<GifCategories, Error> {
        match provider {
            GifProvider::Klipy => {
                self.rest(async |rest| rest.fetch_gif_categories().await)
                    .await
            }
            GifProvider::Tenor => gifs::tenor_categories_request(self.gif_client()?).await,
            GifProvider::Giphy => gifs::giphy_categories_request(self.gif_client()?).await,
        }
    }

    pub async fn frecency_settings(&self) -> Result<Vec<u8>, Error> {
        self.rest(async |rest| rest.fetch_frecency_settings().await)
            .await
    }

    pub async fn update_frecency_settings(&self, proto: &[u8]) -> Result<Vec<u8>, Error> {
        self.rest(async |rest| rest.update_frecency_settings(proto).await)
            .await
    }

    fn gif_client(&self) -> Result<&reqwest::Client, Error> {
        self.inner.gif_client.as_ref().map_err(Clone::clone)
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

    pub fn send_message(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        message: OutgoingMessage,
    ) -> impl Future<Output = Result<model::Message, Error>> + Send + 'static {
        // The ticket is taken here, before spawning, so sends keep composer order.
        let (done, turn) = tokio::sync::oneshot::channel::<()>();
        let previous = self
            .inner
            .sends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(channel_id, turn);
        let session = self.clone();
        async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            let result = session
                .rest(async |rest| rest.send_message(guild_id, channel_id, &message).await)
                .await;
            drop(done);
            result
        }
    }

    pub async fn open_dm(&self, user_id: u64) -> Result<model::Channel, Error> {
        self.rest(async |rest| rest.open_dm(user_id).await).await
    }

    pub async fn accept_message_request(&self, channel_id: u64) -> Result<model::Channel, Error> {
        self.rest(async |rest| rest.accept_message_request(channel_id).await)
            .await
    }

    pub async fn reject_message_request(&self, channel_id: u64) -> Result<(), Error> {
        self.rest(async |rest| rest.reject_message_request(channel_id).await)
            .await
    }

    pub async fn message_request_previews(
        &self,
        channel_ids: Vec<u64>,
    ) -> Result<Vec<model::MessageRequestPreview>, Error> {
        self.rest(async |rest| rest.message_request_previews(&channel_ids).await)
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
    ) -> Result<bool, Error> {
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
        let _request = self
            .request_lock(CacheRequest::Profile(user_id, guild_id))
            .lock_owned()
            .await;
        let key = (user_id, guild_id);
        if let Some(profile) = self.cached_profile(user_id, guild_id, kind) {
            return Ok(profile);
        }
        if let Some(error) = self
            .cache()
            .failure(CacheRequest::Profile(user_id, guild_id), Instant::now())
        {
            return Err(error);
        }
        let profile = match self
            .rest(async |rest| {
                rest.fetch_profile(user_id, guild_id, channel_id, kind)
                    .await
            })
            .await
        {
            Ok(profile) => profile,
            Err(error) => {
                self.cache().remember_failure(
                    CacheRequest::Profile(user_id, guild_id),
                    &error,
                    Instant::now(),
                );
                return Err(error);
            }
        };
        let profile = Arc::new(profile);
        self.cache()
            .insert_profile(key, kind, profile.clone(), Instant::now());
        Ok(profile)
    }

    pub(crate) fn cached_profile(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        kind: ProfileKind,
    ) -> Option<Arc<model::Profile>> {
        self.cache()
            .profile((user_id, guild_id), kind, Instant::now())
    }

    pub(crate) fn cached_profile_details(
        &self,
        user_id: u64,
        game_ids: &[u64],
        application_ids: &[u64],
    ) -> ProfileDetails {
        self.cache()
            .profile_details(user_id, game_ids, application_ids, Instant::now())
    }

    pub async fn collectible_product(
        &self,
        sku_id: u64,
    ) -> Result<Arc<model::CollectibleProduct>, Error> {
        let _request = self
            .request_lock(CacheRequest::Product(sku_id))
            .lock_owned()
            .await;
        if let Some(product) = self.cache().products.get(&sku_id) {
            return Ok(product.clone());
        }
        if let Some(error) = self
            .cache()
            .failure(CacheRequest::Product(sku_id), Instant::now())
        {
            return Err(error);
        }
        let product = match self
            .rest(async |rest| rest.fetch_collectible_product(sku_id).await)
            .await
        {
            Ok(product) => product,
            Err(error) => {
                self.cache()
                    .remember_failure(CacheRequest::Product(sku_id), &error, Instant::now());
                return Err(error);
            }
        };
        let product = Arc::new(product);
        self.cache().insert_product(sku_id, product.clone());
        Ok(product)
    }

    pub async fn outbox(&self, user_id: u64) -> Result<Arc<model::Outbox>, Error> {
        let _request = self
            .request_lock(CacheRequest::Outbox(user_id))
            .lock_owned()
            .await;
        if let Some(outbox) = self.cache().outbox(user_id, Instant::now()) {
            return Ok(outbox);
        }
        if let Some(error) = self
            .cache()
            .failure(CacheRequest::Outbox(user_id), Instant::now())
        {
            return Err(error);
        }
        let outbox = match self
            .rest(async |rest| rest.fetch_outbox(user_id).await)
            .await
        {
            Ok(outbox) => outbox,
            Err(error) => {
                self.cache()
                    .remember_failure(CacheRequest::Outbox(user_id), &error, Instant::now());
                return Err(error);
            }
        };
        let outbox = Arc::new(outbox);
        self.cache()
            .insert_outbox(user_id, outbox.clone(), Instant::now());
        Ok(outbox)
    }

    pub async fn wishlist(&self, wishlist_id: u64) -> Result<Arc<model::Wishlist>, Error> {
        let _request = self
            .request_lock(CacheRequest::Wishlist(wishlist_id))
            .lock_owned()
            .await;
        if let Some(wishlist) = self.cache().wishlist(wishlist_id, Instant::now()) {
            return Ok(wishlist);
        }
        if let Some(error) = self
            .cache()
            .failure(CacheRequest::Wishlist(wishlist_id), Instant::now())
        {
            return Err(error);
        }
        let wishlist = match self
            .rest(async |rest| rest.fetch_wishlist(wishlist_id).await)
            .await
        {
            Ok(wishlist) => wishlist,
            Err(error) => {
                self.cache().remember_failure(
                    CacheRequest::Wishlist(wishlist_id),
                    &error,
                    Instant::now(),
                );
                return Err(error);
            }
        };
        let wishlist = Arc::new(wishlist);
        self.cache()
            .insert_wishlist(wishlist_id, wishlist.clone(), Instant::now());
        Ok(wishlist)
    }

    pub async fn note(&self, user_id: u64) -> Result<Option<String>, Error> {
        self.rest(async |rest| rest.fetch_note(user_id).await)
            .await
    }

    /// `None` means the invite code is invalid or expired; that result is cached too.
    pub async fn invite(&self, code: &str) -> Result<Option<Arc<model::Invite>>, Error> {
        let _request = self
            .request_lock(CacheRequest::Invite(code.to_owned()))
            .lock_owned()
            .await;
        if let Some(invite) = self.cache().invite(code, Instant::now()) {
            return Ok(invite);
        }
        let key = CacheRequest::Invite(code.to_owned());
        if let Some(error) = self.cache().failure(key.clone(), Instant::now()) {
            return Err(error);
        }
        let invite = match self.rest(async |rest| rest.fetch_invite(code).await).await {
            Ok(invite) => invite,
            Err(error) => {
                self.cache().remember_failure(key, &error, Instant::now());
                return Err(error);
            }
        };
        let invite = invite.map(Arc::new);
        self.cache()
            .insert_invite(code.to_owned(), invite.clone(), Instant::now());
        Ok(invite)
    }

    pub async fn user_badges(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
    ) -> Result<Arc<model::UserBadges>, Error> {
        let _request = self
            .request_lock(CacheRequest::Badges(user_id))
            .lock_owned()
            .await;
        if let Some(badges) = self.cache().badges(user_id, Instant::now()) {
            return Ok(badges);
        }
        if let Some(error) = self
            .cache()
            .failure(CacheRequest::Badges(user_id), Instant::now())
        {
            return Err(error);
        }
        let badges = match self
            .rest(async |rest| rest.fetch_user_badges(user_id, guild_id, channel_id).await)
            .await
        {
            Ok(badges) => badges,
            Err(error) => {
                self.cache()
                    .remember_failure(CacheRequest::Badges(user_id), &error, Instant::now());
                return Err(error);
            }
        };
        let badges = Arc::new(badges);
        self.cache()
            .insert_badges(user_id, badges.clone(), Instant::now());
        Ok(badges)
    }

    pub async fn application_identities(
        &self,
        user_id: u64,
    ) -> Result<Arc<model::ApplicationIdentities>, Error> {
        let _request = self
            .request_lock(CacheRequest::Identities(user_id))
            .lock_owned()
            .await;
        if let Some(identities) = self.cache().identities(user_id, Instant::now()) {
            return Ok(identities);
        }
        if let Some(error) = self
            .cache()
            .failure(CacheRequest::Identities(user_id), Instant::now())
        {
            return Err(error);
        }
        let identities = match self
            .rest(async |rest| rest.fetch_application_identities(user_id).await)
            .await
        {
            Ok(identities) => identities,
            Err(error) => {
                self.cache().remember_failure(
                    CacheRequest::Identities(user_id),
                    &error,
                    Instant::now(),
                );
                return Err(error);
            }
        };
        let identities = Arc::new(identities);
        self.cache()
            .insert_identities(user_id, identities.clone(), Instant::now());
        Ok(identities)
    }

    pub async fn games(&self, ids: &[u64]) -> Result<Vec<Arc<model::Game>>, Error> {
        let _request = self.request_lock(CacheRequest::Games).lock_owned().await;
        let missing: Vec<u64> = {
            let cache = self.cache();
            missing_ids(&cache.games, &cache.missing_games, ids, Instant::now())
        };
        if !missing.is_empty() {
            let games = self
                .rest(async |rest| rest.fetch_games(&missing).await)
                .await?;
            let mut missing: HashSet<_> = missing.into_iter().collect();
            let mut cache = self.cache();
            for game in games {
                let game = Arc::new(game);
                missing.remove(&game.id);
                cache.missing_games.remove(&game.id);
                insert_bounded(&mut cache.games, game.id, game);
            }
            let now = Instant::now();
            for id in missing {
                if !cache.games.contains_key(&id) {
                    insert_missing(&mut cache.missing_games, id, now);
                }
            }
        }
        let cache = self.cache();
        Ok(ids
            .iter()
            .filter_map(|id| cache.games.get(id).cloned())
            .collect())
    }

    pub async fn public_applications(
        &self,
        ids: &[u64],
    ) -> Result<Vec<Arc<model::PublicApplication>>, Error> {
        let _request = self
            .request_lock(CacheRequest::Applications)
            .lock_owned()
            .await;
        let missing: Vec<u64> = {
            let cache = self.cache();
            missing_ids(
                &cache.applications,
                &cache.missing_applications,
                ids,
                Instant::now(),
            )
        };
        if !missing.is_empty() {
            let applications = self
                .rest(async |rest| rest.fetch_public_applications(&missing).await)
                .await?;
            let mut missing: HashSet<_> = missing.into_iter().collect();
            let mut cache = self.cache();
            for application in applications {
                let application = Arc::new(application);
                missing.remove(&application.id);
                cache.missing_applications.remove(&application.id);
                insert_bounded(&mut cache.applications, application.id, application);
            }
            let now = Instant::now();
            for id in missing {
                if !cache.applications.contains_key(&id) {
                    insert_missing(&mut cache.missing_applications, id, now);
                }
            }
        }
        let cache = self.cache();
        Ok(ids
            .iter()
            .filter_map(|id| cache.applications.get(id).cloned())
            .collect())
    }

    pub async fn widget_config(
        &self,
        application_id: u64,
    ) -> Result<Option<Arc<model::WidgetConfig>>, Error> {
        let _request = self
            .request_lock(CacheRequest::WidgetConfigs)
            .lock_owned()
            .await;
        let featured = self.cache().featured_widget_configs.clone();
        let featured = match featured {
            Some(featured) => featured,
            None => {
                if let Some(error) = self
                    .cache()
                    .failure(CacheRequest::WidgetConfigs, Instant::now())
                {
                    return Err(error);
                }
                let mut configs = HashMap::new();
                let fetched = match self
                    .rest(async |rest| rest.fetch_featured_widget_configs().await)
                    .await
                {
                    Ok(fetched) => fetched,
                    Err(error) => {
                        self.cache().remember_failure(
                            CacheRequest::WidgetConfigs,
                            &error,
                            Instant::now(),
                        );
                        return Err(error);
                    }
                };
                for (id, config) in fetched {
                    configs.insert(id, Arc::new(config));
                }
                self.cache().featured_widget_configs = Some(configs.clone());
                configs
            }
        };
        if let Some(config) = featured.get(&application_id) {
            return Ok(Some(config.clone()));
        }
        if let Some(config) = self.cache().widget_configs_by_app.get(&application_id) {
            return Ok(config.clone());
        }
        let key = CacheRequest::WidgetConfig(application_id);
        if let Some(error) = self.cache().failure(key.clone(), Instant::now()) {
            return Err(error);
        }
        let config = match self
            .rest(async |rest| rest.fetch_application_widget_configs(application_id).await)
            .await
        {
            Ok(config) => config.map(Arc::new),
            Err(error) => {
                self.cache().remember_failure(key, &error, Instant::now());
                return Err(error);
            }
        };
        self.cache()
            .insert_widget_config(application_id, config.clone());
        Ok(config)
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
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileKind {
    Popout,
    Modal,
}

const PROFILE_TTL: Duration = Duration::from_secs(5 * 60);
const FAILURE_TTL: Duration = Duration::from_secs(60);
const CACHE_LIMIT: usize = 256;

#[derive(Clone, PartialEq, Eq, Hash)]
enum CacheRequest {
    Profile(u64, Option<u64>),
    Product(u64),
    Outbox(u64),
    Wishlist(u64),
    Invite(String),
    Identities(u64),
    Games,
    Applications,
    WidgetConfigs,
    WidgetConfig(u64),
    Badges(u64),
}

pub(crate) struct ProfileDetails {
    pub games: HashMap<u64, Arc<model::Game>>,
    pub applications: HashMap<u64, Arc<model::PublicApplication>>,
    pub identities: Option<Arc<model::ApplicationIdentities>>,
    pub outbox: Option<Arc<model::Outbox>>,
    pub badges: Option<Arc<model::UserBadges>>,
    pub widget_configs: HashMap<u64, Arc<model::WidgetConfig>>,
}

#[derive(Default)]
struct Cache {
    profiles: HashMap<(u64, Option<u64>), CachedProfile>,
    products: HashMap<u64, Arc<model::CollectibleProduct>>,
    outboxes: HashMap<u64, Cached<model::Outbox>>,
    wishlists: HashMap<u64, Cached<model::Wishlist>>,
    invites: HashMap<String, CachedInvite>,
    identities: HashMap<u64, Cached<model::ApplicationIdentities>>,
    badges: HashMap<u64, Cached<model::UserBadges>>,
    games: HashMap<u64, Arc<model::Game>>,
    missing_games: HashMap<u64, Instant>,
    applications: HashMap<u64, Arc<model::PublicApplication>>,
    missing_applications: HashMap<u64, Instant>,
    featured_widget_configs: Option<HashMap<u64, Arc<model::WidgetConfig>>>,
    widget_configs_by_app: HashMap<u64, Option<Arc<model::WidgetConfig>>>,
    failures: HashMap<CacheRequest, (Instant, Error)>,
}

struct CachedProfile {
    kind: ProfileKind,
    fetched: Instant,
    profile: Arc<model::Profile>,
}

struct Cached<T> {
    fetched: Instant,
    value: Arc<T>,
}

struct CachedInvite {
    fetched: Instant,
    invite: Option<Arc<model::Invite>>,
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
        // A popout started before the modal must not replace its richer cached response.
        if kind == ProfileKind::Popout && self.profile(key, ProfileKind::Modal, now).is_some() {
            return;
        }
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

    fn profile_details(
        &self,
        user_id: u64,
        game_ids: &[u64],
        application_ids: &[u64],
        now: Instant,
    ) -> ProfileDetails {
        let outbox = self.outbox(user_id, now);
        let mut applications: HashSet<_> = application_ids.iter().copied().collect();
        for entry in outbox.iter().flat_map(|outbox| &outbox.entries) {
            if let model::OutboxKind::PlayedGame {
                application_id: Some(id),
                ..
            }
            | model::OutboxKind::LaunchedActivity {
                application_id: Some(id),
                ..
            } = &entry.kind
            {
                applications.insert(*id);
            }
        }
        ProfileDetails {
            games: game_ids
                .iter()
                .filter_map(|id| Some((*id, self.games.get(id)?.clone())))
                .collect(),
            applications: applications
                .into_iter()
                .filter_map(|id| Some((id, self.applications.get(&id)?.clone())))
                .collect(),
            identities: self.identities(user_id, now),
            outbox,
            badges: self.badges(user_id, now),
            widget_configs: application_ids
                .iter()
                .filter_map(|id| {
                    let config = self
                        .featured_widget_configs
                        .as_ref()
                        .and_then(|configs| configs.get(id))
                        .or_else(|| self.widget_configs_by_app.get(id)?.as_ref())?;
                    Some((*id, config.clone()))
                })
                .collect(),
        }
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

    fn outbox(&self, user_id: u64, now: Instant) -> Option<Arc<model::Outbox>> {
        cached_value(&self.outboxes, user_id, now)
    }

    fn insert_outbox(&mut self, user_id: u64, outbox: Arc<model::Outbox>, now: Instant) {
        insert_cached(&mut self.outboxes, user_id, outbox, now);
    }

    fn wishlist(&self, wishlist_id: u64, now: Instant) -> Option<Arc<model::Wishlist>> {
        cached_value(&self.wishlists, wishlist_id, now)
    }

    fn insert_wishlist(
        &mut self,
        wishlist_id: u64,
        wishlist: Arc<model::Wishlist>,
        now: Instant,
    ) {
        insert_cached(&mut self.wishlists, wishlist_id, wishlist, now);
    }

    fn invite(&self, code: &str, now: Instant) -> Option<Option<Arc<model::Invite>>> {
        let cached = self.invites.get(code)?;
        (now.saturating_duration_since(cached.fetched) < PROFILE_TTL)
            .then(|| cached.invite.clone())
    }

    fn insert_invite(&mut self, code: String, invite: Option<Arc<model::Invite>>, now: Instant) {
        if self.invites.len() >= CACHE_LIMIT && !self.invites.contains_key(&code) {
            let oldest = self
                .invites
                .iter()
                .min_by_key(|(_, cached)| cached.fetched)
                .map(|(code, _)| code.clone());
            if let Some(oldest) = oldest {
                self.invites.remove(&oldest);
            }
        }
        self.invites.insert(code, CachedInvite { fetched: now, invite });
    }

    fn identities(&self, user_id: u64, now: Instant) -> Option<Arc<model::ApplicationIdentities>> {
        cached_value(&self.identities, user_id, now)
    }

    fn insert_identities(
        &mut self,
        user_id: u64,
        identities: Arc<model::ApplicationIdentities>,
        now: Instant,
    ) {
        insert_cached(&mut self.identities, user_id, identities, now);
    }

    fn badges(&self, user_id: u64, now: Instant) -> Option<Arc<model::UserBadges>> {
        cached_value(&self.badges, user_id, now)
    }

    fn insert_badges(&mut self, user_id: u64, badges: Arc<model::UserBadges>, now: Instant) {
        insert_cached(&mut self.badges, user_id, badges, now);
    }

    fn insert_widget_config(
        &mut self,
        application_id: u64,
        config: Option<Arc<model::WidgetConfig>>,
    ) {
        if self.widget_configs_by_app.len() >= CACHE_LIMIT
            && !self.widget_configs_by_app.contains_key(&application_id)
        {
            if let Some(any) = self.widget_configs_by_app.keys().next().copied() {
                self.widget_configs_by_app.remove(&any);
            }
        }
        self.widget_configs_by_app.insert(application_id, config);
    }

    fn failure(&self, key: CacheRequest, now: Instant) -> Option<Error> {
        self.failures
            .get(&key)
            .filter(|(at, _)| now.saturating_duration_since(*at) < FAILURE_TTL)
            .map(|(_, error)| error.clone())
    }

    fn remember_failure(&mut self, key: CacheRequest, error: &Error, now: Instant) {
        let remember = match error {
            Error::Rejected { .. } | Error::Decode => true,
            Error::Http(status) => *status != 401,
            _ => false,
        };
        if !remember {
            return;
        }
        if self.failures.len() >= CACHE_LIMIT && !self.failures.contains_key(&key) {
            self.failures
                .retain(|_, (at, _)| now.saturating_duration_since(*at) < FAILURE_TTL);
            if self.failures.len() >= CACHE_LIMIT {
                let oldest = self
                    .failures
                    .iter()
                    .min_by_key(|(_, (at, _))| *at)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    self.failures.remove(&oldest);
                }
            }
        }
        self.failures.insert(key, (now, error.clone()));
    }
}

fn missing_ids<T>(
    known: &HashMap<u64, Arc<T>>,
    absent: &HashMap<u64, Instant>,
    ids: &[u64],
    now: Instant,
) -> Vec<u64> {
    let mut seen = HashSet::new();
    ids.iter()
        .copied()
        .filter(|id| {
            seen.insert(*id)
                && !known.contains_key(id)
                && !absent
                    .get(id)
                    .is_some_and(|fetched| now.saturating_duration_since(*fetched) < PROFILE_TTL)
        })
        .collect()
}

// Only successful lookups populate this cache; network failures must remain retryable.
fn insert_missing(map: &mut HashMap<u64, Instant>, id: u64, now: Instant) {
    if map.len() >= CACHE_LIMIT && !map.contains_key(&id) {
        if let Some(oldest) = map
            .iter()
            .min_by_key(|(_, fetched)| **fetched)
            .map(|(id, _)| *id)
        {
            map.remove(&oldest);
        }
    }
    map.insert(id, now);
}

fn cached_value<T>(map: &HashMap<u64, Cached<T>>, key: u64, now: Instant) -> Option<Arc<T>> {
    let cached = map.get(&key)?;
    (now.saturating_duration_since(cached.fetched) < PROFILE_TTL).then(|| cached.value.clone())
}

fn insert_cached<T>(map: &mut HashMap<u64, Cached<T>>, key: u64, value: Arc<T>, now: Instant) {
    if map.len() >= CACHE_LIMIT && !map.contains_key(&key) {
        let oldest = map
            .iter()
            .min_by_key(|(_, cached)| cached.fetched)
            .map(|(key, _)| *key);
        if let Some(oldest) = oldest {
            map.remove(&oldest);
        }
    }
    map.insert(
        key,
        Cached {
            fetched: now,
            value,
        },
    );
}

fn insert_bounded<T>(map: &mut HashMap<u64, Arc<T>>, key: u64, value: Arc<T>) {
    if map.len() >= CACHE_LIMIT && !map.contains_key(&key) {
        if let Some(any) = map.keys().next().copied() {
            map.remove(&any);
        }
    }
    map.insert(key, value);
}

pub enum Event {
    ReadySupplemental(Box<model::ReadySupplemental>),
    PresenceUpdate(Box<model::Presence>),
    SessionsReplace(Vec<model::Session>),
    MembersChunk(Box<model::MembersChunk>),
    MemberListUpdate(Box<model::MemberListUpdate>),
    MemberUpsert(Box<model::MemberUpdate>),
    MemberRemove(Box<model::MemberUpdate>),
    RoleUpsert(Box<model::RoleUpdate>),
    RoleDelete { guild_id: u64, role_id: u64 },
    UserUpdate(Box<model::UserUpdate>),
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
    RelationshipAdd(Box<model::Relationship>),
    RelationshipUpdate(Box<model::RelationshipUpdate>),
    RelationshipRemove { id: u64 },
    ThreadListSync(Box<model::ThreadListSync>),
    GuildUpsert(Box<model::Guild>),
    GuildEmojisUpdate(model::GuildEmojisUpdate),
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

    fn offline_session() -> Session {
        let (control, _) = watch::channel(Control::Running);
        let (commands, _) = tokio::sync::mpsc::unbounded_channel();
        Session {
            inner: Arc::new(Inner {
                rest: Err(Error::Network),
                gif_client: Err(Error::Network),
                cache: Mutex::default(),
                requests: Mutex::default(),
                sends: Mutex::default(),
                control,
                commands,
                generation: Arc::new(AtomicU64::new(0)),
            }),
        }
    }

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
    fn late_popout_does_not_downgrade_or_extend_the_modal_cache() {
        let mut cache = Cache::default();
        let start = Instant::now();
        let key = (1, Some(2));
        let modal = profile();
        cache.insert_profile(key, ProfileKind::Modal, modal.clone(), start);
        cache.insert_profile(
            key,
            ProfileKind::Popout,
            profile(),
            start + Duration::from_secs(1),
        );
        assert!(Arc::ptr_eq(
            &cache.profile(key, ProfileKind::Modal, start).unwrap(),
            &modal
        ));
        let expired = start + PROFILE_TTL;
        assert!(cache.profile(key, ProfileKind::Modal, expired).is_none());
        cache.insert_profile(key, ProfileKind::Popout, profile(), expired);
        assert!(cache.profile(key, ProfileKind::Popout, expired).is_some());
        assert!(cache.profile(key, ProfileKind::Modal, expired).is_none());
    }

    #[test]
    fn profile_details_expose_cached_items_without_waiting_for_missing_ones() {
        let mut cache = Cache::default();
        let start = Instant::now();
        let game: Arc<model::Game> = Arc::new(
            serde_json::from_value(serde_json::json!({"id": "7", "name": "Game"})).unwrap(),
        );
        cache.games.insert(7, game.clone());
        for id in [7, 8, 9] {
            let application: Arc<model::PublicApplication> = Arc::new(
                serde_json::from_value(serde_json::json!({"id": id.to_string(), "name": "App"}))
                    .unwrap(),
            );
            cache.applications.insert(id, application);
        }
        let outbox: Arc<model::Outbox> = Arc::new(
            serde_json::from_value(serde_json::json!({"entries": [{
                "id": "10",
                "extra": {"type": "played_game_extra", "application_id": "8"}
            }]}))
            .unwrap(),
        );
        cache.insert_outbox(1, outbox.clone(), start);
        let identities = Arc::new(model::ApplicationIdentities::default());
        cache.insert_identities(1, identities.clone(), start);
        let config = Arc::new(
            model::parse_application_widget_configs(serde_json::json!({"surfaces": {}}), 7)
                .unwrap(),
        );
        cache.insert_widget_config(7, Some(config.clone()));

        let details = cache.profile_details(1, &[7, 404], &[7, 404], start);
        assert_eq!(details.games.len(), 1);
        assert!(Arc::ptr_eq(&details.games[&7], &game));
        assert_eq!(details.applications.len(), 2);
        assert!(details.applications.contains_key(&8));
        assert!(!details.applications.contains_key(&9));
        assert!(Arc::ptr_eq(details.outbox.as_ref().unwrap(), &outbox));
        assert!(Arc::ptr_eq(
            details.identities.as_ref().unwrap(),
            &identities
        ));
        assert!(Arc::ptr_eq(&details.widget_configs[&7], &config));

        let other = cache.profile_details(2, &[], &[7], start);
        assert!(other.outbox.is_none());
        assert!(other.identities.is_none());
        assert!(!other.applications.contains_key(&8));
        let expired = cache.profile_details(1, &[7], &[7], start + PROFILE_TTL);
        assert!(expired.outbox.is_none());
        assert!(expired.identities.is_none());
        assert!(!expired.applications.contains_key(&8));
        assert_eq!(expired.games.len(), 1);
        assert_eq!(expired.widget_configs.len(), 1);

        cache.featured_widget_configs = Some(HashMap::from([(7, config.clone())]));
        cache.widget_configs_by_app.clear();
        let featured = cache.profile_details(1, &[], &[7], start);
        assert!(Arc::ptr_eq(&featured.widget_configs[&7], &config));
    }

    #[test]
    fn missing_metadata_is_deduplicated_and_retried_after_expiry() {
        let start = Instant::now();
        let known = HashMap::from([(1, Arc::new(()))]);
        let mut absent = HashMap::new();
        insert_missing(&mut absent, 2, start);
        assert_eq!(missing_ids(&known, &absent, &[1, 2, 3, 3], start), [3]);
        assert_eq!(
            missing_ids(&known, &absent, &[1, 2, 3, 3], start + PROFILE_TTL),
            [2, 3]
        );
    }

    #[test]
    fn missing_metadata_cache_is_bounded_and_evicts_the_oldest() {
        let start = Instant::now();
        let mut absent = HashMap::new();
        for id in 0..CACHE_LIMIT as u64 {
            insert_missing(&mut absent, id, start + Duration::from_secs(id));
        }
        insert_missing(&mut absent, 999, start + Duration::from_secs(999));
        assert_eq!(absent.len(), CACHE_LIMIT);
        assert!(!absent.contains_key(&0));
        assert!(absent.contains_key(&999));
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

    #[test]
    fn outbox_and_identity_caches_expire_with_profiles() {
        let mut cache = Cache::default();
        let start = Instant::now();
        let outbox = Arc::new(serde_json::from_value(serde_json::json!({"entries": []})).unwrap());
        let identities =
            Arc::new(serde_json::from_value(serde_json::json!({"identities": []})).unwrap());
        cache.insert_outbox(7, outbox, start);
        cache.insert_identities(7, identities, start);
        assert!(cache.outbox(7, start).is_some());
        assert!(cache.identities(7, start).is_some());
        assert!(cache.outbox(8, start).is_none());
        assert!(cache.identities(8, start).is_none());
        let expired = start + PROFILE_TTL;
        assert!(cache.outbox(7, expired).is_none());
        assert!(cache.identities(7, expired).is_none());
    }

    #[test]
    fn failed_lookups_are_remembered_only_when_discord_rejects_them() {
        let mut cache = Cache::default();
        let start = Instant::now();
        let rejected = Error::Rejected {
            status: 403,
            code: 50001,
            message: "x".into(),
        };
        cache.remember_failure(CacheRequest::Outbox(1), &rejected, start);
        assert!(cache.failure(CacheRequest::Outbox(1), start).is_some());
        assert!(
            cache
                .failure(CacheRequest::Outbox(1), start + FAILURE_TTL)
                .is_none()
        );
        cache.remember_failure(CacheRequest::Identities(1), &Error::Network, start);
        cache.remember_failure(CacheRequest::Identities(1), &Error::Http(401), start);
        assert!(
            cache
                .failure(CacheRequest::Identities(1), start)
                .is_none()
        );
    }

    #[test]
    fn game_and_application_caches_are_per_id_and_bounded() {
        let mut cache = Cache::default();
        for id in 0..CACHE_LIMIT as u64 {
            let game: Arc<model::Game> = Arc::new(
                serde_json::from_value(serde_json::json!({"id": id.to_string(), "name": "Game"}))
                    .unwrap(),
            );
            insert_bounded(&mut cache.games, id, game);
            let application: Arc<model::PublicApplication> = Arc::new(
                serde_json::from_value(
                    serde_json::json!({"id": id.to_string(), "name": "Application"}),
                )
                .unwrap(),
            );
            insert_bounded(&mut cache.applications, id, application);
        }
        assert_eq!(cache.games.len(), CACHE_LIMIT);
        assert_eq!(cache.applications.len(), CACHE_LIMIT);
        let game: Arc<model::Game> = Arc::new(
            serde_json::from_value(serde_json::json!({"id": "999", "name": "Newest"})).unwrap(),
        );
        insert_bounded(&mut cache.games, 999, game);
        let application: Arc<model::PublicApplication> = Arc::new(
            serde_json::from_value(serde_json::json!({"id": "999", "name": "Newest"})).unwrap(),
        );
        insert_bounded(&mut cache.applications, 999, application);
        assert_eq!(cache.games.len(), CACHE_LIMIT);
        assert_eq!(cache.applications.len(), CACHE_LIMIT);
        assert!(cache.games.contains_key(&999));
        assert!(cache.applications.contains_key(&999));
    }

    #[test]
    fn request_locks_share_resources_and_discard_unused_keys() {
        let session = offline_session();
        let profile = session.request_lock(CacheRequest::Profile(1, Some(2)));
        let same = session.request_lock(CacheRequest::Profile(1, Some(2)));
        assert!(Arc::ptr_eq(&profile, &same));
        assert!(!Arc::ptr_eq(
            &profile,
            &session.request_lock(CacheRequest::Profile(1, None))
        ));
        assert!(!Arc::ptr_eq(
            &profile,
            &session.request_lock(CacheRequest::Outbox(1))
        ));
        for id in 0..1000 {
            drop(session.request_lock(CacheRequest::Identities(id)));
        }
        let outbox = session.request_lock(CacheRequest::Outbox(1));
        assert_eq!(session.inner.requests.lock().unwrap().len(), 2);
        drop((profile, same, outbox));
        let _games = session.request_lock(CacheRequest::Games);
        assert_eq!(session.inner.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn overlapping_profile_loads_recheck_the_cache_after_waiting() {
        let session = offline_session();
        let fetching = session
            .request_lock(CacheRequest::Profile(1, Some(2)))
            .lock_owned()
            .await;
        let full = session.profile(1, Some(2), None, ProfileKind::Modal);
        let popout = session.profile(1, Some(2), None, ProfileKind::Popout);
        tokio::pin!(full, popout);
        assert!(futures::poll!(&mut full).is_pending());
        assert!(futures::poll!(&mut popout).is_pending());
        let profile = profile();
        session.cache().insert_profile(
            (1, Some(2)),
            ProfileKind::Modal,
            profile.clone(),
            Instant::now(),
        );
        drop(fetching);
        let (full, popout) = tokio::join!(full, popout);
        assert!(Arc::ptr_eq(&full.unwrap(), &profile));
        assert!(Arc::ptr_eq(&popout.unwrap(), &profile));
    }

    #[tokio::test]
    async fn activity_resources_recheck_cache_independently() {
        let session = offline_session();
        let games = session.request_lock(CacheRequest::Games).lock_owned().await;
        let outboxes = session
            .request_lock(CacheRequest::Outbox(1))
            .lock_owned()
            .await;
        let identities = session
            .request_lock(CacheRequest::Identities(1))
            .lock_owned()
            .await;
        let applications = session
            .request_lock(CacheRequest::Applications)
            .lock_owned()
            .await;
        let game_load = session.games(&[7]);
        let outbox_load = session.outbox(1);
        let identity_load = session.application_identities(1);
        let application_load = session.public_applications(&[7]);
        tokio::pin!(game_load, outbox_load, identity_load, application_load);
        assert!(futures::poll!(&mut game_load).is_pending());
        assert!(futures::poll!(&mut outbox_load).is_pending());
        assert!(futures::poll!(&mut identity_load).is_pending());
        assert!(futures::poll!(&mut application_load).is_pending());
        {
            let mut cache = session.cache();
            cache.games.insert(
                7,
                Arc::new(
                    serde_json::from_value(serde_json::json!({"id": "7", "name": "Game"})).unwrap(),
                ),
            );
            cache.applications.insert(
                7,
                Arc::new(
                    serde_json::from_value(serde_json::json!({"id": "7", "name": "App"})).unwrap(),
                ),
            );
            cache.insert_outbox(
                1,
                Arc::new(serde_json::from_value(serde_json::json!({"entries": []})).unwrap()),
                Instant::now(),
            );
            cache.insert_identities(
                1,
                Arc::new(model::ApplicationIdentities::default()),
                Instant::now(),
            );
        }
        // The games request can remain blocked without delaying an already fetched outbox.
        drop(outboxes);
        assert!(outbox_load.await.is_ok());
        drop((games, identities, applications));
        assert_eq!(game_load.await.unwrap()[0].id, 7);
        assert!(identity_load.await.is_ok());
        assert_eq!(application_load.await.unwrap()[0].id, 7);
    }
}
