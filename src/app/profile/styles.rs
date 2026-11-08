//! State and actions behind the profile styles panel. The panel and its dialogs
//! (`styles_panel`) only read this state and call these actions.

mod actions;
mod encoding;
mod preview;
mod values;

pub(crate) use encoding::StyleCrop;
pub(super) use encoding::{
    decode_style_still, encode_style_bytes, image_limits, read_style_file, style_data_uri,
};
use preview::apply_preview;

use std::{
    collections::HashSet,
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
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

use super::image_editor::{ImageEdit, MAX_ZOOM, STAGE, draft_previews};
use super::styles_panel::StyleDialog;
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
    /// A local file, already validated and encoded for upload. `preview` shows what will be
    /// uploaded (the editor's cut), while `path` names the file the user picked.
    Upload {
        path: PathBuf,
        preview: PathBuf,
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

    /// Tells this editor apart from one opened later, for tasks that outlive it.
    pub(super) fn identity(&self) -> Arc<()> {
        self.identity.clone()
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
        if let Some(widgets) = self.ui.widgets.draft() {
            Arc::make_mut(&mut profile).widgets = widgets.clone();
        }
        Some(profile)
    }
}

#[cfg(test)]
mod tests;
