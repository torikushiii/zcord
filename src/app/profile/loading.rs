use super::*;

impl DiscordApp {
    pub(super) fn profile_slot(&mut self, panel: bool) -> Option<&mut ProfileCard> {
        if panel {
            self.dm_profile.as_mut()
        } else {
            self.profile.as_mut()
        }
    }

    pub(super) fn active_card(&mut self) -> Option<&mut ProfileCard> {
        self.profile.as_mut().or(self.dm_profile.as_mut())
    }

    pub(crate) fn open_profile(
        &mut self,
        user_id: u64,
        position: Point<Pixels>,
        anchor: Anchor,
        cx: &mut Context<Self>,
    ) {
        self.open_profile_at(user_id, position, anchor, ProfileOrigin::Member, cx);
    }

    /// The strip's live bounds keep the popout above it as the window resizes.
    pub(crate) fn open_account_profile(
        &mut self,
        user_id: u64,
        position: Point<Pixels>,
        panel: AccountPanelBounds,
        cx: &mut Context<Self>,
    ) {
        self.open_profile_at(
            user_id,
            position,
            Anchor::BottomLeft,
            ProfileOrigin::Account(panel),
            cx,
        );
    }

    fn open_profile_at(
        &mut self,
        user_id: u64,
        position: Point<Pixels>,
        anchor: Anchor,
        origin: ProfileOrigin,
        cx: &mut Context<Self>,
    ) {
        let guild_id = match &origin {
            ProfileOrigin::Account(_) => None,
            ProfileOrigin::Member => {
                (self.selected_guild_id != 0).then_some(self.selected_guild_id)
            }
        };
        self.drop_styles_editor();
        self.profile = Some(ProfileCard::new(
            user_id, guild_id, position, anchor, origin,
        ));
        self.load_profile_scope(cx);
        cx.notify();
    }

    pub(crate) fn set_profile_menu(&mut self, open: bool, cx: &mut Context<Self>) {
        if let Some(card) = &mut self.profile
            && card.more_menu_open != open
        {
            card.more_menu_open = open;
            cx.notify();
        }
    }

    pub(crate) fn switch_profile_scope(&mut self, cx: &mut Context<Self>) {
        if self.profile_styles.is_some() {
            if let Some(card) = &self.profile
                && let Some(guild_id) = card.source_guild_id
            {
                let scope = if card.guild_id.is_some() {
                    styles::StyleScope::Main
                } else {
                    styles::StyleScope::Guild(guild_id)
                };
                self.set_style_scope(scope, cx);
            }
            return;
        }
        let Some(card) = &mut self.profile else {
            return;
        };
        if card.switch_scope() {
            self.load_profile_scope(cx);
        }
        cx.notify();
    }

    pub(super) fn load_profile_scope(&mut self, cx: &mut Context<Self>) {
        let Some(card) = &self.profile else {
            return;
        };
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        // The full response also covers the popout; avoid fetching both variants on a cold open.
        if let Some(profile) = self
            .session
            .as_ref()
            .filter(|_| self.live)
            .and_then(|session| session.cached_profile(user_id, guild_id, ProfileKind::Popout))
        {
            self.profile_loaded(user_id, guild_id, ProfileKind::Popout, profile, cx);
        }
        self.load_profile(ProfileKind::Modal, cx);
        self.prefetch_profile_details(false, cx);
        self.start_profile_game_timer(cx);
        if let Some(card) = &mut self.profile {
            // Redraw when the wait runs out so the skeleton gives way to whatever has arrived.
            card._tasks.push(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(POPOUT_WAIT).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            }));
        }
    }

    pub(crate) fn open_full_profile_for(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.drop_styles_editor();
        self.profile = Some(ProfileCard::new(
            user_id,
            None,
            point(px(0.), px(0.)),
            Anchor::TopRight,
            ProfileOrigin::Member,
        ));
        self.load_profile_scope(cx);
        self.open_full_profile(cx);
    }

    pub(crate) fn open_full_profile(&mut self, cx: &mut Context<Self>) {
        self.open_full_profile_on(Tab::Board, cx);
    }

    pub(super) fn open_full_profile_on(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.profile.is_none()
            && let Some(user_id) = self.dm_profile.as_ref().map(|card| card.user_id)
        {
            // The DM panel has no popout behind it, so the dialog gets a card of its own.
            self.profile = Some(ProfileCard::new(
                user_id,
                None,
                point(px(0.), px(0.)),
                Anchor::TopRight,
                ProfileOrigin::Member,
            ));
            self.load_profile_scope(cx);
        }
        let Some(card) = &mut self.profile else {
            return;
        };
        card.modal = true;
        card.opened = Instant::now();
        card.effect_started.replace(None);
        // `side_panel` falls back to Activity when there is no Board.
        card.tab = tab;
        self.open_styles_editor(cx);
        self.load_profile(ProfileKind::Modal, cx);
        self.start_profile_game_timer(cx);
        cx.notify();
    }

    pub(super) fn toggle_profile_widget(&mut self, widget_id: u64, cx: &mut Context<Self>) {
        if let Some(card) = self.active_card() {
            if !card.expanded_widgets.remove(&widget_id) {
                card.expanded_widgets.insert(widget_id);
            }
            let generation = card
                .widget_toggles
                .get(&widget_id)
                .map_or(0, |(generation, _)| generation + 1);
            card.widget_toggles
                .insert(widget_id, (generation, Instant::now()));
            cx.notify();
            // Redraw once the animation ends so the rows drop their animation-only clip.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(board::WIDGET_TOGGLE).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }
    }

    pub(super) fn open_profile_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.open_full_profile(cx);
        self.select_profile_tab(tab, cx);
    }

    pub(crate) fn hover_profile_art(
        &mut self,
        slot: ArtSlot,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.active_card() else {
            return;
        };
        let state = &mut card.hovered_art[slot as usize];
        if *state != hovered {
            *state = hovered;
            self.views.overlays.update(cx, |_, cx| cx.notify());
            self.views.members.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn hover_profile_stack(
        &mut self,
        panel: bool,
        stack: Stack,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let card = if panel {
            self.dm_profile.as_mut()
        } else {
            self.profile.as_mut()
        };
        let Some(state) = card.map(|card| &mut card.stack_hover[stack as usize]) else {
            return;
        };
        if state.hovered != hovered {
            state.hovered = hovered;
            state.generation += 1;
            self.views.overlays.update(cx, |_, cx| cx.notify());
            self.views.members.update(cx, |_, cx| cx.notify());
        }
    }

    pub(crate) fn hover_profile_cover(
        &mut self,
        key: (u64, u64),
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.active_card() else {
            return;
        };
        let next = if hovered {
            Some(key)
        } else {
            card.hovered_cover.filter(|current| *current != key)
        };
        if card.hovered_cover != next {
            card.hovered_cover = next;
            self.views.overlays.update(cx, |_, cx| cx.notify());
            self.views.members.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn hover_profile_badge(
        &mut self,
        index: usize,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.active_card() else {
            return;
        };
        let next = if hovered {
            Some(index)
        } else {
            card.hovered_badge.filter(|current| *current != index)
        };
        if card.hovered_badge != next {
            card.hovered_badge = next;
            self.views.overlays.update(cx, |_, cx| cx.notify());
            self.views.members.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn select_profile_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if let Some(card) = &mut self.profile {
            card.tab = tab;
            card.shop_menu_open = false;
            cx.notify();
        }
    }

    pub(crate) fn set_shop_menu(&mut self, open: bool, cx: &mut Context<Self>) {
        if let Some(card) = &mut self.profile
            && card.shop_menu_open != open
        {
            card.shop_menu_open = open;
            cx.notify();
        }
    }

    pub(crate) fn close_profile(&mut self, cx: &mut Context<Self>) -> bool {
        self.drop_styles_editor();
        let open = self.profile.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    fn load_profile(&mut self, kind: ProfileKind, cx: &mut Context<Self>) {
        self.request_profile(kind, false, cx);
    }

    pub(super) fn refresh_style_profile(
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
        // Refresh in place: a style save leaves the board, details and cosmetics intact, and
        // the card keeps showing the saved preview until `profile_loaded` swaps in the response.
        self.request_profile(ProfileKind::Modal, true, cx);
    }

    pub(super) fn request_profile(
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

    pub(super) fn prefetch_profile_details(&mut self, panel: bool, cx: &mut Context<Self>) {
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
        // Start independent resources now, not after the full profile response or a tab switch.
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
        // The activity accessory needs the current game's config even when that app has no
        // Board widget; `details_requested` above keeps this to one request per card.
        if let Some(application_id) = game_ids.first().copied() {
            card.load_widget_configs(session.clone(), vec![application_id], cx);
        }
        card.load_games(session.clone(), game_ids, cx);
        card.load_applications(session.clone(), application_ids, cx);
        card.load_linked_metadata(session, cx);
        self.prefetch_profile_images(cx);
    }

    pub(super) fn prefetch_profile_images(&self, cx: &App) {
        let Some(card) = &self.profile else {
            return;
        };
        let current = self
            .store
            .user(card.user_id)
            .and_then(|user| user.game.as_ref()?.application_id);
        board::prefetch_images(card, current, cx);
    }

    fn profile_loaded(
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
            // Board metadata is independent of the user's activity history.
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

    pub(super) fn load_profile_cosmetics(
        &mut self,
        panel: bool,
        effect_sku: Option<u64>,
        frame_sku: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        let (effect_sku, frame_sku) = if panel {
            (effect_sku, frame_sku)
        } else {
            self.preview_profile()
                .as_deref()
                .map(cosmetic_skus)
                .unwrap_or((effect_sku, frame_sku))
        };
        let available_product = |sku: Option<u64>| {
            let editor = self.profile_styles.as_ref()?;
            Some(editor.find_collectible(sku?)?.product.clone())
        };
        let available_effect = (!panel).then(|| available_product(effect_sku)).flatten();
        let available_frame = (!panel).then(|| available_product(frame_sku)).flatten();
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(card) = self.profile_slot(panel) else {
            return;
        };
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        let matches = |product: &Arc<model::CollectibleProduct>, sku: Option<u64>| {
            sku.is_some_and(|sku| product.items.iter().any(|item| item.sku_id == sku))
        };
        if !card
            .effect
            .as_ref()
            .is_some_and(|product| matches(product, effect_sku))
        {
            card.effect = available_effect;
            card.effect_started.replace(None);
        }
        if !card
            .frame
            .as_ref()
            .is_some_and(|product| matches(product, frame_sku))
        {
            card.frame = available_frame;
        }
        for (sku_id, is_effect) in effect_sku
            .map(|sku| (sku, true))
            .into_iter()
            .chain(frame_sku.map(|sku| (sku, false)))
        {
            let loaded = if is_effect { &card.effect } else { &card.frame };
            if loaded
                .as_ref()
                .is_some_and(|product| matches(product, Some(sku_id)))
            {
                continue;
            }
            if !card.cosmetics_pending.insert(sku_id) {
                continue;
            }
            let session = session.clone();
            let request =
                Tokio::spawn(cx, async move { session.collectible_product(sku_id).await });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let product = match request.await {
                    Ok(Ok(product)) => Some(product),
                    Ok(Err(error)) => {
                        log::warn!(
                            "Could not load profile cosmetic {sku_id} for user {user_id}: {error}"
                        );
                        None
                    }
                    Err(error) => {
                        log::warn!("Profile cosmetic task failed for user {user_id}: {error}");
                        None
                    }
                };
                this.update(cx, |app, cx| {
                    let desired = if panel {
                        app.dm_profile
                            .as_ref()
                            .and_then(|card| card.profile.as_deref())
                            .map(cosmetic_skus)
                    } else {
                        app.preview_profile().as_deref().map(cosmetic_skus)
                    };
                    let Some(card) = app
                        .profile_slot(panel)
                        .filter(|c| c.user_id == user_id && c.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.cosmetics_pending.remove(&sku_id);
                    if desired.is_none_or(
                        |(effect, frame)| if is_effect { effect } else { frame } != Some(sku_id),
                    ) {
                        return;
                    }
                    match product {
                        Some(product) if is_effect => card.effect = Some(product),
                        Some(product) => card.frame = Some(product),
                        None => {}
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
        if !panel && effect_sku.is_some() {
            self.start_style_effect_timer(cx);
        }
    }

    /// Redraws the open profile on a tick, so effect layers can start and loop on time.
    pub(super) fn start_style_effect_timer(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self
            .profile
            .as_mut()
            .filter(|card| !card.effect_timer_started)
        else {
            return;
        };
        card.effect_timer_started = true;
        let user_id = card.user_id;
        card._tasks.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(EFFECT_TICK).await;
                let open = this
                    .update(cx, |app, cx| {
                        let open = app
                            .profile
                            .as_ref()
                            .is_some_and(|card| card.user_id == user_id);
                        if open {
                            app.views.overlays.update(cx, |_, cx| cx.notify());
                        }
                        open
                    })
                    .unwrap_or(false);
                if !open {
                    break;
                }
            }
        }));
    }

    fn start_profile_game_timer(&mut self, cx: &mut Context<Self>) {
        let Some(card) = &mut self.profile else {
            return;
        };
        if card.game_timer_started
            || !self.store.user(card.user_id).is_some_and(|user| {
                [&user.game, &user.listening]
                    .into_iter()
                    .flatten()
                    .any(|activity| activity.started_at.is_some())
            })
        {
            return;
        }
        card.game_timer_started = true;
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        card._tasks.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let open = this
                    .update(cx, |app, cx| {
                        let open = app.profile.as_ref().is_some_and(|card| {
                            card.user_id == user_id && card.guild_id == guild_id
                        });
                        if open {
                            app.views.overlays.update(cx, |_, cx| cx.notify());
                        }
                        open
                    })
                    .unwrap_or(false);
                if !open {
                    break;
                }
            }
        }));
    }

    pub(super) fn load_avatar_theme(&mut self, panel: bool, cx: &mut Context<Self>) {
        let Some(card) = (if panel {
            &mut self.dm_profile
        } else {
            &mut self.profile
        }) else {
            return;
        };
        if !ProfileTheme::needs_avatar(card.profile.as_deref()) {
            card.avatar_theme = None;
            card.avatar_theme_source = None;
            return;
        }
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        let Some(spec) = self
            .store
            .member_avatar_spec(guild_id.unwrap_or(0), user_id)
            .or_else(|| self.store.user(user_id).map(User::avatar_spec))
        else {
            return;
        };
        if card.avatar_theme_source.as_ref() == Some(&spec) {
            return;
        }
        let Some(service) = images::download_service(cx) else {
            return;
        };
        card.avatar_theme = None;
        card.avatar_theme_source = Some(spec.clone());
        let source = spec.clone();
        let request = Tokio::spawn(cx, async move {
            let path = service.load(&source, 128).await?;
            tokio::task::spawn_blocking(move || avatar_colors::load(&path))
                .await
                .ok()
                .flatten()
        });
        card._tasks.push(cx.spawn(async move |this, cx| {
            let colors = request.await.ok().flatten();
            this.update(cx, |app, cx| {
                let Some(card) = app.profile_slot(panel).filter(|card| {
                    card.user_id == user_id
                        && card.guild_id == guild_id
                        && card.avatar_theme_source.as_ref() == Some(&spec)
                }) else {
                    return;
                };
                card.avatar_theme =
                    colors.map(|[primary, secondary]| ProfileTheme { primary, secondary });
                if colors.is_none() {
                    card.avatar_theme_source = None;
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

impl ProfileCard {
    pub(super) fn new(
        user_id: u64,
        guild_id: Option<u64>,
        position: Point<Pixels>,
        anchor: Anchor,
        origin: ProfileOrigin,
    ) -> Self {
        Self {
            user_id,
            panel: false,
            guild_id,
            source_guild_id: guild_id,
            more_menu_open: false,
            hovered_action: None,
            status_menu: popout::status::State::default(),
            scope_snapshots: HashMap::new(),
            origin,
            position,
            anchor,
            modal: false,
            profile: None,
            games: HashMap::new(),
            games_pending: HashSet::new(),
            applications: HashMap::new(),
            applications_pending: HashSet::new(),
            identities: None,
            identities_pending: false,
            outbox: None,
            wishlist: None,
            note: None,
            badge_details: None,
            hovered_badge: None,
            hovered_cover: None,
            hovered_art: [false; 4],
            stack_hover: Default::default(),
            widget_configs: HashMap::new(),
            widget_configs_pending: HashSet::new(),
            board_requested: false,
            modal_request_pending: false,
            details_requested: false,
            wishlist_requested: false,
            note_requested: false,
            shop_menu_open: false,
            game_timer_started: false,
            effect_timer_started: false,
            avatar_theme: None,
            avatar_theme_source: None,
            effect: None,
            frame: None,
            cosmetics_pending: HashSet::new(),
            opened: Instant::now(),
            effect_started: RefCell::new(None),
            tab: Tab::Activity,
            status_prompt: rand::random::<usize>() % STATUS_PROMPTS.len(),
            activity_prompt: rand::random::<usize>() % ACTIVITY_PROMPTS.len(),
            expanded_widgets: HashSet::new(),
            widget_toggles: HashMap::new(),
            commands: Vec::new(),
            commands_requested: false,
            commands_pending: false,
            hovered_command: None,
            _tasks: Vec::new(),
        }
    }

    fn load_commands(&mut self, session: Session, cx: &mut Context<DiscordApp>) {
        let Some(guild_id) = self.source_guild_id.filter(|_| !self.commands_requested) else {
            return;
        };
        let Some(application) = self
            .profile
            .as_ref()
            .and_then(|profile| profile.application.as_ref())
            .filter(|application| !application.popular_application_command_ids.is_empty())
        else {
            return;
        };
        self.commands_requested = true;
        self.commands_pending = true;
        let user_id = self.user_id;
        let application_id = application.id;
        let popular = application.popular_application_command_ids.clone();
        let request = Tokio::spawn(cx, async move {
            session
                .popular_commands(guild_id, application_id, popular)
                .await
        });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let commands = match request.await {
                Ok(Ok(commands)) => commands,
                Ok(Err(error)) => {
                    log::warn!("Could not load commands for app {application_id}: {error}");
                    Vec::new()
                }
                Err(error) => {
                    log::warn!("Commands task failed for app {application_id}: {error}");
                    Vec::new()
                }
            };
            this.update(cx, |app, cx| {
                let Some(card) = app.profile.as_mut().filter(|card| card.user_id == user_id) else {
                    return;
                };
                card.commands_pending = false;
                card.commands = commands;
                cx.notify();
            })
            .ok();
        }));
    }

    pub(super) fn switch_scope(&mut self) -> bool {
        self.more_menu_open = false;
        let Some(source_guild_id) = self.source_guild_id else {
            return false;
        };
        let guild_id = self.guild_id.is_none().then_some(source_guild_id);
        self.switch_to_scope(guild_id)
    }

    pub(super) fn switch_to_scope(&mut self, guild_id: Option<u64>) -> bool {
        self.more_menu_open = false;
        if self.guild_id == guild_id {
            return false;
        }
        if let Some(profile) = &self.profile {
            self.scope_snapshots.insert(self.guild_id, profile.clone());
        }
        if guild_id.is_some() {
            self.source_guild_id = guild_id;
        }
        self.replace_scope(guild_id);
        true
    }

    fn replace_scope(&mut self, guild_id: Option<u64>) {
        let mut next = Self::new(
            self.user_id,
            guild_id,
            self.position,
            self.anchor,
            self.origin.clone(),
        );
        next.source_guild_id = self.source_guild_id;
        next.modal = self.modal;
        next.tab = self.tab;
        next.status_prompt = self.status_prompt;
        next.activity_prompt = self.activity_prompt;
        next.scope_snapshots = std::mem::take(&mut self.scope_snapshots);
        next.profile = next.scope_snapshots.get(&guild_id).cloned().or_else(|| {
            // A guild response includes the main profile; keep it usable offline too.
            self.profile
                .as_ref()
                .filter(|_| guild_id.is_none())
                .map(|profile| scoped_profile(profile.clone(), None))
        });
        // Dropping the old card cancels its callbacks, even after switching back to that scope.
        *self = next;
    }
}

pub(super) fn cosmetic_skus(profile: &model::Profile) -> (Option<u64>, Option<u64>) {
    let effect_of = |p: &model::UserProfile| {
        p.profile_effect
            .as_ref()
            .map(|effect| effect.sku_id)
            .or_else(|| {
                p.collectibles
                    .iter()
                    .find(|item| item.kind == 1)
                    .map(|item| item.sku_id)
            })
    };
    let effect = guild_profile(profile)
        .and_then(effect_of)
        .or_else(|| profile.user_profile.as_ref().and_then(effect_of));
    let frame_of = |p: &model::UserProfile| {
        p.collectibles
            .iter()
            .find(|c| c.kind == 3)
            .map(|c| c.sku_id)
    };
    let frame = guild_profile(profile)
        .and_then(frame_of)
        .or_else(|| profile.user_profile.as_ref().and_then(frame_of));
    (effect, frame)
}

fn scoped_profile(mut profile: Arc<model::Profile>, guild_id: Option<u64>) -> Arc<model::Profile> {
    if guild_id.is_none()
        && (profile.guild_member.is_some() || profile.guild_member_profile.is_some())
    {
        let profile = Arc::make_mut(&mut profile);
        profile.guild_member = None;
        profile.guild_member_profile = None;
    }
    profile
}

pub(super) fn guild_profile(profile: &model::Profile) -> Option<&model::UserProfile> {
    profile.guild_member_profile.as_ref()
}

pub(super) fn widget_game_ids(profile: &model::Profile) -> Vec<u64> {
    let mut ids = Vec::new();
    for widget in &profile.widgets {
        let games = match &widget.data {
            model::WidgetData::FavoriteGames { games }
            | model::WidgetData::CurrentGames { games }
            | model::WidgetData::PlayedGames { games }
            | model::WidgetData::WantToPlayGames { games } => games,
            _ => continue,
        };
        for game in games {
            if !ids.contains(&game.game_id) {
                ids.push(game.game_id);
            }
        }
    }
    ids
}

pub(super) fn widget_application_ids(profile: &model::Profile) -> Vec<u64> {
    let mut ids = Vec::new();
    for widget in &profile.widgets {
        if let model::WidgetData::Application { application_id } = &widget.data
            && !ids.contains(application_id)
        {
            ids.push(*application_id);
        }
    }
    ids
}

pub(super) fn outbox_application_ids(outbox: &model::Outbox) -> Vec<u64> {
    let mut ids = Vec::new();
    for entry in &outbox.entries {
        if let model::OutboxKind::PlayedGame {
            application_id: Some(id),
            ..
        }
        | model::OutboxKind::LaunchedActivity {
            application_id: Some(id),
            ..
        } = &entry.kind
            && !ids.contains(id)
        {
            ids.push(*id);
        }
    }
    ids
}
