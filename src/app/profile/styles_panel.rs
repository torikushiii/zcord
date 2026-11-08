mod editing_panel;
mod panel;
mod tiles;

pub(super) use panel::{save_bar, scope_menu, show_styles_tab, sliding_panel};
pub(crate) use tiles::HoverFade;
use tiles::{DEFAULT_FONT, DEFAULT_THEME};
pub(super) use tiles::{SOLID_EFFECT, measure_into};

use std::path::PathBuf;

use super::image_editor::ImageEdit;
use super::styles::{
    CollectibleKind, ImageChange, ImageTarget, NameStyle, StyleDraft, StyleScope, StylesEditor,
};
use super::*;
use crate::{
    app::{
        display_name::{self, Surface},
        members::member_list::rounded_nameplate_layer,
    },
    store::{Decoration, NameStyle as ShownNameStyle, Nameplate},
};

pub(super) const PANEL_WIDTH: f32 = 242.;
const TILE: f32 = 100.;
const BAR_TILE_HEIGHT: f32 = 42.;
const TILE_RADIUS: f32 = 8.;

#[derive(Default)]
pub(crate) struct PanelUi {
    pub(crate) scope_menu: bool,
    pub(crate) dialog: Option<StyleDialog>,
    pub(crate) color_picker: Option<ColorPicker>,
    pub(crate) picker_bounds: PickerBounds,
    pub(crate) collectible_list: Option<ListState>,
    pub(crate) nameplate_hover: HoverFade,
    pub(crate) hovered_collectible: Option<u64>,
    pub(crate) preview_effect: (Cell<Option<u64>>, RefCell<Option<Instant>>),
    pub(crate) panel_toggles: u64,
    pub(crate) dialog_opens: u64,
    pub(crate) dialog_closing: Option<Task<()>>,
    pub(crate) dialog_swapped: bool,
    pub(crate) name_preview_light: bool,
    pub(crate) widgets: super::widget_editor::WidgetsUi,
    pub(crate) identity: super::identity_editor::IdentityUi,
    pub(crate) theme_suggestions: Option<(AvatarSource, Option<[u32; 5]>)>,
}

type AvatarSource = (Option<ImageSpec>, Option<PathBuf>);

#[derive(Clone)]
pub(crate) enum StyleDialog {
    Collectible {
        kind: CollectibleKind,
        selected: Option<u64>,
    },
    NameStyle(NameStyle),
    Image(ImageTarget),
    ImageEdit(ImageEdit),
}

impl StyleDialog {
    /// Frees the editor's preview from gpui's atlas, which otherwise keeps it until exit.
    fn release(self, cx: &mut App) {
        if let StyleDialog::ImageEdit(edit) = self {
            super::image_editor::drop_image_later(edit.preview, cx);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ColorPicker {
    pub(crate) slot: ColorSlot,
    pub(crate) hsv: [f32; 3],
    pub(crate) anchor: Option<Bounds<Pixels>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorSlot {
    Primary,
    Accent,
    Banner,
    Name(usize),
}

#[derive(Clone, Default)]
pub(crate) struct PickerBounds {
    pub(crate) square: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub(crate) hue: Rc<Cell<Option<Bounds<Pixels>>>>,
}

#[derive(Default)]
pub(super) struct Shown {
    pub avatar: Option<ImageSpec>,
    pub avatar_file: Option<PathBuf>,
    pub banner: Option<ImageSpec>,
    pub banner_file: Option<PathBuf>,
    pub decoration: Option<u64>,
    pub nameplate: Option<u64>,
    pub effect: Option<u64>,
    pub frame: Option<u64>,
    pub name_style: Option<NameStyle>,
    pub theme_colors: Option<[u32; 2]>,
    pub accent_color: Option<u32>,
}

fn layered<T: Clone>(draft: &Option<Option<T>>, saved: Option<T>, main: Option<T>) -> Option<T> {
    match draft {
        Some(value) => value.clone(),
        None => saved,
    }
    .or(main)
}

pub(super) fn shown(app: &DiscordApp, editor: &StylesEditor) -> Shown {
    let user_id = app.store.current_user_id;
    let saved = app.saved_style_values();
    let guild_id = match editor.scope {
        StyleScope::Guild(guild_id) => Some(guild_id),
        StyleScope::Main => None,
    };
    let main = guild_id
        .map(|_| app.main_style_values())
        .unwrap_or_default();
    let draft = &editor.draft;
    let avatar_hash = |hash: SharedString, own: bool| match guild_id.filter(|_| own) {
        Some(guild_id) => ImageSpec::GuildAvatar {
            guild_id,
            user_id,
            hash: hash.to_string(),
        },
        None => ImageSpec::Avatar {
            user_id,
            hash: hash.to_string(),
        },
    };
    let banner_hash = |hash: SharedString, own: bool| match guild_id.filter(|_| own) {
        Some(guild_id) => ImageSpec::GuildMemberBanner {
            guild_id,
            user_id,
            hash: hash.to_string(),
        },
        None => ImageSpec::Banner {
            user_id,
            hash: hash.to_string(),
        },
    };
    let main_avatar = main.avatar.clone().map(|hash| avatar_hash(hash, false));
    let main_banner = main.banner.clone().map(|hash| banner_hash(hash, false));
    let (avatar, avatar_file) = match &draft.avatar {
        Some(ImageChange::Upload { preview, .. }) => (None, Some(preview.clone())),
        Some(ImageChange::Recent { id, storage_hash }) => (
            Some(ImageSpec::ArchivedAvatar {
                user_id,
                avatar_id: *id,
                hash: storage_hash.to_string(),
            }),
            None,
        ),
        Some(ImageChange::Remove) => (main_avatar, None),
        None => (
            saved
                .avatar
                .clone()
                .map(|hash| avatar_hash(hash, true))
                .or(main_avatar),
            None,
        ),
    };
    let (banner, banner_file) = match &draft.banner {
        Some(ImageChange::Upload { preview, .. }) => (None, Some(preview.clone())),
        Some(ImageChange::Remove) => (main_banner, None),
        Some(ImageChange::Recent { .. }) | None => (
            saved
                .banner
                .clone()
                .map(|hash| banner_hash(hash, true))
                .or(main_banner),
            None,
        ),
    };
    Shown {
        avatar,
        avatar_file,
        banner,
        banner_file,
        decoration: layered(&draft.decoration, saved.decoration, main.decoration),
        nameplate: layered(&draft.nameplate, saved.nameplate, main.nameplate),
        effect: layered(&draft.effect, saved.effect, main.effect),
        frame: layered(&draft.frame, saved.frame, main.frame),
        name_style: layered(&draft.name_style, saved.name_style, main.name_style),
        theme_colors: layered(&draft.theme_colors, saved.theme_colors, main.theme_colors),
        accent_color: layered(&draft.accent_color, saved.accent_color, main.accent_color),
    }
}

pub(super) fn collectible_item(
    editor: &StylesEditor,
    sku_id: u64,
) -> Option<&model::CollectibleItem> {
    let owned = editor.find_collectible(sku_id)?;
    owned
        .product
        .items
        .iter()
        .find(|item| item.sku_id == sku_id)
        .or_else(|| owned.product.items.first())
}

pub(super) fn decoration_of(
    app: &DiscordApp,
    editor: &StylesEditor,
    sku_id: u64,
) -> Option<Decoration> {
    let asset = collectible_item(editor, sku_id)
        .and_then(|item| item.asset.clone())
        .or_else(|| {
            app.store
                .user(app.store.current_user_id)?
                .decoration
                .as_ref()
                .filter(|decoration| decoration.sku_id == Some(sku_id))
                .map(|decoration| decoration.asset.clone())
        })?;
    Some(Decoration {
        asset,
        sku_id: Some(sku_id),
    })
}

pub(super) fn nameplate_of(
    app: &DiscordApp,
    editor: &StylesEditor,
    sku_id: u64,
) -> Option<Nameplate> {
    if let Some(item) = collectible_item(editor, sku_id) {
        return Some(Nameplate {
            asset: item.asset.clone()?,
            palette: item.palette.clone(),
            label: item.label.clone(),
        });
    }
    app.store.user(app.store.current_user_id)?.nameplate.clone()
}

pub(super) fn shown_name_style(style: &NameStyle) -> ShownNameStyle {
    ShownNameStyle {
        font_id: Some(style.font_id),
        effect_id: Some(style.effect_id),
        colors: style.colors.clone(),
    }
}

pub(super) fn own_name(app: &DiscordApp) -> String {
    app.profile_styles
        .as_ref()
        .and_then(|editor| editor.ui.identity.display_name.clone())
        .filter(|name| !name.is_empty())
        .or_else(|| {
            app.store
                .user(app.store.current_user_id)
                .map(|user| user.name.clone())
        })
        .unwrap_or_default()
}

impl DiscordApp {
    pub(crate) fn update_styles_ui(
        &mut self,
        update: impl FnOnce(&mut PanelUi),
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = &mut self.profile_styles {
            update(&mut editor.ui);
            cx.notify();
        }
    }

    pub(crate) fn open_name_style_dialog(&mut self, cx: &mut Context<Self>) {
        let style = self
            .profile_styles
            .as_ref()
            .and_then(|editor| shown(self, editor).name_style)
            .unwrap_or(NameStyle {
                font_id: DEFAULT_FONT,
                effect_id: SOLID_EFFECT,
                colors: Vec::new(),
            });
        self.open_style_dialog(StyleDialog::NameStyle(style), cx);
    }

    pub(crate) fn open_style_dialog(&mut self, dialog: StyleDialog, cx: &mut Context<Self>) {
        let collectible = matches!(dialog, StyleDialog::Collectible { .. });
        if collectible {
            self.load_style_shop(cx);
        }
        if matches!(
            dialog,
            StyleDialog::Collectible {
                kind: CollectibleKind::Effect | CollectibleKind::Frame,
                ..
            }
        ) {
            self.start_style_effect_timer(cx);
        }
        if matches!(dialog, StyleDialog::Image(_)) {
            self.preload_gif_categories(cx);
        }
        let (replaced, swapped) = self
            .profile_styles
            .as_mut()
            .map(|editor| {
                let swapped = editor.ui.dialog.is_some() && editor.ui.dialog_closing.is_none();
                (editor.ui.dialog.take(), swapped)
            })
            .unwrap_or_default();
        if let Some(replaced) = replaced {
            replaced.release(cx);
        }
        self.update_styles_ui(
            |ui| {
                ui.scope_menu = false;
                ui.collectible_list =
                    collectible.then(|| ListState::new(0, ListAlignment::Top, px(400.)));
                ui.dialog = Some(dialog);
                ui.dialog_opens += 1;
                ui.dialog_closing = None;
                ui.dialog_swapped = swapped;
            },
            cx,
        );
    }

    pub(crate) fn toggle_styles_panel_animated(&mut self, cx: &mut Context<Self>) {
        self.update_styles_ui(
            |ui| {
                ui.scope_menu = false;
                ui.panel_toggles += 1;
            },
            cx,
        );
        self.toggle_styles_panel(cx);
    }

    pub(crate) fn dismiss_styles_layer(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = &mut self.profile_styles else {
            return false;
        };
        let ui = &mut editor.ui;
        if ui.color_picker.take().is_some()
            || ui.widgets.tag_menu.take().is_some()
            || ui.widgets.manage_menu.take().is_some()
            || std::mem::take(&mut ui.scope_menu)
        {
            cx.notify();
            return true;
        }
        if ui.dialog.is_some() {
            self.close_style_dialog(cx);
            return true;
        }
        false
    }

    pub(crate) fn close_style_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.profile_styles else {
            return;
        };
        let ui = &mut editor.ui;
        if ui.dialog.is_none() || ui.dialog_closing.is_some() {
            return;
        }
        ui.hovered_collectible = None;
        ui.dialog_closing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::app::DIALOG_EXIT)
                .await;
            this.update(cx, |app, cx| {
                let Some(editor) = &mut app.profile_styles else {
                    return;
                };
                editor.ui.dialog_closing = None;
                if let Some(dialog) = editor.ui.dialog.take() {
                    dialog.release(cx);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub(crate) fn apply_style_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self
            .profile_styles
            .as_ref()
            .filter(|editor| editor.ui.dialog_closing.is_none())
            .and_then(|editor| editor.ui.dialog.clone())
        else {
            return;
        };
        self.close_style_dialog(cx);
        self.update_style_draft(
            |draft: &mut StyleDraft| match dialog {
                StyleDialog::Collectible { kind, selected } => {
                    let field = match kind {
                        CollectibleKind::Decoration => &mut draft.decoration,
                        CollectibleKind::Nameplate => &mut draft.nameplate,
                        CollectibleKind::Effect => &mut draft.effect,
                        CollectibleKind::Frame => &mut draft.frame,
                    };
                    *field = Some(selected);
                }
                StyleDialog::NameStyle(style) => draft.name_style = Some(Some(style)),
                StyleDialog::Image(_) | StyleDialog::ImageEdit(_) => {}
            },
            cx,
        );
    }

    pub(crate) fn open_color_picker(
        &mut self,
        slot: ColorSlot,
        color: u32,
        anchor: Option<Bounds<Pixels>>,
        cx: &mut Context<Self>,
    ) {
        self.update_styles_ui(
            |ui| {
                ui.scope_menu = false;
                ui.color_picker = Some(ColorPicker {
                    slot,
                    hsv: rgb_to_hsv(color),
                    anchor,
                });
            },
            cx,
        );
        if !matches!(slot, ColorSlot::Name(_)) {
            self.load_theme_suggestions(cx);
        }
    }

    fn load_theme_suggestions(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &self.profile_styles else {
            return;
        };
        let shown = shown(self, editor);
        let source = (shown.avatar, shown.avatar_file);
        if editor
            .ui
            .theme_suggestions
            .as_ref()
            .is_some_and(|(loaded, _)| *loaded == source)
        {
            return;
        }
        let identity = editor.identity();
        let service = images::download_service(cx);
        let url = source.0.as_ref().and_then(images::avatar_sample_url);
        let file = source.1.clone();
        let request = Tokio::spawn(cx, async move {
            let bytes = match (file, url) {
                (Some(path), _) => tokio::task::spawn_blocking(move || std::fs::read(path))
                    .await
                    .ok()?
                    .ok()?,
                (None, Some(url)) => service?.fetch_original(url.as_str()).await?,
                (None, None) => return None,
            };
            tokio::task::spawn_blocking(move || avatar_colors::suggestions(&bytes))
                .await
                .ok()
                .flatten()
        });
        self.update_styles_ui(|ui| ui.theme_suggestions = Some((source.clone(), None)), cx);
        cx.spawn(async move |this, cx| {
            let colors = request
                .await
                .ok()
                .flatten()
                .unwrap_or_else(super::styles_dialogs::preset_suggestions);
            this.update(cx, |app, cx| {
                let Some(editor) = app
                    .profile_styles
                    .as_mut()
                    .filter(|editor| Arc::ptr_eq(&editor.identity(), &identity))
                else {
                    return;
                };
                if let Some((loaded, slot)) = &mut editor.ui.theme_suggestions
                    && *loaded == source
                {
                    *slot = Some(colors);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn close_color_picker(&mut self, cx: &mut Context<Self>) {
        self.update_styles_ui(|ui| ui.color_picker = None, cx);
    }

    pub(crate) fn pick_style_color(&mut self, hsv: [f32; 3], cx: &mut Context<Self>) {
        let Some(editor) = &mut self.profile_styles else {
            return;
        };
        let Some(picker) = &mut editor.ui.color_picker else {
            return;
        };
        picker.hsv = hsv;
        let slot = picker.slot;
        let color = hsv_to_rgb(hsv);
        if let ColorSlot::Name(index) = slot {
            if let Some(StyleDialog::NameStyle(style)) = &mut editor.ui.dialog {
                match style
                    .colors
                    .get_mut(index)
                    .filter(|_| matches!(style.effect_id, 2 | 7))
                {
                    Some(stop) => *stop = color,
                    None => style.colors = vec![color; style.colors.len().max(1)],
                }
            }
            cx.notify();
            return;
        }
        let Some(current) = self
            .profile_styles
            .as_ref()
            .map(|editor| shown(self, editor))
        else {
            return;
        };
        self.update_style_draft(
            |draft| match slot {
                ColorSlot::Banner => draft.accent_color = Some(Some(color)),
                ColorSlot::Primary | ColorSlot::Accent => {
                    let mut colors = current.theme_colors.unwrap_or(DEFAULT_THEME);
                    colors[usize::from(slot == ColorSlot::Accent)] = color;
                    draft.theme_colors = Some(Some(colors));
                }
                ColorSlot::Name(_) => {}
            },
            cx,
        );
    }
}

/// Every click in the panel and its dialogs stops here, so it never reaches the profile
/// backdrop, whose click closes the profile.
pub(super) fn on_app(
    app: &DiscordApp,
    action: impl Fn(&mut DiscordApp, &mut Context<DiscordApp>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = app.entity.clone();
    move |_, _, cx| {
        cx.stop_propagation();
        entity.update(cx, |app, cx| action(app, cx)).ok();
    }
}

pub(super) fn rgb_to_hsv(color: u32) -> [f32; 3] {
    let [r, g, b] = [16, 8, 0].map(|shift| ((color >> shift) & 0xFF) as f32 / 255.);
    let max = r.max(g).max(b);
    let delta = max - r.min(g).min(b);
    let hue = if delta == 0. {
        0.
    } else if max == r {
        60. * ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / delta + 2.)
    } else {
        60. * ((r - g) / delta + 4.)
    };
    let saturation = if max == 0. { 0. } else { delta / max };
    [hue, saturation, max]
}

pub(super) fn hsv_to_rgb([hue, saturation, value]: [f32; 3]) -> u32 {
    let chroma = value * saturation;
    let x = chroma * (1. - ((hue / 60.).rem_euclid(2.) - 1.).abs());
    let (r, g, b) = match (hue.rem_euclid(360.) / 60.) as u32 {
        0 => (chroma, x, 0.),
        1 => (x, chroma, 0.),
        2 => (0., chroma, x),
        3 => (0., x, chroma),
        4 => (x, 0., chroma),
        _ => (chroma, 0., x),
    };
    let m = value - chroma;
    let channel = |c: f32| ((c + m) * 255.).round().clamp(0., 255.) as u32;
    channel(r) << 16 | channel(g) << 8 | channel(b)
}

#[cfg(test)]
mod tests {
    use super::{hsv_to_rgb, rgb_to_hsv};

    #[test]
    fn hsv_round_trips_discord_swatches() {
        for color in [0xB2563E, 0xD99DA3, 0x5865F2, 0x000000, 0xFFFFFF, 0x2AAE16] {
            assert_eq!(hsv_to_rgb(rgb_to_hsv(color)), color);
        }
    }
}
