//! State and actions behind the profile styles panel. The panel and its dialogs
//! (`styles_panel`) only read this state and call these actions.

use std::{
    collections::HashSet,
    io::{self, Cursor, Read, Write},
    path::PathBuf,
    sync::Arc,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use gpui::{Context, PathPromptOptions, SharedString};
use gpui_tokio::Tokio;
use image::{
    AnimationDecoder, ImageDecoder, ImageFormat, ImageReader,
    codecs::{
        gif::{GifDecoder, GifEncoder, Repeat},
        webp::WebPDecoder,
    },
};

use crate::{
    app::DiscordApp,
    discord::{Error, model},
    media::images,
};

const STYLE_IMAGE_LIMIT: u64 = 10 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StyleScope {
    Main,
    Guild(u64),
}

impl StyleScope {
    pub(super) fn guild_id(self) -> Option<u64> {
        match self {
            Self::Main => None,
            Self::Guild(id) => Some(id),
        }
    }
}

/// `type` of a collectible item: 0 decoration, 1 profile effect, 2 nameplate, 3 frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CollectibleKind {
    Decoration,
    Effect,
    Nameplate,
    Frame,
}

impl CollectibleKind {
    fn from_type(kind: u16) -> Option<Self> {
        match kind {
            0 => Some(Self::Decoration),
            1 => Some(Self::Effect),
            2 => Some(Self::Nameplate),
            3 => Some(Self::Frame),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct OwnedCollectible {
    pub kind: CollectibleKind,
    /// The item's SKU, which is what a save sends.
    pub sku_id: u64,
    /// One product per item; bundle items get a product of their own.
    pub product: Arc<model::CollectibleProduct>,
    pub purchased_at: Option<SharedString>,
}

impl OwnedCollectible {
    fn from_shop_product(product: Arc<model::CollectibleProduct>) -> Option<Self> {
        Some(Self {
            kind: CollectibleKind::from_type(product.kind)?,
            sku_id: product.sku_id,
            product,
            purchased_at: None,
        })
    }
}

#[derive(Clone)]
pub(crate) struct RecentAvatar {
    pub id: u64,
    pub storage_hash: SharedString,
    pub description: SharedString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageTarget {
    Avatar,
    Banner,
}

#[derive(Clone, PartialEq)]
pub(crate) enum ImageChange {
    /// A local file, already validated and encoded for upload.
    Upload {
        path: PathBuf,
        data_uri: Arc<str>,
    },
    /// One of `/users/@me/avatars`; avatars only.
    Recent {
        id: u64,
        storage_hash: SharedString,
    },
    Remove,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NameStyle {
    pub font_id: u32,
    pub effect_id: u32,
    pub colors: Vec<u32>,
}

/// Unsaved edits for the current scope. `None` leaves a field as saved; `Some(None)`
/// resets it to the default ("Use Default").
#[derive(Clone, Default, PartialEq)]
pub(crate) struct StyleDraft {
    pub avatar: Option<ImageChange>,
    pub banner: Option<ImageChange>,
    pub decoration: Option<Option<u64>>,
    pub nameplate: Option<Option<u64>>,
    pub effect: Option<Option<u64>>,
    pub frame: Option<Option<u64>>,
    pub name_style: Option<Option<NameStyle>>,
    /// Nitro: `[primary, accent]`.
    pub theme_colors: Option<Option<[u32; 2]>>,
    /// Without Nitro the theme is a single banner color.
    pub accent_color: Option<Option<u32>>,
}

/// What the scope has saved; `None` means not set at this scope.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct StyleValues {
    pub avatar: Option<SharedString>,
    pub banner: Option<SharedString>,
    pub decoration: Option<u64>,
    pub nameplate: Option<u64>,
    pub effect: Option<u64>,
    pub frame: Option<u64>,
    pub name_style: Option<NameStyle>,
    pub theme_colors: Option<[u32; 2]>,
    pub accent_color: Option<u32>,
}

pub(crate) struct StylesEditor {
    pub expanded: bool,
    pub scope: StyleScope,
    pub draft: StyleDraft,
    pub nitro: bool,
    /// `None` until `/users/@me/collectibles-purchases` answers.
    pub owned: Option<Arc<Vec<OwnedCollectible>>>,
    pub shop: Option<Arc<Vec<OwnedCollectible>>>,
    pub recent_avatars: Option<Arc<Vec<RecentAvatar>>>,
    pub saving: bool,
    pub error: Option<SharedString>,
    /// Panel and dialog presentation state, owned by the UI.
    pub ui: super::styles_panel::PanelUi,
    /// Fetched baseline; the open card holds a derived preview while editing.
    pub(super) saved_profile: Option<Arc<model::Profile>>,
    identity: Arc<()>,
    scope_revision: u64,
    draft_revision: u64,
    shop_requested: bool,
    shop_catalog: Option<Arc<Vec<OwnedCollectible>>>,
}

impl StylesEditor {
    fn new(scope: StyleScope, nitro: bool) -> Self {
        Self {
            expanded: true,
            scope,
            draft: StyleDraft::default(),
            nitro,
            owned: None,
            shop: None,
            recent_avatars: None,
            saving: false,
            error: None,
            ui: Default::default(),
            saved_profile: None,
            identity: Arc::new(()),
            scope_revision: 0,
            draft_revision: 0,
            shop_requested: false,
            shop_catalog: None,
        }
    }

    pub(crate) fn owned_of(&self, kind: CollectibleKind) -> Vec<&OwnedCollectible> {
        self.owned
            .iter()
            .flat_map(|items| items.iter())
            .filter(|item| item.kind == kind)
            .collect()
    }

    pub(crate) fn owned_item(&self, sku_id: u64) -> Option<&OwnedCollectible> {
        self.owned
            .as_ref()?
            .iter()
            .find(|item| item.sku_id == sku_id)
    }

    pub(crate) fn shop_of(&self, kind: CollectibleKind) -> Vec<&OwnedCollectible> {
        self.shop
            .iter()
            .flat_map(|items| items.iter())
            .filter(|item| item.kind == kind)
            .collect()
    }

    pub(crate) fn find_collectible(&self, sku_id: u64) -> Option<&OwnedCollectible> {
        self.owned_item(sku_id).or_else(|| {
            self.shop
                .as_ref()?
                .iter()
                .find(|item| item.sku_id == sku_id)
        })
    }

    pub(crate) fn is_owned(&self, sku_id: u64) -> bool {
        self.owned_item(sku_id).is_some()
    }

    fn refresh_shop(&mut self) {
        let (Some(catalog), Some(owned)) = (&self.shop_catalog, &self.owned) else {
            return;
        };
        let owned: HashSet<_> = owned.iter().map(|item| item.sku_id).collect();
        self.shop = Some(Arc::new(
            catalog
                .iter()
                .filter(|item| !owned.contains(&item.sku_id))
                .cloned()
                .collect(),
        ));
    }

    fn validate_collectible_draft(&self) -> Result<(), &'static str> {
        for sku_id in [
            self.draft.decoration,
            self.draft.nameplate,
            self.draft.effect,
            self.draft.frame,
        ]
        .into_iter()
        .flatten()
        .flatten()
        {
            if self.owned.is_none() {
                return Err("Wait for your collectibles to load before saving.");
            }
            if !self.is_owned(sku_id) {
                return Err("You can only save collectibles you own.");
            }
        }
        Ok(())
    }

    fn preview_profile(&self) -> Option<Arc<model::Profile>> {
        let mut profile = self.saved_profile.clone()?;
        if self.draft != StyleDraft::default() {
            apply_preview(Arc::make_mut(&mut profile), self);
        }
        Some(profile)
    }
}

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

    pub(super) fn open_styles_editor(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn styles_profile_loaded(&mut self) {
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

    pub(super) fn drop_styles_editor(&mut self) {
        self.restore_style_profile();
        self.profile_styles = None;
    }

    pub(super) fn rebuild_style_preview(&mut self) {
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
        editor.error = None;
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
        update(&mut editor.draft);
        editor.draft_revision += 1;
        normalize_draft(&mut editor.draft, &saved);
        editor.error = None;
        self.refresh_style_cosmetics(cx);
        cx.notify();
    }

    pub(crate) fn pick_style_image(&mut self, target: ImageTarget, cx: &mut Context<Self>) {
        let Some(editor) = self.profile_styles.as_ref().filter(|editor| !editor.saving) else {
            return;
        };
        let (identity, scope_revision, nitro) =
            (editor.identity.clone(), editor.scope_revision, editor.nitro);
        let previous = match target {
            ImageTarget::Avatar => editor.draft.avatar.clone(),
            ImageTarget::Banner => editor.draft.banner.clone(),
        };
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
                .spawn(async move { prepare_style_image(path, target, nitro) })
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
                    Ok(image) => app.update_style_draft(
                        |draft| match target {
                            ImageTarget::Avatar => draft.avatar = Some(image),
                            ImageTarget::Banner => draft.banner = Some(image),
                        },
                        cx,
                    ),
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
        let (identity, scope_revision, nitro) =
            (editor.identity.clone(), editor.scope_revision, editor.nitro);
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
        let request = Tokio::spawn(cx, async move {
            let path = service
                .load_gif_provider_media(&src, STYLE_IMAGE_LIMIT)
                .await?;
            tokio::task::spawn_blocking(move || prepare_style_image(path, target, nitro))
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
                    Ok(image) => app.update_style_draft(
                        |draft| match target {
                            ImageTarget::Avatar => draft.avatar = Some(image),
                            ImageTarget::Banner => draft.banner = Some(image),
                        },
                        cx,
                    ),
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
        if let Some(editor) = &mut self.profile_styles {
            editor.draft = StyleDraft::default();
            editor.error = None;
            editor.scope_revision += 1;
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
        let Some(editor) = self.profile_styles.as_mut().filter(|editor| !editor.saving) else {
            return;
        };
        if let Err(error) = editor.validate_collectible_draft() {
            editor.error = Some(error.into());
            cx.notify();
            return;
        }
        normalize_draft(&mut editor.draft, &saved);
        if editor.draft == StyleDraft::default() {
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
        let request = Tokio::spawn(cx, async move {
            let mut patch = style_patch(&draft, &saved);
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
                        if app
                            .profile
                            .as_ref()
                            .is_some_and(|card| card.user_id == user_id)
                        {
                            app.refresh_style_profile(scope, cx);
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
            let (effect, frame) = super::loading::cosmetic_skus(&profile);
            self.load_profile_cosmetics(false, effect, frame, cx);
        }
        self.load_avatar_theme(false, cx);
    }
}

fn name_style(font_id: Option<u32>, effect_id: Option<u32>, colors: &[u32]) -> NameStyle {
    NameStyle {
        font_id: font_id.unwrap_or(11),
        effect_id: effect_id.filter(|id| (1..=8).contains(id)).unwrap_or(1),
        colors: colors.to_vec(),
    }
}

fn profile_values(profile: Option<&model::UserProfile>) -> StyleValues {
    let Some(profile) = profile else {
        return StyleValues::default();
    };
    StyleValues {
        banner: profile.banner.clone().map(Into::into),
        effect: profile
            .profile_effect
            .as_ref()
            .map(|effect| effect.sku_id)
            .or_else(|| {
                profile
                    .collectibles
                    .iter()
                    .find(|item| item.kind == 1)
                    .map(|item| item.sku_id)
            }),
        frame: profile
            .collectibles
            .iter()
            .find(|item| item.kind == 3)
            .map(|item| item.sku_id),
        theme_colors: profile
            .theme_colors
            .as_deref()
            .and_then(|colors| colors.try_into().ok()),
        accent_color: profile.accent_color,
        ..Default::default()
    }
}

fn main_values(profile: &model::Profile) -> StyleValues {
    let user = &profile.user.user;
    let mut values = profile_values(profile.user_profile.as_ref());
    values.avatar = user.avatar.clone().map(Into::into);
    values.banner = profile
        .user
        .banner
        .clone()
        .map(Into::into)
        .or(values.banner);
    values.accent_color = profile
        .user
        .accent_color
        .or(profile.user.banner_color)
        .or(values.accent_color);
    values.decoration = user
        .avatar_decoration_data
        .as_ref()
        .and_then(Option::as_ref)
        .and_then(|decoration| decoration.sku_id);
    values.nameplate = user
        .collectibles
        .as_ref()
        .and_then(Option::as_ref)
        .and_then(|items| items.nameplate.as_ref())
        .and_then(|nameplate| nameplate.sku_id);
    values.name_style = user
        .display_name_styles
        .as_ref()
        .and_then(Option::as_ref)
        .map(|style| name_style(style.font_id, style.effect_id, &style.colors));
    values
}

fn guild_values(profile: &model::Profile) -> StyleValues {
    let mut values = profile_values(profile.guild_member_profile.as_ref());
    if let Some(member) = &profile.guild_member {
        values.avatar = member
            .member
            .avatar
            .as_ref()
            .and_then(Option::as_ref)
            .cloned()
            .map(Into::into);
        values.banner = member.banner.clone().map(Into::into).or(values.banner);
        values.decoration = member
            .member
            .avatar_decoration_data
            .as_ref()
            .and_then(Option::as_ref)
            .and_then(|decoration| decoration.sku_id);
        values.nameplate = member
            .member
            .collectibles
            .as_ref()
            .and_then(Option::as_ref)
            .and_then(|items| items.nameplate.as_ref())
            .and_then(|nameplate| nameplate.sku_id);
        values.name_style = member
            .member
            .display_name_styles
            .as_ref()
            .and_then(Option::as_ref)
            .map(|style| name_style(style.font_id, style.effect_id, &style.colors));
    }
    values
}

fn normalize<T: PartialEq>(change: &mut Option<Option<T>>, saved: &Option<T>) {
    if change.as_ref() == Some(saved) {
        *change = None;
    }
}

fn normalize_image(change: &mut Option<ImageChange>, saved: &Option<SharedString>) {
    let unchanged = match change.as_ref() {
        Some(ImageChange::Remove) => saved.is_none(),
        Some(ImageChange::Recent { storage_hash, .. }) => saved.as_ref() == Some(storage_hash),
        _ => false,
    };
    if unchanged {
        *change = None;
    }
}

fn normalize_draft(draft: &mut StyleDraft, saved: &StyleValues) {
    if let Some(Some(style)) = &mut draft.name_style
        && !(1..=8).contains(&style.effect_id)
    {
        style.effect_id = 1;
    }
    normalize_image(&mut draft.avatar, &saved.avatar);
    normalize_image(&mut draft.banner, &saved.banner);
    normalize(&mut draft.decoration, &saved.decoration);
    normalize(&mut draft.nameplate, &saved.nameplate);
    normalize(&mut draft.effect, &saved.effect);
    normalize(&mut draft.frame, &saved.frame);
    normalize(&mut draft.name_style, &saved.name_style);
    normalize(&mut draft.theme_colors, &saved.theme_colors);
    normalize(&mut draft.accent_color, &saved.accent_color);
}

fn style_patch(draft: &StyleDraft, saved: &StyleValues) -> model::ProfileStylePatch {
    let mut patch = model::ProfileStylePatch {
        avatar: match &draft.avatar {
            Some(ImageChange::Upload { data_uri, .. }) => Some(Some(data_uri.to_string())),
            Some(ImageChange::Remove) => Some(None),
            _ => None,
        },
        banner: match &draft.banner {
            Some(ImageChange::Upload { data_uri, .. }) => Some(Some(data_uri.to_string())),
            Some(ImageChange::Remove) => Some(None),
            _ => None,
        },
        decoration: draft.decoration,
        nameplate: draft.nameplate,
        theme_colors: draft.theme_colors,
        accent_color: draft.accent_color,
        ..Default::default()
    };
    if let Some(style) = &draft.name_style {
        match style {
            Some(style) => {
                if saved
                    .name_style
                    .as_ref()
                    .is_none_or(|saved| saved.font_id != style.font_id)
                {
                    patch.display_name_font_id = Some(Some(style.font_id));
                }
                let effect = name_style(None, Some(style.effect_id), &[]).effect_id;
                if saved
                    .name_style
                    .as_ref()
                    .is_none_or(|saved| saved.effect_id != effect)
                {
                    patch.display_name_effect_id = Some(Some(effect));
                }
                if saved
                    .name_style
                    .as_ref()
                    .is_none_or(|saved| saved.colors != style.colors)
                {
                    patch.display_name_colors = Some(Some(style.colors.clone()));
                }
            }
            None => {
                patch.display_name_font_id = Some(None);
                patch.display_name_effect_id = Some(None);
                patch.display_name_colors = Some(None);
            }
        }
    }
    if draft.effect.is_some() || draft.frame.is_some() {
        patch.collectibles_sku_ids = Some(
            draft
                .effect
                .unwrap_or(saved.effect)
                .into_iter()
                .chain(draft.frame.unwrap_or(saved.frame))
                .collect(),
        );
    }
    patch
}

fn style_error(error: &Error) -> SharedString {
    match error {
        Error::Rejected { message, .. } if !message.is_empty() => {
            message.chars().take(240).collect::<String>().into()
        }
        _ => error.to_string().into(),
    }
}

fn prepare_style_image(
    path: PathBuf,
    target: ImageTarget,
    nitro: bool,
) -> Result<ImageChange, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") {
        return Err("Choose a PNG, JPEG, GIF, or WebP image.".into());
    }
    let file = std::fs::File::open(&path).map_err(|_| "Could not read the image.".to_owned())?;
    let metadata = file
        .metadata()
        .map_err(|_| "Could not read the image.".to_owned())?;
    if !metadata.is_file() || metadata.len() >= STYLE_IMAGE_LIMIT {
        return Err("Choose an image smaller than 10 MB.".into());
    }
    let mut bytes = Vec::new();
    file.take(STYLE_IMAGE_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the image.".to_owned())?;
    if bytes.len() as u64 >= STYLE_IMAGE_LIMIT {
        return Err("Choose an image smaller than 10 MB.".into());
    }
    let data_uri = encode_style_image(&bytes, target, nitro)?;
    Ok(ImageChange::Upload {
        path,
        data_uri: data_uri.into(),
    })
}

fn image_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits
}

struct StyleGifWriter {
    bytes: Vec<u8>,
    limit: usize,
    overflowed: bool,
}

impl Write for StyleGifWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) >= self.limit {
            self.overflowed = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "GIF exceeds upload limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn webp_style_gif(bytes: &[u8], max_bytes: usize) -> Result<Option<Vec<u8>>, String> {
    let mut decoder = WebPDecoder::new(Cursor::new(bytes))
        .map_err(|_| "Could not decode this image.".to_owned())?;
    if !decoder.has_animation() {
        return Ok(None);
    }
    let mut limits = image_limits();
    decoder
        .set_limits(limits.clone())
        .map_err(|_| "This GIF is too large to process.".to_owned())?;
    let (width, height) = decoder.dimensions();
    // WebP's animation iterator does not enforce max_alloc itself.
    limits
        .reserve_buffer(width, height, image::ColorType::Rgba8)
        .map_err(|_| "This GIF is too large to process.".to_owned())?;
    let repeat = match decoder.loop_count() {
        image::metadata::LoopCount::Infinite => Repeat::Infinite,
        image::metadata::LoopCount::Finite(count) => {
            Repeat::Finite(u16::try_from(count.get()).unwrap_or(u16::MAX))
        }
    };
    let mut output = StyleGifWriter {
        bytes: Vec::new(),
        limit: max_bytes,
        overflowed: false,
    };
    let encoded = (|| -> Result<usize, String> {
        let mut encoder = GifEncoder::new_with_speed(&mut output, 10);
        encoder
            .set_repeat(repeat)
            .map_err(|_| "Could not convert this GIF.".to_owned())?;
        let mut count = 0;
        let mut pixels = 0u64;
        for frame in decoder.into_frames() {
            if count >= 1024 {
                return Err("This GIF has too many frames.".into());
            }
            pixels = pixels.saturating_add(u64::from(width) * u64::from(height));
            if pixels > 128 * 1024 * 1024 {
                return Err("This GIF is too large to process.".into());
            }
            encoder
                .encode_frame(frame.map_err(|_| "Could not decode this image.".to_owned())?)
                .map_err(|_| "Could not convert this GIF.".to_owned())?;
            count += 1;
        }
        Ok(count)
    })();
    if output.overflowed {
        return Err("This GIF exceeds the 10 MB upload limit after conversion.".into());
    }
    Ok((encoded? > 1).then_some(output.bytes))
}

fn encode_style_image(bytes: &[u8], target: ImageTarget, nitro: bool) -> Result<String, String> {
    if bytes.len() as u64 >= STYLE_IMAGE_LIMIT {
        return Err("Choose an image smaller than 10 MB.".into());
    }
    let invalid = || "Could not decode this image.".to_owned();
    let format = image::guess_format(bytes).map_err(|_| invalid())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return Err("Choose a PNG, JPEG, GIF, or WebP image.".into());
    }
    if format == ImageFormat::WebP
        && nitro
        && let Some(gif) = webp_style_gif(bytes, STYLE_IMAGE_LIMIT as usize)?
    {
        return Ok(format!("data:image/gif;base64,{}", STANDARD.encode(gif)));
    }
    let image = if format == ImageFormat::Gif {
        let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|_| invalid())?;
        decoder.set_limits(image_limits()).map_err(|_| invalid())?;
        let mut frames = decoder.into_frames();
        let first = frames.next().ok_or_else(invalid)?.map_err(|_| invalid())?;
        if nitro && frames.next().transpose().map_err(|_| invalid())?.is_some() {
            return Ok(format!("data:image/gif;base64,{}", STANDARD.encode(bytes)));
        }
        image::DynamicImage::ImageRgba8(first.into_buffer())
    } else {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(image_limits());
        reader.decode().map_err(|_| invalid())?
    };
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return Err(invalid());
    }
    let (ratio_width, ratio_height, max_width, max_height) = match target {
        ImageTarget::Avatar => (1, 1, 1024, 1024),
        ImageTarget::Banner => (17, 6, 2400, 848),
    };
    let (crop_width, crop_height) =
        if u64::from(width) * ratio_height > u64::from(height) * ratio_width {
            (
                (u64::from(height) * ratio_width / ratio_height).max(1) as u32,
                height,
            )
        } else {
            (
                width,
                (u64::from(width) * ratio_height / ratio_width).max(1) as u32,
            )
        };
    let cropped = image.crop_imm(
        (width - crop_width) / 2,
        (height - crop_height) / 2,
        crop_width,
        crop_height,
    );
    let resized = if crop_width > max_width || crop_height > max_height {
        cropped.resize(max_width, max_height, image::imageops::FilterType::Lanczos3)
    } else {
        cropped
    };
    let mut png = Cursor::new(Vec::new());
    resized
        .into_rgba8()
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|_| "Could not encode the image.".to_owned())?;
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(png.into_inner())
    ))
}

fn preview_decoration(editor: &StylesEditor) -> Option<Option<model::AvatarDecoration>> {
    match editor.draft.decoration? {
        Some(sku_id) => {
            let item = editor
                .find_collectible(sku_id)?
                .product
                .items
                .iter()
                .find(|item| item.kind == 0)?;
            Some(Some(model::AvatarDecoration {
                asset: item.asset.clone()?,
                sku_id: Some(sku_id),
            }))
        }
        None => Some(None),
    }
}

fn preview_nameplate(editor: &StylesEditor) -> Option<Option<model::Collectibles>> {
    let nameplate = match editor.draft.nameplate? {
        Some(sku_id) => {
            let item = editor
                .find_collectible(sku_id)?
                .product
                .items
                .iter()
                .find(|item| item.kind == 2)?;
            Some(model::Nameplate {
                asset: item.asset.clone()?,
                palette: item.palette.clone(),
                label: item.label.clone(),
                sku_id: Some(sku_id),
            })
        }
        None => None,
    };
    Some(nameplate.map(|nameplate| model::Collectibles {
        nameplate: Some(nameplate),
    }))
}

fn apply_preview(profile: &mut model::Profile, editor: &StylesEditor) {
    let draft = &editor.draft;
    let style = draft.name_style.as_ref().map(|style| {
        style.as_ref().map(|style| model::DisplayNameStyles {
            font_id: Some(style.font_id),
            effect_id: Some(style.effect_id),
            colors: style.colors.clone(),
        })
    });
    let decoration = preview_decoration(editor);
    let nameplate = preview_nameplate(editor);
    match editor.scope {
        StyleScope::Main => {
            let user = &mut profile.user.user;
            match &draft.avatar {
                Some(ImageChange::Remove) => user.avatar = None,
                Some(ImageChange::Recent { storage_hash, .. }) => {
                    user.avatar = Some(storage_hash.to_string())
                }
                _ => {}
            }
            if let Some(decoration) = decoration {
                user.avatar_decoration_data = Some(decoration);
            }
            if let Some(nameplate) = nameplate {
                user.collectibles = Some(nameplate);
            }
            if let Some(style) = style {
                user.display_name_styles = Some(style);
            }
            if let Some(accent) = draft.accent_color {
                profile.user.accent_color = accent;
                profile.user.banner_color = accent;
            }
            if matches!(draft.banner, Some(ImageChange::Remove)) {
                profile.user.banner = None;
            }
        }
        StyleScope::Guild(_) => {
            let member = profile.guild_member.get_or_insert_with(Default::default);
            if matches!(draft.avatar, Some(ImageChange::Remove)) {
                member.member.avatar = Some(None);
            }
            if let Some(decoration) = decoration {
                member.member.avatar_decoration_data = Some(decoration);
            }
            if let Some(nameplate) = nameplate {
                member.member.collectibles = Some(nameplate);
            }
            if let Some(style) = style {
                member.member.display_name_styles = Some(style);
            }
            if matches!(draft.banner, Some(ImageChange::Remove)) {
                member.banner = None;
            }
        }
    }
    let data = match editor.scope {
        StyleScope::Main => &mut profile.user_profile,
        StyleScope::Guild(_) => &mut profile.guild_member_profile,
    }
    .get_or_insert_with(Default::default);
    if let Some(colors) = draft.theme_colors {
        data.theme_colors = colors.map(Vec::from);
    }
    if let Some(accent) = draft.accent_color {
        data.accent_color = accent;
    }
    if matches!(draft.banner, Some(ImageChange::Remove)) {
        data.banner = None;
    }
    if let Some(effect) = draft.effect {
        data.profile_effect = effect.map(|sku_id| model::ProfileEffectRef { sku_id });
        data.collectibles.retain(|item| item.kind != 1);
        if let Some(sku_id) = effect {
            data.collectibles
                .push(model::ProfileCollectible { sku_id, kind: 1 });
        }
    }
    if let Some(frame) = draft.frame {
        data.collectibles.retain(|item| item.kind != 3);
        if let Some(sku_id) = frame {
            data.collectibles
                .push(model::ProfileCollectible { sku_id, kind: 3 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn inventory_item(sku_id: u64, kind: u8, name: &str) -> OwnedCollectible {
        let product = serde_json::from_value(json!({
            "sku_id": sku_id.to_string(), "type": kind, "name": name, "premium_type": 2,
            "items": [{"sku_id": sku_id.to_string(), "type": kind, "asset": name}]
        }))
        .unwrap();
        OwnedCollectible::from_shop_product(Arc::new(product)).unwrap()
    }

    #[test]
    fn profile_styles_shop_waits_for_owned_and_filters_both_load_orders() {
        let catalog = Arc::new(vec![
            inventory_item(3, 2, "Nameplate"),
            inventory_item(1, 1, "Effect"),
            inventory_item(2, 3, "Frame"),
        ]);
        let owned = Arc::new(vec![catalog[1].clone()]);
        for owned_first in [false, true] {
            let mut editor = StylesEditor::new(StyleScope::Main, true);
            if owned_first {
                editor.owned = Some(owned.clone());
            } else {
                editor.shop_catalog = Some(catalog.clone());
            }
            editor.refresh_shop();
            assert!(editor.shop.is_none());
            editor.owned = Some(owned.clone());
            editor.shop_catalog = Some(catalog.clone());
            editor.refresh_shop();
            let shop = editor.shop.as_ref().unwrap();
            assert_eq!(
                shop.iter().map(|item| item.sku_id).collect::<Vec<_>>(),
                vec![3, 2]
            );
            assert!(shop.iter().all(|item| item.purchased_at.is_none()));
            assert_eq!(editor.shop_of(CollectibleKind::Frame)[0].sku_id, 2);
            assert!(editor.shop_of(CollectibleKind::Effect).is_empty());
            editor.owned = Some(Arc::new(vec![catalog[1].clone(), catalog[0].clone()]));
            editor.refresh_shop();
            assert_eq!(
                editor
                    .shop
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|item| item.sku_id)
                    .collect::<Vec<_>>(),
                vec![2]
            );
        }
    }

    #[test]
    fn profile_styles_collectible_lookup_prefers_owned_and_keeps_ownership_separate() {
        let mut editor = StylesEditor::new(StyleScope::Main, true);
        editor.owned = Some(Arc::new(vec![inventory_item(1, 1, "Owned effect")]));
        editor.shop = Some(Arc::new(vec![
            inventory_item(1, 1, "Stale shop effect"),
            inventory_item(2, 3, "Shop frame"),
        ]));
        assert_eq!(
            editor.find_collectible(1).unwrap().product.name,
            "Owned effect"
        );
        assert_eq!(
            editor.find_collectible(2).unwrap().product.name,
            "Shop frame"
        );
        assert!(editor.find_collectible(99).is_none());
        assert!(editor.owned_item(2).is_none());
        assert!(editor.is_owned(1));
        assert!(!editor.is_owned(2));
        assert!(!editor.is_owned(99));
    }

    #[test]
    fn profile_styles_collectible_draft_refuses_shop_skus_in_every_slot() {
        for kind in [
            CollectibleKind::Decoration,
            CollectibleKind::Nameplate,
            CollectibleKind::Effect,
            CollectibleKind::Frame,
        ] {
            let mut editor = StylesEditor::new(StyleScope::Main, true);
            let item = inventory_item(99, 0, "Shop item");
            editor.shop = Some(Arc::new(vec![item.clone()]));
            editor.owned = Some(Arc::new(Vec::new()));
            let field = match kind {
                CollectibleKind::Decoration => &mut editor.draft.decoration,
                CollectibleKind::Nameplate => &mut editor.draft.nameplate,
                CollectibleKind::Effect => &mut editor.draft.effect,
                CollectibleKind::Frame => &mut editor.draft.frame,
            };
            *field = Some(Some(99));
            assert_eq!(
                editor.validate_collectible_draft(),
                Err("You can only save collectibles you own.")
            );
            editor.owned = None;
            assert_eq!(
                editor.validate_collectible_draft(),
                Err("Wait for your collectibles to load before saving.")
            );
            editor.owned = Some(Arc::new(vec![item]));
            assert_eq!(editor.validate_collectible_draft(), Ok(()));
        }
        let mut editor = StylesEditor::new(StyleScope::Main, false);
        editor.draft = StyleDraft {
            decoration: Some(None),
            nameplate: Some(None),
            effect: Some(None),
            frame: Some(None),
            ..Default::default()
        };
        assert_eq!(editor.validate_collectible_draft(), Ok(()));
    }

    fn profile() -> model::Profile {
        serde_json::from_value(json!({
            "user": {"id": "1", "avatar": "main", "banner": "main-banner", "accent_color": 5,
                "avatar_decoration_data": {"asset": "main-decoration", "sku_id": "10"},
                "collectibles": {"nameplate": {"asset": "main-plate", "sku_id": "11"}},
                "display_name_styles": {"font_id": 14, "effect_id": 3, "colors": [1, 2]}},
            "user_profile": {"theme_colors": [3, 4], "profile_effect": {"sku_id": "12"}, "collectibles": [{"type": 3, "sku_id": "13"}]},
            "guild_member": {"avatar": "server", "banner": "server-banner",
                "avatar_decoration_data": {"asset": "server-decoration", "sku_id": "20"},
                "collectibles": {"nameplate": {"asset": "server-plate", "sku_id": "21"}},
                "display_name_styles": {"font_id": 11, "effect_id": 1, "colors": [2, 3]}},
            "guild_member_profile": {"theme_colors": [5, 6], "profile_effect": {"sku_id": "22"}, "collectibles": [{"type": 3, "sku_id": "23"}]}
        })).unwrap()
    }

    #[test]
    fn profile_styles_saved_values_do_not_inherit_server_overrides() {
        let mut profile = profile();
        let main = main_values(&profile);
        let guild = guild_values(&profile);
        assert_eq!(main.avatar.as_deref(), Some("main"));
        assert_eq!(main.banner.as_deref(), Some("main-banner"));
        assert_eq!(
            (main.decoration, main.nameplate, main.effect, main.frame),
            (Some(10), Some(11), Some(12), Some(13))
        );
        assert_eq!(main.theme_colors, Some([3, 4]));
        assert_eq!(main.accent_color, Some(5));
        assert_eq!(guild.avatar.as_deref(), Some("server"));
        assert_eq!(guild.banner.as_deref(), Some("server-banner"));
        assert_eq!(
            (guild.decoration, guild.nameplate, guild.effect, guild.frame),
            (Some(20), Some(21), Some(22), Some(23))
        );
        assert_eq!(guild.theme_colors, Some([5, 6]));
        profile.guild_member = None;
        profile.guild_member_profile = None;
        assert!(guild_values(&profile) == StyleValues::default());
        assert_eq!(
            name_style(None, None, &[]),
            NameStyle {
                font_id: 11,
                effect_id: 1,
                colors: vec![]
            }
        );
    }

    #[test]
    fn profile_styles_normalize_same_values_and_resets() {
        let saved = main_values(&profile());
        let mut draft = StyleDraft {
            avatar: Some(ImageChange::Recent {
                id: 100,
                storage_hash: "main".into(),
            }),
            decoration: Some(saved.decoration),
            nameplate: Some(saved.nameplate),
            effect: Some(saved.effect),
            frame: Some(saved.frame),
            name_style: Some(saved.name_style.clone()),
            theme_colors: Some(saved.theme_colors),
            accent_color: Some(saved.accent_color),
            ..Default::default()
        };
        normalize_draft(&mut draft, &saved);
        assert!(draft == StyleDraft::default());
        let mut draft = StyleDraft {
            avatar: Some(ImageChange::Remove),
            banner: Some(ImageChange::Remove),
            decoration: Some(None),
            nameplate: Some(None),
            effect: Some(None),
            frame: Some(None),
            name_style: Some(None),
            theme_colors: Some(None),
            accent_color: Some(None),
        };
        normalize_draft(&mut draft, &StyleValues::default());
        assert!(draft == StyleDraft::default());
        let mut draft = StyleDraft {
            name_style: Some(Some(NameStyle {
                font_id: 11,
                effect_id: 0,
                colors: vec![],
            })),
            ..Default::default()
        };
        let saved = StyleValues {
            name_style: Some(NameStyle {
                font_id: 11,
                effect_id: 1,
                colors: vec![],
            }),
            ..Default::default()
        };
        normalize_draft(&mut draft, &saved);
        assert!(draft == StyleDraft::default());
    }

    #[test]
    fn profile_styles_patch_keeps_the_unchanged_effect_or_frame() {
        let saved = main_values(&profile());
        let patch = style_patch(
            &StyleDraft {
                effect: Some(Some(99)),
                ..Default::default()
            },
            &saved,
        );
        assert_eq!(patch.collectibles_sku_ids, Some(vec![99, 13]));
        let patch = style_patch(
            &StyleDraft {
                frame: Some(None),
                ..Default::default()
            },
            &saved,
        );
        assert_eq!(patch.collectibles_sku_ids, Some(vec![12]));
        let patch = style_patch(
            &StyleDraft {
                effect: Some(None),
                frame: Some(None),
                ..Default::default()
            },
            &saved,
        );
        assert_eq!(patch.collectibles_sku_ids, Some(vec![]));
        let patch = style_patch(&StyleDraft::default(), &saved);
        assert_eq!(patch.collectibles_sku_ids, None);
    }

    #[test]
    fn profile_styles_patch_only_changes_the_edited_name_style_component() {
        let saved = main_values(&profile());
        let mut next = saved.name_style.clone().unwrap();
        next.font_id = 16;
        let patch = style_patch(
            &StyleDraft {
                name_style: Some(Some(next)),
                ..Default::default()
            },
            &saved,
        );
        assert_eq!(patch.display_name_font_id, Some(Some(16)));
        assert_eq!(patch.display_name_effect_id, None);
        assert_eq!(patch.display_name_colors, None);
        let patch = style_patch(
            &StyleDraft {
                name_style: Some(None),
                ..Default::default()
            },
            &saved,
        );
        assert_eq!(patch.display_name_font_id, Some(None));
        assert_eq!(patch.display_name_effect_id, Some(None));
        assert_eq!(patch.display_name_colors, Some(None));
    }

    #[test]
    fn profile_styles_preview_is_a_clone_and_resets_inherit_main() {
        let original = Arc::new(profile());
        let mut editor = StylesEditor::new(StyleScope::Guild(7), true);
        editor.saved_profile = Some(original.clone());
        editor.draft = StyleDraft {
            decoration: Some(None),
            nameplate: Some(None),
            name_style: Some(None),
            theme_colors: Some(None),
            effect: Some(None),
            frame: Some(None),
            ..Default::default()
        };
        let preview = editor.preview_profile().unwrap();
        assert_eq!(
            super::super::loading::cosmetic_skus(&preview),
            (Some(12), Some(13))
        );
        let values = guild_values(&preview);
        assert_eq!(
            (
                values.decoration,
                values.nameplate,
                values.effect,
                values.frame
            ),
            (None, None, None, None)
        );
        assert!(values.name_style.is_none() && values.theme_colors.is_none());
        assert_eq!(guild_values(&original).effect, Some(22));
        assert_eq!(guild_values(&original).theme_colors, Some([5, 6]));
    }

    #[test]
    fn profile_styles_previews_always_derive_from_the_fetched_baseline() {
        let original = Arc::new(profile());
        let mut editor = StylesEditor::new(StyleScope::Main, true);
        editor.saved_profile = Some(original.clone());
        editor.draft.effect = Some(Some(99));
        editor.draft.theme_colors = Some(Some([10, 11]));
        let preview = editor.preview_profile().unwrap();
        assert_eq!(main_values(&preview).effect, Some(99));
        assert_eq!(main_values(&preview).theme_colors, Some([10, 11]));
        assert_eq!(
            main_values(editor.saved_profile.as_ref().unwrap()).effect,
            Some(12)
        );
        assert_eq!(
            main_values(editor.saved_profile.as_ref().unwrap()).theme_colors,
            Some([3, 4])
        );
        editor.draft.effect = None;
        let preview = editor.preview_profile().unwrap();
        assert_eq!(main_values(&preview).effect, Some(12));
        assert_eq!(main_values(&preview).theme_colors, Some([10, 11]));
        editor.draft = StyleDraft::default();
        assert!(Arc::ptr_eq(&editor.preview_profile().unwrap(), &original));
        let mut fetched = profile();
        fetched.user_profile.as_mut().unwrap().theme_colors = Some(vec![20, 21]);
        editor.saved_profile = Some(Arc::new(fetched));
        assert_eq!(
            main_values(&editor.preview_profile().unwrap()).theme_colors,
            Some([20, 21])
        );
    }

    #[test]
    fn profile_styles_preview_resolves_owned_assets() {
        let mut editor = StylesEditor::new(StyleScope::Main, true);
        let product: model::CollectibleProduct = serde_json::from_value(json!({
            "sku_id": "30", "name": "Owned", "items": [{"sku_id": "30", "type": 0, "asset": "new-decoration"}]
        })).unwrap();
        editor.owned = Some(Arc::new(vec![OwnedCollectible {
            kind: CollectibleKind::Decoration,
            sku_id: 30,
            product: Arc::new(product),
            purchased_at: None,
        }]));
        editor.draft = StyleDraft {
            decoration: Some(Some(30)),
            effect: Some(Some(31)),
            frame: Some(Some(32)),
            theme_colors: Some(Some([10, 11])),
            ..Default::default()
        };
        let mut preview = profile();
        apply_preview(&mut preview, &editor);
        let decoration = preview
            .user
            .user
            .avatar_decoration_data
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(decoration.asset, "new-decoration");
        assert_eq!(decoration.sku_id, Some(30));
        assert_eq!(main_values(&preview).effect, Some(31));
        assert_eq!(main_values(&preview).frame, Some(32));
        assert_eq!(main_values(&preview).theme_colors, Some([10, 11]));
        assert_eq!(editor.owned_of(CollectibleKind::Decoration).len(), 1);
        assert!(editor.owned_of(CollectibleKind::Frame).is_empty());
    }

    #[test]
    fn profile_styles_scope_snapshots_support_arbitrary_guilds() {
        let main = Arc::new(profile());
        let mut card = super::super::ProfileCard::new(
            1,
            None,
            gpui::point(gpui::px(0.), gpui::px(0.)),
            gpui::Anchor::TopLeft,
            super::super::ProfileOrigin::Member,
        );
        card.profile = Some(main.clone());
        card.modal = true;
        assert!(card.switch_to_scope(Some(7)));
        let server_seven = Arc::new(profile());
        card.profile = Some(server_seven.clone());
        assert!(card.switch_to_scope(Some(8)));
        let server_eight = Arc::new(profile());
        card.profile = Some(server_eight.clone());
        assert!(card.switch_to_scope(None));
        assert!(Arc::ptr_eq(card.profile.as_ref().unwrap(), &main));
        assert!(card.switch_to_scope(Some(7)));
        assert!(Arc::ptr_eq(card.profile.as_ref().unwrap(), &server_seven));
        assert!(card.switch_to_scope(Some(8)));
        assert!(Arc::ptr_eq(card.profile.as_ref().unwrap(), &server_eight));
        assert!(card.modal);
        assert!(!card.switch_to_scope(Some(8)));
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn decode_data_uri(uri: &str) -> image::DynamicImage {
        let bytes = STANDARD.decode(uri.split_once(',').unwrap().1).unwrap();
        image::load_from_memory(&bytes).unwrap()
    }

    fn animated_webp_bytes() -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], bytes: &[u8]) {
            out.extend_from_slice(tag);
            out.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
            out.extend_from_slice(bytes);
            if bytes.len() % 2 != 0 {
                out.push(0);
            }
        }
        // The image crate's WebP encoder is still-only; wrap its lossless frames in an animation.
        let mut chunks = Vec::new();
        chunk(&mut chunks, b"VP8X", &[0x12, 0, 0, 0, 31, 0, 0, 15, 0, 0]);
        chunk(&mut chunks, b"ANIM", &[0, 0, 0, 0, 3, 0]);
        for (color, delay) in [([255, 0, 0, 255], 40), ([0, 0, 255, 255], 90)] {
            let mut still = Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(32, 16, image::Rgba(color))
                .write_to(&mut still, ImageFormat::WebP)
                .unwrap();
            let mut frame = vec![0, 0, 0, 0, 0, 0, 31, 0, 0, 15, 0, 0, delay, 0, 0, 2];
            frame.extend_from_slice(&still.into_inner()[12..]);
            chunk(&mut chunks, b"ANMF", &frame);
        }
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&u32::try_from(chunks.len() + 4).unwrap().to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(&chunks);
        bytes
    }

    #[test]
    fn profile_styles_animated_webp_becomes_gif_without_losing_frames_or_delays() {
        let bytes = animated_webp_bytes();
        for target in [ImageTarget::Avatar, ImageTarget::Banner] {
            let uri = encode_style_image(&bytes, target, true).unwrap();
            assert!(uri.starts_with("data:image/gif;base64,"));
            let gif = STANDARD.decode(uri.split_once(',').unwrap().1).unwrap();
            let decoder = GifDecoder::new(Cursor::new(gif)).unwrap();
            assert!(
                matches!(decoder.loop_count(), image::metadata::LoopCount::Finite(count) if count.get() == 3)
            );
            let frames = decoder.into_frames().collect_frames().unwrap();
            assert_eq!(frames.len(), 2);
            assert_eq!(frames[0].delay(), image::Delay::from_numer_denom_ms(40, 1));
            assert_eq!(frames[1].delay(), image::Delay::from_numer_denom_ms(90, 1));
            assert_eq!(frames[0].buffer().get_pixel(0, 0).0, [255, 0, 0, 255]);
            assert_eq!(frames[1].buffer().get_pixel(0, 0).0, [0, 0, 255, 255]);
        }
        let still = encode_style_image(&bytes, ImageTarget::Avatar, false).unwrap();
        assert!(still.starts_with("data:image/png;base64,"));
        assert_eq!(
            decode_data_uri(&still).to_rgba8().get_pixel(0, 0).0,
            [255, 0, 0, 255]
        );
    }

    #[test]
    fn profile_styles_webp_conversion_rejects_oversized_output_and_invalid_input() {
        let bytes = animated_webp_bytes();
        assert_eq!(
            webp_style_gif(&bytes, 32).unwrap_err(),
            "This GIF exceeds the 10 MB upload limit after conversion."
        );
        assert!(webp_style_gif(b"not a webp", STYLE_IMAGE_LIMIT as usize).is_err());
        let mut still = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut still, ImageFormat::WebP)
            .unwrap();
        assert!(
            webp_style_gif(&still.into_inner(), STYLE_IMAGE_LIMIT as usize)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn profile_styles_gif_writer_enforces_the_exclusive_limit() {
        let mut writer = StyleGifWriter {
            bytes: Vec::new(),
            limit: 8,
            overflowed: false,
        };
        writer.write_all(b"1234567").unwrap();
        assert_eq!(
            writer.write_all(b"8").unwrap_err().kind(),
            io::ErrorKind::FileTooLarge
        );
        assert_eq!(writer.bytes, b"1234567");
        assert!(writer.overflowed);
        writer.flush().unwrap();
        assert!(
            encode_style_image(
                &vec![0; STYLE_IMAGE_LIMIT as usize],
                ImageTarget::Avatar,
                true
            )
            .unwrap_err()
            .contains("10 MB")
        );
    }

    #[test]
    fn profile_styles_cached_gif_uses_the_file_upload_pipeline() {
        let path = std::env::temp_dir().join(format!(
            "zcord-style-gif-{}-{:016x}.webp",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::write(&path, animated_webp_bytes()).unwrap();
        let image = prepare_style_image(path.clone(), ImageTarget::Avatar, true);
        std::fs::remove_file(&path).unwrap();
        let ImageChange::Upload {
            path: uploaded_path,
            data_uri,
        } = image.unwrap()
        else {
            panic!("expected an upload");
        };
        assert_eq!(uploaded_path, path);
        assert!(data_uri.starts_with("data:image/gif;base64,"));
        let bytes = STANDARD
            .decode(data_uri.split_once(',').unwrap().1)
            .unwrap();
        assert_eq!(
            GifDecoder::new(Cursor::new(bytes))
                .unwrap()
                .into_frames()
                .collect_frames()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn profile_styles_images_crop_and_only_resize_down() {
        let avatar =
            encode_style_image(&png_bytes(1200, 1400), ImageTarget::Avatar, false).unwrap();
        let avatar = decode_data_uri(&avatar);
        assert_eq!((avatar.width(), avatar.height()), (1024, 1024));
        let banner = encode_style_image(&png_bytes(340, 240), ImageTarget::Banner, false).unwrap();
        let banner = decode_data_uri(&banner);
        assert_eq!((banner.width(), banner.height()), (340, 120));
        let banner = encode_style_image(&png_bytes(2550, 900), ImageTarget::Banner, false).unwrap();
        let banner = decode_data_uri(&banner);
        assert!(banner.width() <= 2400 && banner.height() <= 848);
    }

    #[test]
    fn profile_styles_animated_gif_passthrough_requires_nitro() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for color in [[1, 2, 3, 255], [4, 5, 6, 255]] {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        32,
                        16,
                        image::Rgba(color),
                    )))
                    .unwrap();
            }
        }
        let animated = encode_style_image(&bytes, ImageTarget::Avatar, true).unwrap();
        assert!(animated.starts_with("data:image/gif;base64,"));
        assert_eq!(
            STANDARD
                .decode(animated.split_once(',').unwrap().1)
                .unwrap(),
            bytes
        );
        let still = encode_style_image(&bytes, ImageTarget::Avatar, false).unwrap();
        assert!(still.starts_with("data:image/png;base64,"));
        let still = decode_data_uri(&still);
        assert_eq!((still.width(), still.height()), (16, 16));
    }
}
