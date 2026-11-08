use super::values::{
    guild_values, main_values, name_style, normalize_draft, style_error, style_patch,
};
use super::*;

impl DiscordApp {
    pub(crate) fn load_style_shop(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self
            .profile_styles
            .as_mut()
            .filter(|editor| editor.shop.is_none() && !editor.shop_requested)
        else {
            return;
        };
        editor.shop_requested = true;
        let identity = editor.identity.clone();
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            editor.error = Some("Connect to Discord to load the shop.".into());
            cx.notify();
            return;
        };
        let request = Tokio::spawn(cx, async move { session.collectible_catalog().await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let Some(editor) = app
                    .profile_styles
                    .as_mut()
                    .filter(|editor| Arc::ptr_eq(&editor.identity, &identity))
                else {
                    return;
                };
                match result {
                    Ok(Ok(catalog)) => {
                        editor.shop_catalog = Some(Arc::new(
                            catalog
                                .iter()
                                .cloned()
                                .filter_map(OwnedCollectible::from_shop_product)
                                .collect(),
                        ));
                        editor.refresh_shop();
                    }
                    Ok(Err(error)) => editor.error = Some(style_error(&error)),
                    Err(error) => {
                        log::warn!("Collectible catalog task failed: {error}");
                        editor.error = Some("Could not load the collectibles shop.".into());
                    }
                }
                app.refresh_style_cosmetics(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn open_styles_editor(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self
            .profile
            .as_ref()
            .filter(|card| card.modal && card.user_id == self.store.current_user_id)
        else {
            self.drop_styles_editor();
            return;
        };
        if self.profile_styles.is_some() {
            return;
        }
        let scope = card.guild_id.map_or(StyleScope::Main, StyleScope::Guild);
        let nitro = card.profile.as_ref().map_or_else(
            || self.store.has_nitro(),
            |profile| profile.premium_type.is_some_and(|premium| premium != 0),
        );
        let mut editor = StylesEditor::new(scope, nitro);
        editor.saved_profile = card.profile.clone();
        let identity = editor.identity.clone();
        self.profile_styles = Some(editor);
        // No earlier editor's draft survives, so its previews can all go.
        self.prune_style_previews(false);
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            if let Some(editor) = &mut self.profile_styles {
                editor.error = Some("Connect to Discord to load your styles.".into());
            }
            return;
        };
        let avatars_session = session.clone();
        let purchases = Tokio::spawn(cx, async move { session.owned_collectibles().await });
        let avatars = Tokio::spawn(cx, async move { avatars_session.recent_avatars().await });
        let purchases_identity = identity.clone();
        cx.spawn(async move |this, cx| {
            let result = purchases.await;
            this.update(cx, |app, cx| {
                let Some(editor) = app
                    .profile_styles
                    .as_mut()
                    .filter(|editor| Arc::ptr_eq(&editor.identity, &purchases_identity))
                else {
                    return;
                };
                match result {
                    Ok(Ok(purchases)) => {
                        editor.owned = Some(Arc::new(
                            purchases
                                .into_iter()
                                .filter_map(|purchase| {
                                    let kind = CollectibleKind::from_type(purchase.product.kind)?;
                                    Some(OwnedCollectible {
                                        kind,
                                        sku_id: purchase.product.sku_id,
                                        product: Arc::new(purchase.product),
                                        purchased_at: purchase.purchased_at.map(Into::into),
                                    })
                                })
                                .collect(),
                        ))
                    }
                    Ok(Err(error)) => editor.error = Some(style_error(&error)),
                    Err(error) => {
                        log::warn!("Owned collectibles task failed: {error}");
                        editor.error = Some("Could not load your collectibles.".into());
                    }
                }
                editor.refresh_shop();
                app.refresh_style_cosmetics(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = avatars.await;
            this.update(cx, |app, cx| {
                let Some(editor) = app
                    .profile_styles
                    .as_mut()
                    .filter(|editor| Arc::ptr_eq(&editor.identity, &identity))
                else {
                    return;
                };
                match result {
                    Ok(Ok(avatars)) => {
                        editor.recent_avatars = Some(Arc::new(
                            avatars
                                .into_iter()
                                .map(|avatar| RecentAvatar {
                                    id: avatar.id,
                                    storage_hash: avatar.storage_hash.into(),
                                    description: avatar.description.into(),
                                })
                                .collect(),
                        ))
                    }
                    Ok(Err(error)) => editor.error = Some(style_error(&error)),
                    Err(error) => {
                        log::warn!("Recent avatars task failed: {error}");
                        editor.error = Some("Could not load your recent avatars.".into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn styles_profile_loaded(&mut self) {
        self.capture_saved_style_profile();
        let saved = self.saved_style_values();
        if let Some(editor) = &mut self.profile_styles {
            if let Some(profile) = self.profile.as_ref().and_then(|card| card.profile.as_ref()) {
                editor.nitro = profile.premium_type.is_some_and(|premium| premium != 0);
            }
            if !editor.saving {
                normalize_draft(&mut editor.draft, &saved);
            }
        }
        self.rebuild_style_preview();
    }

    pub(super) fn capture_saved_style_profile(&mut self) {
        if let Some(editor) = &mut self.profile_styles
            && let Some(card) = &self.profile
            && card.user_id == self.store.current_user_id
            && card.guild_id == editor.scope.guild_id()
        {
            editor.saved_profile = card.profile.clone();
        }
    }

    pub(super) fn restore_style_profile(&mut self) {
        if let Some(editor) = &self.profile_styles
            && let Some(card) = &mut self.profile
            && card.user_id == self.store.current_user_id
            && card.guild_id == editor.scope.guild_id()
        {
            card.profile = editor.saved_profile.clone();
        }
    }

    pub(crate) fn drop_styles_editor(&mut self) {
        self.restore_style_profile();
        if self.profile_styles.take().is_some() {
            self.prune_style_previews(false);
        }
    }

    pub(crate) fn rebuild_style_preview(&mut self) {
        if let Some(editor) = &self.profile_styles
            && let Some(card) = &mut self.profile
            && card.user_id == self.store.current_user_id
            && card.guild_id == editor.scope.guild_id()
        {
            card.profile = editor.preview_profile();
        }
    }

    fn fetched_style_profile(&self) -> Option<&Arc<model::Profile>> {
        let card = self.profile.as_ref()?;
        match self.profile_styles.as_ref().filter(|editor| {
            card.user_id == self.store.current_user_id && card.guild_id == editor.scope.guild_id()
        }) {
            Some(editor) => editor.saved_profile.as_ref(),
            None => card.profile.as_ref(),
        }
    }

    /// Saved values for the editor's scope, read from the open profile and the store.
    pub(crate) fn saved_style_values(&self) -> StyleValues {
        let scope = self
            .profile_styles
            .as_ref()
            .map(|editor| editor.scope)
            .or_else(|| {
                self.profile
                    .as_ref()
                    .map(|card| card.guild_id.map_or(StyleScope::Main, StyleScope::Guild))
            });
        let Some(StyleScope::Guild(guild_id)) = scope else {
            return self.main_style_values();
        };
        let Some(card) = &self.profile else {
            return StyleValues::default();
        };
        let profile = if card.guild_id == Some(guild_id) {
            self.fetched_style_profile().map(Arc::as_ref)
        } else {
            card.scope_snapshots.get(&Some(guild_id)).map(Arc::as_ref)
        };
        profile.map(guild_values).unwrap_or_default()
    }

    /// The main profile's saved values, which a server scope falls back to.
    pub(crate) fn main_style_values(&self) -> StyleValues {
        let Some(card) = &self.profile else {
            return StyleValues::default();
        };
        let profile = self
            .fetched_style_profile()
            .map(Arc::as_ref)
            .or_else(|| card.scope_snapshots.values().next().map(Arc::as_ref));
        let mut values = profile.map(main_values).unwrap_or_default();
        if let Some(user) = self.store.user(card.user_id) {
            if profile.is_none() {
                values.avatar = user.avatar.clone().map(Into::into);
            }
            if profile.is_none_or(|p| p.user.user.avatar_decoration_data.is_none()) {
                values.decoration = user
                    .decoration
                    .as_ref()
                    .and_then(|decoration| decoration.sku_id);
            }
            if profile.is_none_or(|p| p.user.user.collectibles.is_none()) {
                values.nameplate =
                    user.nameplate.as_ref().and_then(|nameplate| {
                        self.profile_styles
                            .as_ref()?
                            .owned_of(CollectibleKind::Nameplate)
                            .into_iter()
                            .find(|owned| {
                                owned.product.items.iter().any(|item| {
                                    item.asset.as_deref() == Some(nameplate.asset.as_str())
                                })
                            })
                            .map(|item| item.sku_id)
                    });
            }
            if profile.is_none_or(|p| p.user.user.display_name_styles.is_none()) {
                values.name_style = user
                    .name_style
                    .as_ref()
                    .map(|style| name_style(style.font_id, style.effect_id, &style.colors));
            }
        }
        values
    }

    pub(crate) fn toggle_styles_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.profile_styles {
            editor.expanded = !editor.expanded;
            cx.notify();
        }
    }

    pub(crate) fn set_style_scope(&mut self, scope: StyleScope, cx: &mut Context<Self>) {
        if let StyleScope::Guild(id) = scope
            && self.store.guild_by_id(id).is_none()
        {
            return;
        }
        let Some(editor) = &self.profile_styles else {
            return;
        };
        if editor.scope == scope {
            return;
        }
        self.restore_style_profile();
        let Some(editor) = &mut self.profile_styles else {
            return;
        };
        editor.scope = scope;
        editor.scope_revision += 1;
        editor.draft = StyleDraft::default();
        editor.ui.identity.leave_scope();
        editor.error = None;
        self.prune_style_previews(true);
        let changed = self
            .profile
            .as_mut()
            .is_some_and(|card| card.switch_to_scope(scope.guild_id()));
        self.capture_saved_style_profile();
        if changed {
            self.load_profile_scope(cx);
        }
        self.refresh_style_cosmetics(cx);
        cx.notify();
    }

    /// Edits that end up matching the saved values drop back to unchanged.
    pub(crate) fn update_style_draft(
        &mut self,
        update: impl FnOnce(&mut StyleDraft),
        cx: &mut Context<Self>,
    ) {
        let saved = self.saved_style_values();
        let Some(editor) = &mut self.profile_styles else {
            return;
        };
        let before: Vec<PathBuf> = draft_previews(&editor.draft)
            .into_iter()
            .map(Path::to_path_buf)
            .collect();
        update(&mut editor.draft);
        editor.draft_revision += 1;
        normalize_draft(&mut editor.draft, &saved);
        editor.error = None;
        // Only an image change can leave a preview behind; colors change on every drag.
        if draft_previews(&editor.draft) != before {
            self.prune_style_previews(true);
        }
        self.refresh_style_cosmetics(cx);
        cx.notify();
    }

    pub(crate) fn pick_style_image(&mut self, target: ImageTarget, cx: &mut Context<Self>) {
        let Some(editor) = self.profile_styles.as_ref().filter(|editor| !editor.saving) else {
            return;
        };
        let (identity, scope_revision) = (editor.identity.clone(), editor.scope_revision);
        let previous = match target {
            ImageTarget::Avatar => editor.draft.avatar.clone(),
            ImageTarget::Banner => editor.draft.banner.clone(),
        };
        let preview_px = crate::theme::image_px(STAGE[0] * MAX_ZOOM);
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let path = match paths.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => return,
                _ => {
                    this.update(cx, |app, cx| {
                        if let Some(editor) = app.profile_styles.as_mut().filter(|editor| {
                            Arc::ptr_eq(&editor.identity, &identity)
                                && editor.scope_revision == scope_revision
                        }) {
                            editor.error = Some("Could not open the image picker.".into());
                            cx.notify();
                        }
                    })
                    .ok();
                    return;
                }
            };
            let Some(path) = path else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { ImageEdit::load(path, target, preview_px) })
                .await;
            this.update(cx, |app, cx| {
                let Some(editor) = app.profile_styles.as_mut().filter(|editor| {
                    Arc::ptr_eq(&editor.identity, &identity)
                        && editor.scope_revision == scope_revision
                        && !editor.saving
                }) else {
                    return;
                };
                let current = match target {
                    ImageTarget::Avatar => &editor.draft.avatar,
                    ImageTarget::Banner => &editor.draft.banner,
                };
                if current != &previous {
                    return;
                }
                match result {
                    Ok(edit) => app.open_style_dialog(StyleDialog::ImageEdit(edit), cx),
                    Err(error) => {
                        editor.error = Some(error.into());
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn pick_style_gif(
        &mut self,
        target: ImageTarget,
        src: String,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.profile_styles.as_ref().filter(|editor| !editor.saving) else {
            return;
        };
        let (identity, scope_revision) = (editor.identity.clone(), editor.scope_revision);
        let previous = match target {
            ImageTarget::Avatar => editor.draft.avatar.clone(),
            ImageTarget::Banner => editor.draft.banner.clone(),
        };
        let Some(service) = images::download_service(cx) else {
            if let Some(editor) = &mut self.profile_styles {
                editor.error = Some("Could not start the GIF download.".into());
                cx.notify();
            }
            return;
        };
        let preview_px = crate::theme::image_px(STAGE[0] * MAX_ZOOM);
        let request = Tokio::spawn(cx, async move {
            let path = service
                .load_gif_provider_media(&src, STYLE_IMAGE_LIMIT)
                .await?;
            tokio::task::spawn_blocking(move || ImageEdit::load(path, target, preview_px))
                .await
                .map_err(|error| {
                    log::warn!("Profile GIF preparation task failed: {error}");
                    "Could not prepare this GIF.".to_owned()
                })?
        });
        cx.spawn(async move |this, cx| {
            let result = request.await.unwrap_or_else(|error| {
                log::warn!("Profile GIF download task failed: {error}");
                Err("Could not download this GIF.".to_owned())
            });
            this.update(cx, |app, cx| {
                let Some(editor) = app.profile_styles.as_mut().filter(|editor| {
                    Arc::ptr_eq(&editor.identity, &identity)
                        && editor.scope_revision == scope_revision
                        && !editor.saving
                }) else {
                    return;
                };
                let current = match target {
                    ImageTarget::Avatar => &editor.draft.avatar,
                    ImageTarget::Banner => &editor.draft.banner,
                };
                if current != &previous {
                    return;
                }
                match result {
                    Ok(edit) => app.open_style_dialog(StyleDialog::ImageEdit(edit), cx),
                    Err(error) => {
                        editor.error = Some(error.into());
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn reset_style_draft(&mut self, cx: &mut Context<Self>) {
        self.reset_widget_draft();
        if let Some(editor) = &mut self.profile_styles {
            editor.draft = StyleDraft::default();
            editor.ui.identity = Default::default();
            editor.error = None;
            editor.scope_revision += 1;
            self.prune_style_previews(true);
            self.refresh_style_cosmetics(cx);
            cx.notify();
        }
    }

    pub(crate) fn save_style_draft(&mut self, cx: &mut Context<Self>) {
        let saved = self.saved_style_values();
        let Some(card) = self
            .profile
            .as_ref()
            .filter(|card| card.user_id == self.store.current_user_id && card.modal)
        else {
            return;
        };
        let user_id = card.user_id;
        if card.profile.is_none() || card.modal_request_pending || !card.board_requested {
            return;
        }
        let identity_changes = self
            .profile_styles
            .as_ref()
            .map(|editor| crate::app::profile::identity_editor::changes(self, editor))
            .unwrap_or_default();
        let Some(editor) = self.profile_styles.as_mut().filter(|editor| !editor.saving) else {
            return;
        };
        if let Err(error) = editor.validate_collectible_draft() {
            editor.error = Some(error.into());
            cx.notify();
            return;
        }
        normalize_draft(&mut editor.draft, &saved);
        if editor.draft == StyleDraft::default()
            && identity_changes == crate::app::profile::identity_editor::Changes::default()
        {
            cx.notify();
            return;
        }
        if matches!(editor.scope, StyleScope::Guild(_)) && editor.draft.accent_color.is_some() {
            editor.error = Some("Banner color can only be changed on your main profile.".into());
            cx.notify();
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            editor.error = Some("Connect to Discord to save your profile.".into());
            cx.notify();
            return;
        };
        let (scope, scope_revision, draft_revision, identity, draft) = (
            editor.scope,
            editor.scope_revision,
            editor.draft_revision,
            editor.identity.clone(),
            editor.draft.clone(),
        );
        let avatar_description = match &draft.avatar {
            Some(ImageChange::Upload { path, .. }) => Some(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
            Some(ImageChange::Recent { id, .. }) => Some(
                editor
                    .recent_avatars
                    .as_ref()
                    .and_then(|avatars| avatars.iter().find(|avatar| avatar.id == *id))
                    .map_or_else(
                        || id.to_string(),
                        |avatar| format!("{}, {}", avatar.id, avatar.description),
                    ),
            ),
            _ => None,
        };
        editor.saving = true;
        editor.error = None;
        let sent = (
            identity_changes.name.clone(),
            identity_changes.pronouns.clone(),
            identity_changes.tag_guild,
        );
        let request = Tokio::spawn(cx, async move {
            let mut patch = style_patch(&draft, &saved);
            patch.name = identity_changes.name;
            patch.pronouns = identity_changes.pronouns;
            patch.tag_guild = identity_changes.tag_guild;
            if let Some(ImageChange::Recent { storage_hash, id }) = &draft.avatar {
                if scope.guild_id().is_some() {
                    patch.avatar = Some(Some(
                        session
                            .recent_avatar_data_uri(user_id, storage_hash.as_ref())
                            .await?,
                    ));
                } else {
                    patch.avatar_id = Some(*id);
                }
            }
            patch.avatar_description = avatar_description;
            session
                .save_profile_styles(user_id, scope.guild_id(), &patch)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let mut editor = app
                    .profile_styles
                    .as_mut()
                    .filter(|editor| Arc::ptr_eq(&editor.identity, &identity));
                if let Some(editor) = &mut editor {
                    editor.saving = false;
                }
                match result {
                    Ok(Ok(())) => {
                        if let Some(editor) = editor
                            && editor.scope_revision == scope_revision
                            && editor.draft_revision == draft_revision
                        {
                            editor.draft = StyleDraft::default();
                            editor.error = None;
                            editor.scope_revision += 1;
                        }
                        // Edits made to these while saving stay as the next ones to save.
                        if let Some(editor) = app.profile_styles.as_mut().filter(|editor| {
                            Arc::ptr_eq(&editor.identity, &identity) && editor.scope == scope
                        }) {
                            let ui = &mut editor.ui.identity;
                            if sent.0.is_some() && ui.display_name == sent.0 {
                                ui.display_name = None;
                            }
                            if sent.1.is_some() && ui.pronouns == sent.1 {
                                ui.pronouns = None;
                            }
                            if sent.2.is_some() && ui.tag_guild == sent.2 {
                                ui.tag_guild = None;
                            }
                        }
                        app.prune_style_previews(true);
                        if app
                            .profile
                            .as_ref()
                            .is_some_and(|card| card.user_id == user_id)
                        {
                            // A tag is account-wide, so every scope's profile has to reload.
                            let refreshed = if sent.2.is_some() {
                                StyleScope::Main
                            } else {
                                scope
                            };
                            app.refresh_style_profile(refreshed, cx);
                        }
                    }
                    Ok(Err(error)) => {
                        if let Some(editor) = editor {
                            editor.error = Some(style_error(&error));
                        }
                    }
                    Err(error) => {
                        log::warn!("Profile styles save task failed: {error}");
                        if let Some(editor) = editor {
                            editor.error = Some("Could not save your profile.".into());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Also installed on the open card. Uploads still need the draft's data URI;
    /// a server's recent avatar uses the user's CDN, not the guild's.
    pub(crate) fn preview_profile(&self) -> Option<Arc<model::Profile>> {
        let card = self.profile.as_ref()?;
        let Some(editor) = self.profile_styles.as_ref().filter(|editor| {
            card.user_id == self.store.current_user_id && editor.scope.guild_id() == card.guild_id
        }) else {
            return card.profile.clone();
        };
        editor.preview_profile()
    }

    fn refresh_style_cosmetics(&mut self, cx: &mut Context<Self>) {
        self.rebuild_style_preview();
        if let Some(profile) = self.preview_profile() {
            let (effect, frame) = crate::app::profile::loading::cosmetic_skus(&profile);
            self.load_profile_cosmetics(false, effect, frame, cx);
        }
        self.load_avatar_theme(false, cx);
    }
}
