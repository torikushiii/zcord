use super::*;

impl DiscordApp {
    pub(crate) fn load_profile_cosmetics(
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
    pub(crate) fn start_style_effect_timer(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn start_profile_game_timer(&mut self, cx: &mut Context<Self>) {
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
    pub(crate) fn load_avatar_theme(&mut self, panel: bool, cx: &mut Context<Self>) {
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
