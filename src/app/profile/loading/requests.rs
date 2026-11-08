use super::*;

impl DiscordApp {
    pub(super) fn load_profile(&mut self, kind: ProfileKind, cx: &mut Context<Self>) {
        self.request_profile(kind, false, cx);
    }
    pub(crate) fn refresh_style_profile(
        &mut self,
        saved_scope: styles::StyleScope,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = &mut self.profile else {
            return;
        };
        if saved_scope == styles::StyleScope::Main {
            card.scope_snapshots.clear();
        } else {
            card.scope_snapshots.remove(&saved_scope.guild_id());
        }
        // A profile fetched before a running widget save lands would bring back the old widgets.
        if let Some(widgets) = self
            .profile_styles
            .as_mut()
            .map(|editor| &mut editor.ui.widgets)
            .filter(|widgets| widgets.saving)
        {
            widgets.refresh_after_save = true;
            return;
        }
        self.request_profile(ProfileKind::Modal, true, cx);
    }
    pub(crate) fn request_profile(
        &mut self,
        kind: ProfileKind,
        refresh: bool,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(card) = &mut self.profile else {
            return;
        };
        if kind == ProfileKind::Modal {
            if card.modal_request_pending || (!refresh && card.board_requested) {
                return;
            }
            card.modal_request_pending = true;
        }
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        if !refresh && let Some(profile) = session.cached_profile(user_id, guild_id, kind) {
            self.profile_loaded(user_id, guild_id, kind, profile, cx);
            return;
        }
        let request = Tokio::spawn(cx, async move {
            if refresh {
                session
                    .refresh_profile(user_id, guild_id, Some(channel_id), kind)
                    .await
            } else {
                session
                    .profile(user_id, guild_id, Some(channel_id), kind)
                    .await
            }
        });
        card._tasks.push(cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                if let Some(card) = app.profile.as_mut().filter(|card| {
                    card.user_id == user_id && card.guild_id == guild_id
                }) && kind == ProfileKind::Modal {
                    card.modal_request_pending = false;
                    cx.notify();
                }
                match result {
                    Ok(Ok(profile)) => app.profile_loaded(user_id, guild_id, kind, profile, cx),
                    Ok(Err(error)) => log::warn!(
                        "Could not load {kind:?} profile for user {user_id} in guild {guild_id:?}: {error}"
                    ),
                    Err(error) => log::warn!(
                        "Profile task failed for user {user_id}: {error}"
                    ),
                }
            })
            .ok();
        }));
    }
    pub(crate) fn prefetch_profile_details(&mut self, panel: bool, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(card) = (if panel {
            &mut self.dm_profile
        } else {
            &mut self.profile
        }) else {
            return;
        };
        if card.details_requested {
            return;
        }
        card.details_requested = true;
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        let game_ids: Vec<_> = self
            .store
            .user(user_id)
            .and_then(|user| user.game.as_ref()?.application_id)
            .into_iter()
            .collect();
        let cached = session.cached_profile_details(user_id, &game_ids, &game_ids);
        card.games.extend(cached.games);
        card.applications.extend(cached.applications);
        card.widget_configs.extend(cached.widget_configs);
        card.identities = cached.identities;
        card.outbox = cached.outbox;
        card.badge_details = cached.badges;
        let mut application_ids = game_ids.clone();
        if let Some(outbox) = &card.outbox {
            for id in outbox_application_ids(outbox) {
                if !application_ids.contains(&id) {
                    application_ids.push(id);
                }
            }
        }
        if card.identities.is_none() {
            card.identities_pending = true;
            let session = session.clone();
            let request = Tokio::spawn(
                cx,
                async move { session.application_identities(user_id).await },
            );
            card._tasks.push(cx.spawn(async move |this, cx| {
                let identities = match request.await {
                    Ok(Ok(identities)) => Some(identities),
                    Ok(Err(error)) => {
                        log::warn!(
                            "Could not load application identities for user {user_id}: {error}"
                        );
                        None
                    }
                    Err(error) => {
                        log::warn!(
                            "Application identities task failed for user {user_id}: {error}"
                        );
                        None
                    }
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile_slot(panel)
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.identities_pending = false;
                    if identities.is_some() {
                        card.identities = identities;
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
        if card.badge_details.is_none() {
            let session = session.clone();
            let request = Tokio::spawn(cx, async move {
                session
                    .user_badges(user_id, guild_id, Some(channel_id))
                    .await
            });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let badges = match request.await {
                    Ok(Ok(badges)) => badges,
                    Ok(Err(error)) => {
                        log::warn!("Could not load user badges for user {user_id}: {error}");
                        return;
                    }
                    Err(error) => {
                        log::warn!("User badges task failed for user {user_id}: {error}");
                        return;
                    }
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile_slot(panel)
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.badge_details = Some(badges);
                    cx.notify();
                })
                .ok();
            }));
        }
        if card.outbox.is_none() {
            let outbox_session = session.clone();
            let application_session = session.clone();
            let request = Tokio::spawn(cx, async move { outbox_session.outbox(user_id).await });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let outbox = match request.await {
                    Ok(Ok(outbox)) => outbox,
                    Ok(Err(error)) => {
                        log::warn!("Could not load profile outbox for user {user_id}: {error}");
                        return;
                    }
                    Err(error) => {
                        log::warn!("Profile outbox task failed for user {user_id}: {error}");
                        return;
                    }
                };
                let application_ids = outbox_application_ids(&outbox);
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile_slot(panel)
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.outbox = Some(outbox);
                    card.load_applications(application_session, application_ids, cx);
                    app.prefetch_profile_images(cx);
                    cx.notify();
                })
                .ok();
            }));
        }
        if card.note.is_none() && !card.note_requested && user_id != self.store.current_user_id {
            card.note_requested = true;
            let note_session = session.clone();
            let request = Tokio::spawn(cx, async move { note_session.note(user_id).await });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let note = match request.await {
                    Ok(Ok(note)) => note,
                    Ok(Err(error)) => {
                        log::warn!("Could not load profile note for user {user_id}: {error}");
                        return;
                    }
                    Err(error) => {
                        log::warn!("Profile note task failed for user {user_id}: {error}");
                        return;
                    }
                };
                let Some(note) = note else {
                    return;
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile_slot(panel)
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.note = Some(note);
                    cx.notify();
                })
                .ok();
            }));
        }
        if let Some(application_id) = game_ids.first().copied() {
            card.load_widget_configs(session.clone(), vec![application_id], cx);
        }
        card.load_games(session.clone(), game_ids, cx);
        card.load_applications(session.clone(), application_ids, cx);
        card.load_linked_metadata(session, cx);
        self.prefetch_profile_images(cx);
    }
    pub(crate) fn prefetch_profile_images(&self, cx: &App) {
        let Some(card) = &self.profile else {
            return;
        };
        let current = self
            .store
            .user(card.user_id)
            .and_then(|user| user.game.as_ref()?.application_id);
        board::prefetch_images(card, current, cx);
    }
    pub(super) fn profile_loaded(
        &mut self,
        user_id: u64,
        guild_id: Option<u64>,
        kind: ProfileKind,
        profile: Arc<model::Profile>,
        cx: &mut Context<Self>,
    ) {
        if !self
            .profile
            .as_ref()
            .is_some_and(|card| card.user_id == user_id && card.guild_id == guild_id)
        {
            return;
        }
        let profile = scoped_profile(profile, guild_id);
        self.store.absorb_profile(guild_id, &profile);
        let game = self.store.user(user_id).and_then(|user| user.game.clone());
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(card) = self
            .profile
            .as_mut()
            .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
        else {
            return;
        };
        // A late popout response lacks the modal's widgets and must not start Board loading.
        if (card.modal || card.board_requested) && kind == ProfileKind::Popout {
            return;
        }
        if kind == ProfileKind::Modal {
            card.modal_request_pending = false;
        }
        let (effect, frame) = cosmetic_skus(&profile);
        let load_board = kind == ProfileKind::Modal && !card.board_requested;
        let popout = kind == ProfileKind::Popout;
        let mut game_ids = if load_board || popout {
            widget_game_ids(&profile)
        } else {
            Vec::new()
        };
        let widget_application_ids = if load_board || popout {
            widget_application_ids(&profile)
        } else {
            Vec::new()
        };
        let mut application_ids = widget_application_ids.clone();
        if load_board
            && let Some(application_id) = game.as_ref().and_then(|game| game.application_id)
        {
            if !game_ids.contains(&application_id) {
                game_ids.push(application_id);
            }
            if !application_ids.contains(&application_id) {
                application_ids.push(application_id);
            }
        }
        if load_board {
            let cached = session.cached_profile_details(user_id, &game_ids, &application_ids);
            card.games.extend(cached.games);
            card.applications.extend(cached.applications);
            card.identities = cached.identities;
            card.outbox = cached.outbox;
            card.widget_configs.extend(cached.widget_configs);
            if let Some(outbox) = &card.outbox {
                for id in outbox_application_ids(outbox) {
                    if !application_ids.contains(&id) {
                        application_ids.push(id);
                    }
                }
            }
        }
        card.profile = Some(profile);
        if card.wishlist.is_none()
            && !card.wishlist_requested
            && let Some(wishlist_id) = card
                .profile
                .as_ref()
                .and_then(|profile| profile.wishlist_ids.first().copied())
        {
            card.wishlist_requested = true;
            let wishlist_session = session.clone();
            let request = Tokio::spawn(
                cx,
                async move { wishlist_session.wishlist(wishlist_id).await },
            );
            card._tasks.push(cx.spawn(async move |this, cx| {
                let wishlist = match request.await {
                    Ok(Ok(wishlist)) => wishlist,
                    Ok(Err(error)) => {
                        log::warn!(
                            "Could not load wishlist {wishlist_id} for user {user_id}: {error}"
                        );
                        return;
                    }
                    Err(error) => {
                        log::warn!("Wishlist task failed for user {user_id}: {error}");
                        return;
                    }
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile
                        .as_mut()
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.wishlist = Some(wishlist);
                    cx.notify();
                })
                .ok();
            }));
        }
        if load_board {
            card.board_requested = true;
            card.load_applications(session.clone(), application_ids, cx);
            card.load_games(session.clone(), game_ids, cx);
            card.load_widget_configs(session.clone(), widget_application_ids, cx);
        } else if popout {
            card.load_applications(session.clone(), widget_application_ids.clone(), cx);
            card.load_widget_configs(session.clone(), widget_application_ids, cx);
            card.load_games(session.clone(), game_ids, cx);
        }
        card.load_commands(session.clone(), cx);
        card.load_linked_metadata(session, cx);
        self.styles_profile_loaded();
        self.prefetch_profile_images(cx);
        self.start_profile_game_timer(cx);
        self.load_profile_cosmetics(false, effect, frame, cx);
        self.load_avatar_theme(false, cx);
        cx.notify();
    }
}
