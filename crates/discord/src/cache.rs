use super::*;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum CacheRequest {
    Profile(u64, Option<u64>),
    Product(u64),
    CollectibleCatalog,
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

pub struct ProfileDetails {
    pub games: HashMap<u64, Arc<model::Game>>,
    pub applications: HashMap<u64, Arc<model::PublicApplication>>,
    pub identities: Option<Arc<model::ApplicationIdentities>>,
    pub outbox: Option<Arc<model::Outbox>>,
    pub badges: Option<Arc<model::UserBadges>>,
    pub widget_configs: HashMap<u64, Arc<model::WidgetConfig>>,
}

#[derive(Default)]
pub(super) struct Cache {
    pub(super) profiles: HashMap<(u64, Option<u64>), CachedProfile>,
    pub(super) profile_revision: u64,
    pub(super) products: HashMap<u64, Arc<model::CollectibleProduct>>,
    pub(super) collectible_catalog: Option<Arc<Vec<Arc<model::CollectibleProduct>>>>,
    pub(super) outboxes: HashMap<u64, Cached<model::Outbox>>,
    pub(super) wishlists: HashMap<u64, Cached<model::Wishlist>>,
    pub(super) invites: HashMap<String, CachedInvite>,
    pub(super) identities: HashMap<u64, Cached<model::ApplicationIdentities>>,
    pub(super) badges: HashMap<u64, Cached<model::UserBadges>>,
    pub(super) games: HashMap<u64, Arc<model::Game>>,
    pub(super) missing_games: HashMap<u64, Instant>,
    pub(super) applications: HashMap<u64, Arc<model::PublicApplication>>,
    pub(super) missing_applications: HashMap<u64, Instant>,
    pub(super) featured_widget_configs: Option<HashMap<u64, Arc<model::WidgetConfig>>>,
    pub(super) widget_configs_by_app: HashMap<u64, Option<Arc<model::WidgetConfig>>>,
    pub(super) failures: HashMap<CacheRequest, (Instant, Error)>,
}

pub(super) struct CachedProfile {
    kind: ProfileKind,
    fetched: Instant,
    profile: Arc<model::Profile>,
}

pub(super) struct Cached<T> {
    fetched: Instant,
    value: Arc<T>,
}

pub(super) struct CachedInvite {
    fetched: Instant,
    invite: Option<Arc<model::Invite>>,
}

impl Cache {
    pub(super) fn profile(
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

    pub(super) fn insert_profile(
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

    pub(super) fn profile_details(
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

    pub(super) fn insert_product(&mut self, sku_id: u64, product: Arc<model::CollectibleProduct>) {
        if self.products.len() >= CACHE_LIMIT {
            let any = self.products.keys().next().copied();
            if let Some(any) = any {
                self.products.remove(&any);
            }
        }
        self.products.insert(sku_id, product);
    }

    pub(super) fn outbox(&self, user_id: u64, now: Instant) -> Option<Arc<model::Outbox>> {
        cached_value(&self.outboxes, user_id, now)
    }

    pub(super) fn insert_outbox(&mut self, user_id: u64, outbox: Arc<model::Outbox>, now: Instant) {
        insert_cached(&mut self.outboxes, user_id, outbox, now);
    }

    pub(super) fn wishlist(&self, wishlist_id: u64, now: Instant) -> Option<Arc<model::Wishlist>> {
        cached_value(&self.wishlists, wishlist_id, now)
    }

    pub(super) fn insert_wishlist(
        &mut self,
        wishlist_id: u64,
        wishlist: Arc<model::Wishlist>,
        now: Instant,
    ) {
        insert_cached(&mut self.wishlists, wishlist_id, wishlist, now);
    }

    pub(super) fn invite(&self, code: &str, now: Instant) -> Option<Option<Arc<model::Invite>>> {
        let cached = self.invites.get(code)?;
        (now.saturating_duration_since(cached.fetched) < PROFILE_TTL).then(|| cached.invite.clone())
    }

    pub(super) fn insert_invite(
        &mut self,
        code: String,
        invite: Option<Arc<model::Invite>>,
        now: Instant,
    ) {
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
        self.invites.insert(
            code,
            CachedInvite {
                fetched: now,
                invite,
            },
        );
    }

    pub(super) fn identities(
        &self,
        user_id: u64,
        now: Instant,
    ) -> Option<Arc<model::ApplicationIdentities>> {
        cached_value(&self.identities, user_id, now)
    }

    pub(super) fn insert_identities(
        &mut self,
        user_id: u64,
        identities: Arc<model::ApplicationIdentities>,
        now: Instant,
    ) {
        insert_cached(&mut self.identities, user_id, identities, now);
    }

    pub(super) fn badges(&self, user_id: u64, now: Instant) -> Option<Arc<model::UserBadges>> {
        cached_value(&self.badges, user_id, now)
    }

    pub(super) fn insert_badges(
        &mut self,
        user_id: u64,
        badges: Arc<model::UserBadges>,
        now: Instant,
    ) {
        insert_cached(&mut self.badges, user_id, badges, now);
    }

    pub(super) fn insert_widget_config(
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

    pub(super) fn failure(&self, key: CacheRequest, now: Instant) -> Option<Error> {
        self.failures
            .get(&key)
            .filter(|(at, _)| now.saturating_duration_since(*at) < FAILURE_TTL)
            .map(|(_, error)| error.clone())
    }

    pub(super) fn remember_failure(&mut self, key: CacheRequest, error: &Error, now: Instant) {
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

pub(super) fn missing_ids<T>(
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
pub(super) fn insert_missing(map: &mut HashMap<u64, Instant>, id: u64, now: Instant) {
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

pub(super) fn cached_value<T>(
    map: &HashMap<u64, Cached<T>>,
    key: u64,
    now: Instant,
) -> Option<Arc<T>> {
    let cached = map.get(&key)?;
    (now.saturating_duration_since(cached.fetched) < PROFILE_TTL).then(|| cached.value.clone())
}

pub(super) fn insert_cached<T>(
    map: &mut HashMap<u64, Cached<T>>,
    key: u64,
    value: Arc<T>,
    now: Instant,
) {
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

pub(super) fn insert_bounded<T>(map: &mut HashMap<u64, Arc<T>>, key: u64, value: Arc<T>) {
    if map.len() >= CACHE_LIMIT && !map.contains_key(&key) {
        if let Some(any) = map.keys().next().copied() {
            map.remove(&any);
        }
    }
    map.insert(key, value);
}

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
                presence: Arc::default(),
                voice: voice::VoiceManager::default(),
            }),
        }
    }

    fn profile() -> Arc<model::Profile> {
        Arc::new(serde_json::from_str(include_str!("fixtures/profile_modal.json")).unwrap())
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
        assert!(cache.failure(CacheRequest::Identities(1), start).is_none());
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
    async fn profile_styles_catalog_reuses_session_cache_for_overlapping_loads() {
        let session = offline_session();
        let fetching = session
            .request_lock(CacheRequest::CollectibleCatalog)
            .lock_owned()
            .await;
        let first = session.collectible_catalog();
        let second = session.collectible_catalog();
        tokio::pin!(first, second);
        assert!(futures::poll!(&mut first).is_pending());
        assert!(futures::poll!(&mut second).is_pending());
        let product = serde_json::from_value(
            serde_json::json!({"sku_id": "1", "type": 1, "items": [{"sku_id": "1", "type": 1}]}),
        )
        .unwrap();
        let catalog = Arc::new(vec![Arc::new(product)]);
        session.cache().collectible_catalog = Some(catalog.clone());
        drop(fetching);
        let (first, second) = tokio::join!(first, second);
        assert!(Arc::ptr_eq(&first.unwrap(), &catalog));
        assert!(Arc::ptr_eq(&second.unwrap(), &catalog));
        assert!(Arc::ptr_eq(
            &session.clone().collectible_catalog().await.unwrap(),
            &catalog
        ));
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
