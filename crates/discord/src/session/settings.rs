use super::super::*;

enum ClientThemeEdit {
    Keep,
    Set(Option<settings_proto::ClientTheme>),
}

impl Session {
    pub fn status_connected(&self) -> bool {
        self.status_generation().is_ok()
    }

    fn status_generation(&self) -> Result<u64, Error> {
        let generation = self.inner.generation.load(Ordering::Acquire);
        if generation.is_multiple_of(2) || *self.inner.control.borrow() != Control::Running {
            return Err(Error::Closed);
        }
        Ok(generation)
    }

    pub fn self_status(&self) -> Option<SelfStatus> {
        self.inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .status()
    }

    pub fn next_status_expiry(&self) -> Option<std::time::SystemTime> {
        self.status_generation().ok()?;
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_status_expiry_at(now)
    }

    pub fn refresh_presence(&self) -> Result<(), Error> {
        let generation = self.status_generation()?;
        let command = {
            let presence = self
                .inner
                .presence
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
            commands::presence(
                presence.payload(now).ok_or(Error::Decode)?,
                presence.revision,
            )
        };
        self.inner
            .commands
            .send((generation, command))
            .map_err(|_| Error::Closed)
    }

    pub async fn set_status_for(
        &self,
        status: SelfStatus,
        duration: StatusDuration,
    ) -> Result<(), Error> {
        let generation = self.status_generation()?;
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let expires_at = if status == SelfStatus::Online {
            None
        } else {
            duration.deadline_ms(now)?
        };
        let proto = self
            .rest(async |rest| rest.fetch_preloaded_settings().await)
            .await?;
        let version = settings_proto::data_version(&proto)
            .map_err(|_| Error::Decode)?
            .ok_or(Error::Decode)?;
        let patch = match expires_at {
            Some(expires_at) => {
                settings_proto::status_patch_at(&proto, status.wire(), expires_at, Some(now))
            }
            None => settings_proto::status_patch(&proto, status.wire()),
        }
        .map_err(|_| Error::Decode)?;
        if self.status_generation()? != generation {
            return Err(Error::Closed);
        }
        let proto = self
            .rest(async |rest| rest.update_status_settings(&patch, version).await)
            .await?;
        if self.status_generation()? != generation {
            return Err(Error::Closed);
        }
        let command = {
            let mut presence = self
                .inner
                .presence
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            presence.install(proto)?;
            if presence.configured_status() != Some(status)
                || presence
                    .timing()
                    .is_none_or(|timing| timing.expires_at_ms != expires_at.unwrap_or(0))
            {
                return Err(Error::SettingsOutdated);
            }
            let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
            commands::presence(
                presence.payload(now).ok_or(Error::Decode)?,
                presence.revision,
            )
        };
        self.inner
            .commands
            .send((generation, command))
            .map_err(|_| Error::Closed)?;
        Ok(())
    }

    pub fn activity_privacy(&self) -> settings_proto::ActivityPrivacy {
        self.inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .proto()
            .and_then(|proto| settings_proto::activity_privacy(proto).ok())
            .unwrap_or_default()
    }

    pub async fn set_activity_privacy(
        &self,
        base: settings_proto::ActivityPrivacy,
        edited: settings_proto::ActivityPrivacy,
    ) -> Result<(), Error> {
        let generation = self.status_generation()?;
        let proto = self
            .rest(async |rest| rest.fetch_preloaded_settings().await)
            .await?;
        let version = settings_proto::data_version(&proto)
            .map_err(|_| Error::Decode)?
            .ok_or(Error::Decode)?;
        let latest = settings_proto::activity_privacy(&proto).map_err(|_| Error::Decode)?;
        let desired = edited.rebase(&base, &latest);
        let sharing_changed = latest.share_activity != desired.share_activity;
        let Some(patch) =
            settings_proto::activity_privacy_patch(&proto, &desired).map_err(|_| Error::Decode)?
        else {
            if self.status_generation()? != generation {
                return Err(Error::Closed);
            }
            let command = {
                let mut presence = self
                    .inner
                    .presence
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let was_sharing = presence
                    .proto()
                    .and_then(|cached| settings_proto::activity_privacy(cached).ok())
                    .unwrap_or_default()
                    .share_activity;
                presence.install(proto)?;
                let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
                (was_sharing != latest.share_activity)
                    .then(|| {
                        presence
                            .payload(now)
                            .map(|payload| (payload, presence.revision))
                    })
                    .flatten()
            };
            if let Some((payload, revision)) = command {
                self.inner
                    .commands
                    .send((generation, commands::presence(payload, revision)))
                    .map_err(|_| Error::Closed)?;
            }
            return Ok(());
        };
        if self.status_generation()? != generation {
            return Err(Error::Closed);
        }
        let proto = self
            .rest(async |rest| rest.update_preloaded_settings(&patch, version).await)
            .await?;
        if self.status_generation()? != generation {
            return Err(Error::Closed);
        }
        let saved = settings_proto::activity_privacy(&proto).map_err(|_| Error::Decode)?;
        let command = {
            let mut presence = self
                .inner
                .presence
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            presence.install(proto)?;
            if saved != desired {
                return Err(Error::SettingsOutdated);
            }
            let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
            sharing_changed
                .then(|| {
                    presence
                        .payload(now)
                        .map(|payload| (payload, presence.revision))
                })
                .flatten()
        };
        if let Some((payload, revision)) = command {
            self.inner
                .commands
                .send((generation, commands::presence(payload, revision)))
                .map_err(|_| Error::Closed)?;
        }
        Ok(())
    }

    pub fn text_images(&self) -> settings_proto::TextImages {
        self.inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .proto()
            .and_then(|proto| settings_proto::text_images(proto).ok())
            .unwrap_or_default()
    }

    pub async fn set_text_images(
        &self,
        base: settings_proto::TextImages,
        edited: settings_proto::TextImages,
    ) -> Result<(), Error> {
        let generation = self.status_generation()?;
        let proto = self
            .rest(async |rest| rest.fetch_preloaded_settings().await)
            .await?;
        let latest = settings_proto::text_images(&proto).map_err(|_| Error::Decode)?;
        let desired = edited.rebase(&base, &latest);
        let patch =
            settings_proto::text_images_patch(&proto, &desired).map_err(|_| Error::Decode)?;
        let saved = self.save_preloaded_patch(generation, proto, patch).await?;
        if settings_proto::text_images(&saved).map_err(|_| Error::Decode)? != desired {
            return Err(Error::SettingsOutdated);
        }
        Ok(())
    }

    pub fn client_theme(&self) -> Option<settings_proto::ClientTheme> {
        self.inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .proto()
            .and_then(|proto| settings_proto::client_theme(proto).ok().flatten())
    }

    pub async fn set_theme(&self, theme: settings_proto::AccountTheme) -> Result<(), Error> {
        self.set_appearance_edit(theme, ClientThemeEdit::Keep).await
    }

    pub async fn set_appearance(
        &self,
        theme: settings_proto::AccountTheme,
        client_theme: Option<settings_proto::ClientTheme>,
    ) -> Result<(), Error> {
        self.set_appearance_edit(theme, ClientThemeEdit::Set(client_theme))
            .await
    }

    async fn set_appearance_edit(
        &self,
        theme: settings_proto::AccountTheme,
        edit: ClientThemeEdit,
    ) -> Result<(), Error> {
        let generation = self.status_generation()?;
        let proto = self
            .rest(async |rest| rest.fetch_preloaded_settings().await)
            .await?;
        let (client_theme, patch) = match edit {
            ClientThemeEdit::Keep => (
                settings_proto::client_theme(&proto).map_err(|_| Error::Decode)?,
                settings_proto::theme_patch(&proto, theme).map_err(|_| Error::Decode)?,
            ),
            ClientThemeEdit::Set(client_theme) => {
                let patch = settings_proto::appearance_patch(&proto, theme, client_theme.as_ref())
                    .map_err(|_| Error::Decode)?;
                (client_theme, patch)
            }
        };
        let saved = self
            .save_preloaded_patch(generation, proto, Some(patch))
            .await?;
        let saved = settings_proto::decode(&saved).map_err(|_| Error::Decode)?;
        if saved.theme != Some(theme) || saved.client_theme != client_theme {
            return Err(Error::SettingsOutdated);
        }
        Ok(())
    }

    pub fn favorites(&self) -> settings_proto::Favorites {
        self.inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .proto()
            .and_then(|proto| settings_proto::favorites(proto).ok())
            .unwrap_or_default()
    }

    pub async fn set_favorites_visible(&self, visible: bool) -> Result<(), Error> {
        let generation = self.status_generation()?;
        let proto = self
            .rest(async |rest| rest.fetch_preloaded_settings().await)
            .await?;
        let patch =
            settings_proto::favorites_visible_patch(&proto, visible).map_err(|_| Error::Decode)?;
        let saved = self.save_preloaded_patch(generation, proto, patch).await?;
        if settings_proto::favorites(&saved)
            .map_err(|_| Error::Decode)?
            .guild_visible
            != visible
        {
            return Err(Error::SettingsOutdated);
        }
        Ok(())
    }

    pub async fn set_favorite_channel(
        &self,
        channel_id: u64,
        channel: Option<settings_proto::FavoriteChannel>,
    ) -> Result<(), Error> {
        let generation = self.status_generation()?;
        let proto = self
            .rest(async |rest| rest.fetch_preloaded_settings().await)
            .await?;
        let patch = settings_proto::favorite_channel_patch(&proto, channel_id, channel.clone())
            .map_err(|_| Error::Decode)?;
        let saved = self.save_preloaded_patch(generation, proto, patch).await?;
        let saved = settings_proto::favorites(&saved).map_err(|_| Error::Decode)?;
        if saved.channels.iter().find(|c| c.id == channel_id) != channel.as_ref() {
            return Err(Error::SettingsOutdated);
        }
        Ok(())
    }

    async fn save_preloaded_patch(
        &self,
        generation: u64,
        proto: Vec<u8>,
        patch: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, Error> {
        let version = settings_proto::data_version(&proto)
            .map_err(|_| Error::Decode)?
            .ok_or(Error::Decode)?;
        if self.status_generation()? != generation {
            return Err(Error::Closed);
        }
        let proto = match patch {
            Some(patch) => {
                self.rest(async |rest| rest.update_preloaded_settings(&patch, version).await)
                    .await?
            }
            None => proto,
        };
        if self.status_generation()? != generation {
            return Err(Error::Closed);
        }
        self.inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .install(proto.clone())?;
        Ok(proto)
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

    pub async fn content_inbox(&self) -> Result<model::ContentInbox, Error> {
        self.rest(async |rest| rest.fetch_content_inbox().await)
            .await
    }

    pub async fn update_frecency_settings(&self, proto: &[u8]) -> Result<Vec<u8>, Error> {
        self.rest(async |rest| rest.update_frecency_settings(proto).await)
            .await
    }
}

#[cfg(test)]
mod tests;
