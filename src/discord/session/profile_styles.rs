use super::super::*;

impl Session {
    pub async fn collectible_catalog(
        &self,
    ) -> Result<Arc<Vec<Arc<model::CollectibleProduct>>>, Error> {
        let _request = self
            .request_lock(CacheRequest::CollectibleCatalog)
            .lock_owned()
            .await;
        if let Some(catalog) = self.cache().collectible_catalog.clone() {
            return Ok(catalog);
        }
        if let Some(error) = self
            .cache()
            .failure(CacheRequest::CollectibleCatalog, Instant::now())
        {
            return Err(error);
        }
        let products = match self
            .rest(async |rest| rest.fetch_collectible_catalog().await)
            .await
        {
            Ok(products) => products,
            Err(error) => {
                self.cache().remember_failure(
                    CacheRequest::CollectibleCatalog,
                    &error,
                    Instant::now(),
                );
                return Err(error);
            }
        };
        let catalog = Arc::new(products.into_iter().map(Arc::new).collect::<Vec<_>>());
        self.cache().collectible_catalog = Some(catalog.clone());
        Ok(catalog)
    }

    pub async fn owned_collectibles(&self) -> Result<Vec<model::CollectiblePurchase>, Error> {
        self.rest(async |rest| rest.fetch_owned_collectibles().await)
            .await
    }

    pub async fn recent_avatars(&self) -> Result<Vec<model::RecentAvatar>, Error> {
        self.rest(async |rest| rest.fetch_recent_avatars().await)
            .await
    }

    pub async fn recent_avatar_data_uri(&self, user_id: u64, hash: &str) -> Result<String, Error> {
        self.rest(async |rest| rest.recent_avatar_data_uri(user_id, hash).await)
            .await
    }

    pub async fn save_widgets(
        &self,
        user_id: u64,
        widgets: &[model::ProfileWidget],
    ) -> Result<Vec<model::ProfileWidget>, Error> {
        let result = self
            .rest(async |rest| rest.put_widgets(widgets).await)
            .await;
        // Every cached scope of this profile carries the old list.
        let mut cache = self.cache();
        cache.profile_revision = cache.profile_revision.wrapping_add(1);
        cache.profiles.retain(|(id, _), _| *id != user_id);
        result
    }

    pub async fn save_profile_styles(
        &self,
        user_id: u64,
        guild_id: Option<u64>,
        patch: &model::ProfileStylePatch,
    ) -> Result<(), Error> {
        let result = self
            .rest(async |rest| rest.patch_profile_styles(guild_id, patch).await)
            .await;
        // Either PATCH can have succeeded even if the other failed. A server tag is
        // account-wide, so saving one invalidates every scope, as a main profile save does.
        let only_guild = guild_id.filter(|_| patch.tag_guild.is_none());
        let mut cache = self.cache();
        cache.profile_revision = cache.profile_revision.wrapping_add(1);
        cache.profiles.retain(|(id, scope), _| {
            *id != user_id || only_guild.is_some_and(|guild| *scope != Some(guild))
        });
        cache.failures.retain(|key, _| {
            !matches!(key, CacheRequest::Profile(id, scope)
            if *id == user_id && (only_guild.is_none() || *scope == only_guild))
        });
        result
    }
}
