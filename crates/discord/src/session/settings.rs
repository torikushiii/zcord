use super::super::*;

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

    /// The cached absolute manual deadline, only while READY and still in the future.
    pub fn next_status_expiry(&self) -> Option<std::time::SystemTime> {
        self.status_generation().ok()?;
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next_status_expiry_at(now)
    }

    /// Reproject current account settings after a local status expiry.
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

    /// Persist the account setting with CAS, then queue this socket's presence write.
    /// Not retried after a reconnect; the next READY reflects whatever was saved.
    /// A finite manual choice persists its absolute deadline and creation time. Online
    /// and Forever clear old manual timing without changing the custom-status deadline.
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
        // The setting is saved; the gateway's presence throttle (one write per 5s) only
        // delays when other clients see it, so don't hold the caller for the socket write.
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

    /// Not retried after a reconnect.
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
            // Already saved elsewhere, possibly ahead of its gateway update: the cache still
            // has to take the fetched settings, or the page falls back to the old value.
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

    /// Not retried after a reconnect; the next READY reflects whatever was saved.
    pub async fn set_theme(&self, theme: settings_proto::AccountTheme) -> Result<(), Error> {
        let generation = self.status_generation()?;
        let proto = self
            .rest(async |rest| rest.fetch_preloaded_settings().await)
            .await?;
        let version = settings_proto::data_version(&proto)
            .map_err(|_| Error::Decode)?
            .ok_or(Error::Decode)?;
        let patch = settings_proto::theme_patch(&proto, theme).map_err(|_| Error::Decode)?;
        if self.status_generation()? != generation {
            return Err(Error::Closed);
        }
        let proto = self
            .rest(async |rest| rest.update_preloaded_settings(&patch, version).await)
            .await?;
        if self.status_generation()? != generation {
            return Err(Error::Closed);
        }
        let installed_theme = settings_proto::decode(&proto)
            .map_err(|_| Error::Decode)?
            .theme;
        self.inner
            .presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .install(proto)?;
        if installed_theme != Some(theme) {
            return Err(Error::SettingsOutdated);
        }
        Ok(())
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
mod tests {
    use super::*;

    fn offline_session(
        generation: u64,
    ) -> (
        Session,
        tokio::sync::mpsc::UnboundedReceiver<(u64, commands::Command)>,
    ) {
        let (control, _) = watch::channel(Control::Running);
        let (commands, receiver) = tokio::sync::mpsc::unbounded_channel();
        let session = Session {
            inner: Arc::new(Inner {
                rest: Err(Error::Network),
                gif_client: Err(Error::Network),
                cache: Mutex::default(),
                requests: Mutex::default(),
                sends: Mutex::default(),
                control,
                commands,
                generation: Arc::new(AtomicU64::new(generation)),
                presence: Arc::default(),
                voice: voice::VoiceManager::default(),
            }),
        };
        (session, receiver)
    }

    #[test]
    fn manual_expiry_deadline_survives_reconnect_and_refreshes_online_without_rest() {
        use crate::settings_fixtures::status_fixture;
        use base64::{Engine, engine::general_purpose::STANDARD};
        use std::time::{Duration, UNIX_EPOCH};

        let (session, mut receiver) = offline_session(0);
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let deadline = now + 86_400_000;
        let base = STANDARD
            .decode(status_fixture(
                Some("idle"),
                Some(("keep custom", 42, "party", 0)),
            ))
            .unwrap();
        let patch = settings_proto::status_patch_at(&base, "idle", deadline, Some(now)).unwrap();
        session
            .inner
            .presence
            .lock()
            .unwrap()
            .install(settings_proto::merge_partial(&base, &patch).unwrap())
            .unwrap();
        assert_eq!(session.next_status_expiry(), None, "not READY yet");
        session.inner.generation.store(1, Ordering::Release);
        let expected = Some(UNIX_EPOCH + Duration::from_millis(deadline));
        assert_eq!(session.next_status_expiry(), expected);
        assert_eq!(session.self_status(), Some(SelfStatus::Idle));
        session.inner.generation.store(2, Ordering::Release);
        assert_eq!(session.next_status_expiry(), None);
        session.inner.generation.store(3, Ordering::Release);
        assert_eq!(
            session.next_status_expiry(),
            expected,
            "reconnect does not reset the clock"
        );

        let expired = settings_proto::status_patch_at(&base, "dnd", 1, Some(0)).unwrap();
        session
            .inner
            .presence
            .lock()
            .unwrap()
            .install(settings_proto::merge_partial(&base, &expired).unwrap())
            .unwrap();
        assert_eq!(session.next_status_expiry(), None);
        assert_eq!(session.self_status(), Some(SelfStatus::Online));
        session.refresh_presence().unwrap();
        let (generation, command) = receiver.try_recv().unwrap();
        assert_eq!(generation, 3);
        assert_eq!(command.payload["d"]["status"], "online");
        assert_eq!(
            command.payload["d"]["activities"][0]["state"],
            "keep custom"
        );
        let presence = session.inner.presence.lock().unwrap();
        assert_eq!(presence.configured_status(), Some(SelfStatus::DoNotDisturb));
        assert_eq!(
            presence.timing().unwrap().expires_at_ms,
            1,
            "expiry is projected, not persisted again"
        );
        assert!(session.inner.rest.is_err());
    }

    #[test]
    fn expiry_refresh_withdraws_custom_activity_using_current_status_without_rest() {
        use crate::settings_fixtures::status_fixture;
        use base64::{Engine, engine::general_purpose::STANDARD};

        let (session, mut receiver) = offline_session(1);
        let install = |status, custom| {
            session
                .inner
                .presence
                .lock()
                .unwrap()
                .install(
                    STANDARD
                        .decode(status_fixture(Some(status), Some(custom)))
                        .unwrap(),
                )
                .unwrap();
        };
        install("online", ("expiring", 42, "party", u64::MAX));
        session.refresh_presence().unwrap();
        let (_, initial) = receiver.try_recv().unwrap();
        assert_eq!(
            initial.payload["d"]["activities"].as_array().unwrap().len(),
            1
        );
        initial.complete(Ok(()));

        // A later choice must not be rolled back to the timer's original Online status.
        install("invisible", ("expiring", 42, "party", 1));
        session.refresh_presence().unwrap();
        let (generation, expired) = receiver.try_recv().unwrap();
        assert_eq!(generation, 1);
        assert_eq!(expired.payload["op"], 3);
        assert_eq!(expired.payload["d"]["status"], "invisible");
        assert_eq!(expired.payload["d"]["activities"], serde_json::json!([]));
        assert_eq!(session.self_status(), Some(SelfStatus::Invisible));

        // A replaced custom status is projected intact, never cleared by a captured expiry.
        install("dnd", ("replacement", 0, "", u64::MAX));
        session.refresh_presence().unwrap();
        let (_, replacement) = receiver.try_recv().unwrap();
        assert_eq!(replacement.payload["d"]["status"], "dnd");
        assert_eq!(
            replacement.payload["d"]["activities"][0]["state"],
            "replacement"
        );
        session.inner.generation.store(2, Ordering::Release);
        assert!(matches!(session.refresh_presence(), Err(Error::Closed)));
        assert!(receiver.try_recv().is_err());
        session.inner.generation.store(3, Ordering::Release);
        session.refresh_presence().unwrap();
        assert_eq!(receiver.try_recv().unwrap().0, 3);
        // REST is deliberately unavailable in this fixture; all refreshes above are local.
        assert!(session.inner.rest.is_err());
    }

    #[tokio::test]
    async fn status_writes_fail_before_rest_when_not_ready_or_shutdown() {
        let (session, mut receiver) = offline_session(0);
        assert!(!session.status_connected());
        assert!(matches!(
            session
                .set_status_for(SelfStatus::Invisible, StatusDuration::Forever)
                .await,
            Err(Error::Closed)
        ));
        assert!(matches!(
            session
                .set_status_for(SelfStatus::Idle, StatusDuration::Minutes15)
                .await,
            Err(Error::Closed)
        ));
        assert!(matches!(
            session.set_theme(settings_proto::AccountTheme::Light).await,
            Err(Error::Closed)
        ));
        session.inner.generation.store(1, Ordering::Release);
        assert!(session.status_connected());
        assert!(matches!(
            session
                .set_status_for(SelfStatus::Idle, StatusDuration::Forever)
                .await,
            Err(Error::Network)
        ));
        assert!(matches!(
            session
                .set_status_for(SelfStatus::Invisible, StatusDuration::Days3)
                .await,
            Err(Error::Network)
        ));
        assert!(matches!(
            session.set_theme(settings_proto::AccountTheme::Dark).await,
            Err(Error::Network)
        ));
        session.shutdown();
        assert!(!session.status_connected());
        assert!(matches!(
            session
                .set_status_for(SelfStatus::Online, StatusDuration::Forever)
                .await,
            Err(Error::Closed)
        ));
        assert!(matches!(
            session.set_theme(settings_proto::AccountTheme::Dark).await,
            Err(Error::Closed)
        ));
        assert!(receiver.try_recv().is_err());
    }
}
