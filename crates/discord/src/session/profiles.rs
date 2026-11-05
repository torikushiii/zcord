use super::super::*;

impl Session {
    pub async fn profile(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
        kind: ProfileKind,
    ) -> Result<Arc<model::Profile>, Error> {
        self.profile_request(user_id, guild_id, channel_id, kind, false)
            .await
    }

    pub async fn refresh_profile(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
        kind: ProfileKind,
    ) -> Result<Arc<model::Profile>, Error> {
        self.profile_request(user_id, guild_id, channel_id, kind, true)
            .await
    }

    async fn profile_request(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        channel_id: Option<u64>,
        kind: ProfileKind,
        refresh: bool,
    ) -> Result<Arc<model::Profile>, Error> {
        let _request = self
            .request_lock(CacheRequest::Profile(user_id, guild_id))
            .lock_owned()
            .await;
        let key = (user_id, guild_id);
        if refresh {
            let mut cache = self.cache();
            cache.profiles.remove(&key);
            cache
                .failures
                .remove(&CacheRequest::Profile(user_id, guild_id));
        }
        if let Some(profile) = self.cached_profile(user_id, guild_id, kind) {
            return Ok(profile);
        }
        if let Some(error) = self
            .cache()
            .failure(CacheRequest::Profile(user_id, guild_id), Instant::now())
        {
            return Err(error);
        }
        let revision = self.cache().profile_revision;
        let profile = match self
            .rest(async |rest| {
                rest.fetch_profile(user_id, guild_id, channel_id, kind)
                    .await
            })
            .await
        {
            Ok(profile) => profile,
            Err(error) => {
                let mut cache = self.cache();
                if cache.profile_revision == revision {
                    cache.remember_failure(
                        CacheRequest::Profile(user_id, guild_id),
                        &error,
                        Instant::now(),
                    );
                }
                return Err(error);
            }
        };
        let profile = Arc::new(profile);
        let mut cache = self.cache();
        if cache.profile_revision == revision {
            cache.insert_profile(key, kind, profile.clone(), Instant::now());
        }
        Ok(profile)
    }

    pub fn cached_profile(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        kind: ProfileKind,
    ) -> Option<Arc<model::Profile>> {
        self.cache()
            .profile((user_id, guild_id), kind, Instant::now())
    }

    pub fn cached_profile_details(
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
                self.cache().remember_failure(
                    CacheRequest::Product(sku_id),
                    &error,
                    Instant::now(),
                );
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
                self.cache().remember_failure(
                    CacheRequest::Outbox(user_id),
                    &error,
                    Instant::now(),
                );
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

    pub async fn user_affinities(&self) -> Result<Vec<model::UserAffinity>, Error> {
        self.rest(async |rest| rest.fetch_user_affinities().await)
            .await
    }

    pub async fn note(&self, user_id: u64) -> Result<Option<String>, Error> {
        self.rest(async |rest| rest.fetch_note(user_id).await).await
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
                self.cache().remember_failure(
                    CacheRequest::Badges(user_id),
                    &error,
                    Instant::now(),
                );
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
}
