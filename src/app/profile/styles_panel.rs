use std::path::PathBuf;

use super::image_editor::ImageEdit;
use super::styles::{
    CollectibleKind, ImageChange, ImageTarget, NameStyle, StyleDraft, StyleScope, StylesEditor,
};
use super::*;
use crate::{
    app::{
        display_name::{self, Surface},
        member_list::rounded_nameplate_layer,
    },
    store::{Decoration, NameStyle as ShownNameStyle, Nameplate},
};

pub(super) const PANEL_WIDTH: f32 = 242.;
const PANEL_RADIUS: f32 = 16.;
const PANEL_PADDING: f32 = 16.;
const TILE: f32 = 100.;
const BAR_TILE_HEIGHT: f32 = 42.;
const TILE_RADIUS: f32 = 8.;
const SAVE_BAR_WIDTH: f32 = 500.;
const SCOPE_MENU_WIDTH: f32 = 262.;
const SCOPE_MENU_HEIGHT: f32 = 208.;

#[derive(Default)]
pub(crate) struct PanelUi {
    pub(crate) scope_menu: bool,
    pub(crate) dialog: Option<StyleDialog>,
    /// Edits live: theme colors go straight to the draft, a name color to the open dialog.
    pub(crate) color_picker: Option<ColorPicker>,
    pub(crate) picker_bounds: PickerBounds,
    /// Scroll position of the open collectible dialog's list.
    pub(crate) collectible_list: Option<ListState>,
    pub(crate) nameplate_hover: HoverFade,
    /// The dialog collectible under the pointer, which plays its animation.
    pub(crate) hovered_collectible: Option<u64>,
    /// When the effect dialog's preview started playing, and for which effect.
    pub(crate) preview_effect: (Cell<Option<u64>>, RefCell<Option<Instant>>),
    /// Show/Hide Styles presses, which restart the panel's slide.
    pub(crate) panel_toggles: u64,
    /// Bumped on every open, so each dialog pops in afresh.
    pub(crate) dialog_opens: u64,
    /// Set while the dialog pops out; `dialog` clears once it finishes.
    pub(crate) dialog_closing: Option<Task<()>>,
    /// The dialog replaced one still on screen, so the scrim is already up.
    pub(crate) dialog_swapped: bool,
    pub(crate) name_preview_light: bool,
    pub(crate) widgets: super::widget_editor::WidgetsUi,
    pub(crate) identity: super::identity_editor::IdentityUi,
    /// The theme color picker's suggestions and the avatar they came from; `None` while the
    /// avatar is still being sampled.
    pub(crate) theme_suggestions: Option<(AvatarSource, Option<[u32; 5]>)>,
}

/// `Shown`'s avatar: a CDN image, or a picked file not yet uploaded.
type AvatarSource = (Option<ImageSpec>, Option<PathBuf>);

/// The open dialog and its working copy, applied to the draft only on Apply.
#[derive(Clone)]
pub(crate) enum StyleDialog {
    Collectible {
        kind: CollectibleKind,
        /// `None` is "Use Default".
        selected: Option<u64>,
    },
    NameStyle(NameStyle),
    Image(ImageTarget),
    /// Cropping a picked avatar or banner before it goes in the draft.
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
    /// The swatch that opened it, which the picker sits beside.
    pub(crate) anchor: Option<Bounds<Pixels>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorSlot {
    Primary,
    Accent,
    /// The single banner color of profiles without Nitro.
    Banner,
    /// A custom display name color from the name style dialog: the stop it edits in a
    /// Gradient or Prism, otherwise 0.
    Name(usize),
}

/// Where the picker's saturation square and hue bar were last painted, for mapping drags.
#[derive(Clone, Default)]
pub(crate) struct PickerBounds {
    pub(crate) square: Rc<Cell<Option<Bounds<Pixels>>>>,
    pub(crate) hue: Rc<Cell<Option<Bounds<Pixels>>>>,
}

/// What the panel and its dialogs show: the draft over the scope's saved values, falling back
/// to the main profile inside a server.
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
        // Earlier uploads live under the archive, not beside the current avatar.
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

/// The item of `kind` inside an owned product, or the one already on the user when the
/// inventory hasn't loaded.
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

/// Your name as the editor previews it: a rename typed into the profile, else your own.
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

    /// Closes the styles editor's topmost layer for Escape: a picker or menu, then an open
    /// dialog. Returns false when nothing was open, so Escape can close the profile itself.
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

    /// Plays the pop-out, then drops the dialog.
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

    /// Samples the shown avatar for the theme picker's suggestions, once per avatar.
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

    /// Moves the open picker to `hsv` and writes the color where its slot lives.
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
                // Blended effects edit one stop; the rest take a single custom color throughout.
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

fn heading(title: &'static str, nitro: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .h(rpx(18.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .child(title)
        .when(nitro, |this| {
            this.child(theme::icon("nitro", 16., theme::strong()))
        })
}

/// Hovering a tile reveals its remove button.
const TILE_GROUP: &str = "styles-tile";

fn tile(id: &'static str, width: f32, height: f32) -> Stateful<Div> {
    plain_tile(id, width, height).hover(|style| style.bg(rgba(theme::control_selected())))
}

/// A tile without the grey hover fill; gpui allows only one hover style per element.
fn plain_tile(id: &'static str, width: f32, height: f32) -> Stateful<Div> {
    div()
        .id(id)
        .group(TILE_GROUP)
        .relative()
        .w(rpx(width))
        .h(rpx(height))
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .rounded(rpx(TILE_RADIUS))
        .bg(rgba(theme::control_bg()))
        .cursor_pointer()
}

/// Whether the scope sets its own value, as opposed to the default or the main profile's
/// showing through, which a reset would leave in place.
fn removable<T>(draft: &Option<Option<T>>, saved: &Option<T>) -> bool {
    match draft {
        Some(value) => value.is_some(),
        None => saved.is_some(),
    }
}

fn image_removable(draft: &Option<ImageChange>, saved: &Option<SharedString>) -> bool {
    match draft {
        Some(ImageChange::Remove) => false,
        Some(_) => true,
        None => saved.is_some(),
    }
}

/// The trash button in a tile's top-right corner, which resets that style to its default.
/// Hidden until the tile is hovered, as the nameplate's is.
fn remove_button(
    app: &DiscordApp,
    id: &'static str,
    update: impl Fn(&mut StyleDraft) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .absolute()
        .top(rpx(4.))
        .right(rpx(4.))
        .size(rpx(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rpx(6.))
        .bg(rgba(0x00000099))
        .opacity(0.)
        .group_hover(TILE_GROUP, |style| style.opacity(1.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(0x000000CC)))
        .on_click(on_app(app, move |app, cx| {
            app.update_style_draft(|draft| update(draft), cx)
        }))
        .child(theme::icon("trash-2", 16., 0xFFFFFF))
        .child(theme::hover_tooltip(
            SharedString::from(format!("{id}-tooltip")),
            "Remove",
        ))
        .into_any_element()
}

fn group(title: &'static str, nitro: bool, tiles: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .child(heading(title, nitro))
        .child(div().flex().gap(rpx(8.)).children(tiles))
}

pub(super) fn editing_panel(app: &DiscordApp, editor: &StylesEditor, cx: &App) -> AnyElement {
    let shown = shown(app, editor);
    let saved = app.saved_style_values();
    let draft = &editor.draft;
    let user_id = app.store.current_user_id;
    let user = app.store.user(user_id);
    let decoration = shown
        .decoration
        .and_then(|sku| decoration_of(app, editor, sku));
    let nameplate_tile = nameplate_tile(app, editor, &shown, decoration.as_ref(), cx);
    // Without Nitro the theme is the banner color, which only the main profile takes.
    let avatar_tile = tile("styles-avatar", TILE, TILE)
        .on_click(on_app(app, |app, cx| {
            app.open_style_dialog(StyleDialog::Image(ImageTarget::Avatar), cx)
        }))
        .child(theme::preview_avatar(
            user,
            user_id,
            72.,
            None,
            shown.avatar.clone(),
            shown.avatar_file.clone(),
            None,
            cx,
        ))
        .when(image_removable(&draft.avatar, &saved.avatar), |this| {
            this.child(remove_button(app, "styles-avatar-remove", |draft| {
                draft.avatar = Some(ImageChange::Remove)
            }))
        })
        .into_any_element();
    let decoration_tile = tile("styles-decoration", TILE, TILE)
        .on_click(on_app(app, |app, cx| {
            let selected = app
                .profile_styles
                .as_ref()
                .and_then(|editor| self::shown(app, editor).decoration);
            app.open_style_dialog(
                StyleDialog::Collectible {
                    kind: CollectibleKind::Decoration,
                    selected,
                },
                cx,
            );
        }))
        .child(theme::placeholder_avatar(72., decoration.as_ref(), cx))
        .when(removable(&draft.decoration, &saved.decoration), |this| {
            this.child(remove_button(app, "styles-decoration-remove", |draft| {
                draft.decoration = Some(None)
            }))
        })
        .into_any_element();
    let name = own_name(app);
    let name_style = shown.name_style.as_ref().map(shown_name_style);
    let name_tile = tile("styles-name", 208., BAR_TILE_HEIGHT)
        .on_click(on_app(app, |app, cx| app.open_name_style_dialog(cx)))
        .child(
            div()
                .px(rpx(8.))
                .max_w_full()
                .text_size(rpx(24.))
                .line_height(rpx(32.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(display_name::render(
                    "styles-name-preview",
                    &name,
                    name_style.as_ref(),
                    Surface::Account { hovered: true },
                    cx,
                )),
        )
        .when(removable(&draft.name_style, &saved.name_style), |this| {
            this.child(remove_button(app, "styles-name-remove", |draft| {
                draft.name_style = Some(None)
            }))
        })
        .into_any_element();
    let theme_tile = theme_tile(app, editor, &shown);
    let banner_fill = banner_color(editor, &shown);
    // Dimmed on hover below, rather than taking the grey other tiles do.
    let banner_tile = plain_tile("styles-banner", TILE, TILE)
        .on_click(on_app(app, |app, cx| {
            app.open_style_dialog(StyleDialog::Image(ImageTarget::Banner), cx)
        }))
        .bg(rgb(banner_fill))
        .children(banner_image(&shown, cx))
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(rpx(TILE_RADIUS))
                .bg(rgba(0x0000004D))
                .opacity(0.)
                .group_hover(TILE_GROUP, |style| style.opacity(1.)),
        )
        .when(image_removable(&draft.banner, &saved.banner), |this| {
            this.child(remove_button(app, "styles-banner-remove", |draft| {
                draft.banner = Some(ImageChange::Remove)
            }))
        })
        .into_any_element();
    let effect_tile = tile("styles-effect", TILE, TILE)
        .on_click(on_app(app, |app, cx| {
            let selected = app
                .profile_styles
                .as_ref()
                .and_then(|editor| self::shown(app, editor).effect);
            app.open_style_dialog(
                StyleDialog::Collectible {
                    kind: CollectibleKind::Effect,
                    selected,
                },
                cx,
            );
        }))
        // Discord sits the card 12px down and runs it past the tile's bottom edge.
        .items_start()
        .child(
            div()
                .mt(rpx(12.))
                .child(effect_thumbnail(editor, shown.effect, 60., 92., true, cx)),
        )
        .when(removable(&draft.effect, &saved.effect), |this| {
            this.child(remove_button(app, "styles-effect-remove", |draft| {
                draft.effect = Some(None)
            }))
        })
        .into_any_element();
    let frame_tile = tile("styles-frame", TILE, TILE)
        .on_click(on_app(app, |app, cx| {
            let selected = app
                .profile_styles
                .as_ref()
                .and_then(|editor| self::shown(app, editor).frame);
            app.open_style_dialog(
                StyleDialog::Collectible {
                    kind: CollectibleKind::Frame,
                    selected,
                },
                cx,
            );
        }))
        .child(frame_thumbnail(editor, shown.frame, 60., 60., cx))
        .when(removable(&draft.frame, &saved.frame), |this| {
            this.child(remove_button(app, "styles-frame-remove", |draft| {
                draft.frame = Some(None)
            }))
        })
        .into_any_element();
    div()
        .id("profile-styles-panel")
        .without_text_selection()
        .flex()
        .flex_col()
        .flex_none()
        .w(rpx(PANEL_WIDTH))
        .h_full()
        .rounded_l(rpx(PANEL_RADIUS))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::surface_high()))
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(panel_header(app, editor, cx))
        .child(
            div()
                .id("profile-styles-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(rpx(PANEL_PADDING))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(24.))
                        .child(group("Nameplate", false, [nameplate_tile]))
                        .child(group(
                            "Avatar & Decoration",
                            false,
                            [avatar_tile, decoration_tile],
                        ))
                        .child(group("Display Name Style", true, [name_tile]))
                        .child(group("Theme & Banner", true, [theme_tile, banner_tile]))
                        .child(group(
                            "Profile Effect & Frame",
                            false,
                            [effect_tile, frame_tile],
                        ))
                        .children(editor.error.clone().map(|error| {
                            div()
                                .text_size(rpx(12.))
                                .line_height(rpx(16.))
                                .text_color(rgb(theme::RED))
                                .child(error)
                        })),
                ),
        )
        .into_any_element()
}

const HOVER_FADE: Duration = Duration::from_millis(220);

/// How far the nameplate tile has faded from its stand-in row (0) to your member row (1).
#[derive(Default)]
pub(crate) struct HoverFade {
    hovered: bool,
    /// Bumped on every change to restart the animation.
    changes: u64,
    /// The level when the current fade began, so leaving mid-fade reverses from there.
    from: f32,
    started: Option<Instant>,
}

impl HoverFade {
    fn target(&self) -> f32 {
        if self.hovered { 1. } else { 0. }
    }

    /// A partial fade takes the matching share of the full duration.
    fn duration(&self) -> Duration {
        HOVER_FADE.mul_f32((self.target() - self.from).abs())
    }

    fn level(&self, now: Instant) -> f32 {
        let duration = self.duration();
        let Some(started) = self.started.filter(|_| !duration.is_zero()) else {
            return self.target();
        };
        let progress = (now - started).as_secs_f32() / duration.as_secs_f32();
        self.from + (self.target() - self.from) * ease_in_out(progress.min(1.))
    }

    pub(super) fn set(&mut self, hovered: bool) {
        if self.hovered == hovered {
            return;
        }
        let now = Instant::now();
        self.from = self.level(now);
        self.hovered = hovered;
        self.changes += 1;
        self.started = Some(now);
    }

    /// `layer` faded to the hover level, easing there after each change.
    pub(super) fn fade(&self, id: &'static str, layer: Div) -> AnyElement {
        let (from, target, duration) = (self.from, self.target(), self.duration());
        if duration.is_zero() {
            return layer.opacity(target).into_any_element();
        }
        layer
            .with_animation(
                (id, self.changes),
                Animation::new(duration).with_easing(ease_in_out),
                move |layer, delta| layer.opacity(from + (target - from) * delta),
            )
            .into_any_element()
    }
}

/// A stand-in member row that cross-fades to your real one, with a remove button, on hover.
fn nameplate_tile(
    app: &DiscordApp,
    editor: &StylesEditor,
    shown: &Shown,
    decoration: Option<&Decoration>,
    cx: &App,
) -> AnyElement {
    let user_id = app.store.current_user_id;
    let user = app.store.user(user_id);
    let hover = &editor.ui.nameplate_hover;
    let hovered = hover.hovered;
    let has_nameplate = removable(&editor.draft.nameplate, &app.saved_style_values().nameplate);
    let plate = shown
        .nameplate
        .and_then(|sku| nameplate_of(app, editor, sku))
        .and_then(|plate| {
            rounded_nameplate_layer(&plate, user_id, BAR_TILE_HEIGHT, hovered, TILE_RADIUS, cx)
        });
    let (from, target, changes, duration) =
        (hover.from, hover.target(), hover.changes, hover.duration());
    let opacity = move |level: f32, shown_when_hovered: bool| {
        if shown_when_hovered {
            level
        } else {
            1. - level
        }
    };
    let fade = move |layer: Div, id: &'static str, shown_when_hovered: bool| -> AnyElement {
        if duration.is_zero() {
            return layer
                .opacity(opacity(target, shown_when_hovered))
                .into_any_element();
        }
        layer
            .with_animation(
                (id, changes),
                Animation::new(duration).with_easing(ease_in_out),
                move |layer, delta| {
                    layer.opacity(opacity(from + (target - from) * delta, shown_when_hovered))
                },
            )
            .into_any_element()
    };
    let row = || {
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .gap(rpx(12.))
            .px(rpx(8.))
    };
    let stand_in = row().child(theme::placeholder_avatar(32., None, cx)).child(
        div()
            .w(rpx(111.))
            .h(rpx(14.))
            .rounded(rpx(8.))
            .bg(rgb(0x706F74))
            .opacity(0.5),
    );
    let name = own_name(app);
    let name_style = shown.name_style.as_ref().map(shown_name_style);
    let member = row()
        .child(theme::preview_avatar(
            user,
            user_id,
            32.,
            None,
            shown.avatar.clone(),
            shown.avatar_file.clone(),
            decoration,
            cx,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .child(
                    div()
                        .min_w_0()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(display_name::render(
                            "styles-nameplate-name",
                            &name,
                            name_style.as_ref(),
                            Surface::Account { hovered },
                            cx,
                        )),
                )
                .children(
                    user.and_then(|user| user.tag.as_ref())
                        .map(|tag| theme::guild_tag(tag, cx)),
                ),
        )
        .when(has_nameplate, |this| {
            this.child(
                div()
                    .id("styles-nameplate-remove")
                    .relative()
                    .size(rpx(24.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(rpx(6.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgba(0x0000007A)))
                    .on_click(on_app(app, |app, cx| {
                        app.update_style_draft(|draft| draft.nameplate = Some(None), cx)
                    }))
                    .child(theme::icon("trash-2", 16., 0xFFFFFF))
                    .child(theme::hover_tooltip(
                        "styles-nameplate-remove-tooltip",
                        "Remove",
                    )),
            )
        });
    let entity = app.entity.clone();
    tile("styles-nameplate", 208., BAR_TILE_HEIGHT)
        .on_hover(move |hovered, _, cx| {
            let hovered = *hovered;
            entity
                .update(cx, |app, cx| {
                    app.update_styles_ui(|ui| ui.nameplate_hover.set(hovered), cx)
                })
                .ok();
        })
        .on_click(on_app(app, |app, cx| {
            let selected = app
                .profile_styles
                .as_ref()
                .and_then(|editor| self::shown(app, editor).nameplate);
            app.open_style_dialog(
                StyleDialog::Collectible {
                    kind: CollectibleKind::Nameplate,
                    selected,
                },
                cx,
            );
        }))
        .children(plate)
        .child(fade(stand_in, "styles-nameplate-stand-in", false))
        .child(fade(member, "styles-nameplate-member", true))
        .into_any_element()
}

/// Discord's default display name font (gg sans) and plain-color effect.
pub(super) const DEFAULT_FONT: u32 = 11;
pub(super) const SOLID_EFFECT: u32 = 1;

fn banner_image(shown: &Shown, cx: &App) -> Option<AnyElement> {
    if let Some(path) = &shown.banner_file {
        return Some(
            img(path.clone())
                .size_full()
                .rounded(rpx(TILE_RADIUS))
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
        );
    }
    let spec = shown.banner.as_ref()?;
    let path = images::resolve(spec, theme::image_px(TILE * 2.), cx)?;
    // The tile's overflow clip is square, so the picture rounds its own corners.
    Some(
        img(path)
            .size_full()
            .rounded(rpx(TILE_RADIUS))
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
    )
}

/// Records where its parent was last painted, for placing popovers beside it.
pub(super) fn measure_into(cell: Rc<Cell<Option<Bounds<Pixels>>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
        .absolute()
        .size_full()
}

/// What a profile shows where it has no banner image: the theme's primary color.
fn banner_color(editor: &StylesEditor, shown: &Shown) -> u32 {
    if editor.nitro {
        shown.theme_colors.unwrap_or(DEFAULT_THEME)[0]
    } else {
        shown.accent_color.unwrap_or(DEFAULT_THEME[0])
    }
}

fn theme_tile(app: &DiscordApp, editor: &StylesEditor, shown: &Shown) -> AnyElement {
    let swatch = |id: &'static str, slot: ColorSlot, color: u32| {
        let bounds = Rc::new(Cell::new(None));
        div()
            .id(id)
            .relative()
            .size(rpx(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(rpx(8.))
            .cursor_pointer()
            .child(measure_into(bounds.clone()))
            .on_click(on_app(app, move |app, cx| {
                app.open_color_picker(slot, color, bounds.get(), cx)
            }))
            .child(
                div()
                    .size(rpx(23.6))
                    .rounded(rpx(8.))
                    .border(rpx(1.78))
                    .border_color(gpui::white())
                    .bg(rgb(color)),
            )
    };
    let base = div()
        .id("styles-theme")
        .group(TILE_GROUP)
        .relative()
        .w(rpx(TILE))
        .h(rpx(TILE))
        .flex()
        .flex_col()
        .items_center()
        .rounded(rpx(TILE_RADIUS))
        .border_1()
        .border_color(rgba(theme::border_soft()));
    if editor.nitro {
        let [primary, accent] = shown.theme_colors.unwrap_or(DEFAULT_THEME);
        base.justify_between()
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgb(primary), 0.),
                linear_color_stop(rgb(accent), 1.),
            ))
            .child(swatch("styles-primary", ColorSlot::Primary, primary))
            .child(swatch("styles-accent", ColorSlot::Accent, accent))
            .into_any_element()
    } else {
        // Discord only takes a banner color on the main profile.
        let color = shown.accent_color.unwrap_or(DEFAULT_THEME[0]);
        base.justify_center()
            .bg(rgb(color))
            .when(editor.scope == StyleScope::Main, |this| {
                this.child(swatch("styles-banner-color", ColorSlot::Banner, color))
            })
            .into_any_element()
    }
}

/// Discord's starting gradient when no theme is set.
pub(super) const DEFAULT_THEME: [u32; 2] = [0x5865F2, 0x232428];

/// Height over width of profile effect thumbnails, which are all 450×880.
const EFFECT_ASPECT: f32 = 880. / 450.;

/// A miniature profile wearing the effect. With `animate`, it plays the effect's own layers,
/// which are animated, instead of the still thumbnail.
pub(super) fn effect_thumbnail(
    editor: &StylesEditor,
    sku_id: Option<u64>,
    width: f32,
    height: f32,
    animate: bool,
    cx: &App,
) -> AnyElement {
    let item = sku_id.and_then(|sku| collectible_item(editor, sku));
    let pixels = theme::image_px(width.max(height));
    // Effect art is drawn for a whole profile, so it spans the card's width from the top.
    let place = |this: Div| {
        this.absolute()
            .top_0()
            .left_0()
            .w(rpx(width))
            .h(rpx(width * EFFECT_ASPECT))
    };
    let layers = item.filter(|_| animate).and_then(|item| {
        let mut layers: Vec<_> = item.effects.iter().enumerate().collect();
        layers.sort_by_key(|(_, layer)| layer.z_index);
        // Only once every layer has decoded, so they start together.
        layers
            .into_iter()
            .map(|(index, layer)| {
                let spec = ImageSpec::Collectible {
                    url: layer.src.clone(),
                };
                let frames = animated::resolve(&spec, pixels, cx)?;
                Some(
                    place(div()).child(
                        animated::image(frames)
                            .id(SharedString::from(format!(
                                "effect-thumbnail-{}-{index}",
                                sku_id.unwrap_or_default()
                            )))
                            .size_full(),
                    ),
                )
            })
            .collect::<Option<Vec<_>>>()
            .filter(|layers| !layers.is_empty())
    });
    let still = layers.is_none().then(|| {
        item.and_then(|item| {
            item.thumbnail_preview_src
                .clone()
                .or_else(|| item.reduced_motion_src.clone())
        })
        .and_then(|url| images::resolve(&ImageSpec::Collectible { url }, pixels, cx))
        .map(|path| place(div()).child(img(path).size_full()))
    });
    div()
        .relative()
        .w(rpx(width))
        .h(rpx(height))
        .overflow_hidden()
        .rounded(rpx(4.))
        .bg(rgb(theme::base_lowest()))
        .shadow(vec![
            BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., 0.14)).blur_radius(px(4.)),
        ])
        .child(skeleton(width))
        .children(still.flatten())
        .children(layers.into_iter().flatten())
        .into_any_element()
}

/// A greyed-out miniature profile drawn behind effect and frame thumbnails.
fn skeleton(width: f32) -> Div {
    let line = |w: f32| {
        div()
            .w(rpx(w))
            .h(rpx(3.))
            .rounded(rpx(2.))
            .bg(rgb(0x2E2F34))
    };
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .child(div().w_full().h(rpx(width * 0.35)).bg(rgb(0x2E2F34)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(3.))
                .p(rpx(width * 0.1))
                .child(
                    div()
                        .size(rpx(width * 0.3))
                        .mt(rpx(-width * 0.18))
                        .rounded_full()
                        .bg(rgb(0x3F4046)),
                )
                .child(line(width * 0.5))
                .child(line(width * 0.35))
                .child(line(width * 0.6)),
        )
}

pub(super) fn frame_thumbnail(
    editor: &StylesEditor,
    sku_id: Option<u64>,
    width: f32,
    height: f32,
    cx: &App,
) -> AnyElement {
    let frame = sku_id.and_then(|sku| {
        let owned = editor.find_collectible(sku)?;
        let item = owned.product.items.iter().find(|item| item.kind == 3)?;
        Some((owned.product.sku_id, item))
    });
    // Back layers sit behind the card and front layers over it, as on a real profile.
    let layers = |front: bool| -> Vec<AnyElement> {
        frame
            .map(|(product_sku, item)| {
                let scale = width / item.inner_width.unwrap_or(1200).max(1) as f32;
                let overflow_x = item.overflow_horizontal.unwrap_or(0) as f32 * scale;
                let frame_width = width + overflow_x * 2.;
                item.layers
                    .iter()
                    .filter(|layer| (layer.order == "front") == front)
                    .filter(|layer| layer.kind != "border" && layer.kind != "rail")
                    .filter_map(|layer| {
                        let spec = ImageSpec::Collectible {
                            url: profile_frame_layer_url(product_sku, layer.id),
                        };
                        let path = images::resolve(&spec, theme::image_px(frame_width), cx)?;
                        let base = div().absolute().left(rpx(-overflow_x)).w(rpx(frame_width));
                        let base = if layer.anchor == "bottom" {
                            base.bottom(rpx(-(item.overflow_bottom.unwrap_or(0) as f32) * scale))
                        } else {
                            base.top(rpx(-(item.overflow_top.unwrap_or(0) as f32) * scale))
                        };
                        Some(base.child(img(path).w(rpx(frame_width))).into_any_element())
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    div()
        .relative()
        .w(rpx(width))
        .h(rpx(height))
        .children(layers(false))
        .child(
            div()
                .absolute()
                .inset_0()
                .overflow_hidden()
                .rounded(rpx(4.))
                .bg(rgb(theme::base_lowest()))
                .child(skeleton(width)),
        )
        .children(layers(true))
        .into_any_element()
}

fn panel_header(app: &DiscordApp, editor: &StylesEditor, cx: &App) -> AnyElement {
    let label: SharedString = match editor.scope {
        StyleScope::Main => "Main Profile".into(),
        StyleScope::Guild(guild_id) => app
            .store
            .guild_by_id(guild_id)
            .map_or_else(|| "Server".into(), |guild| guild.name.clone().into()),
    };
    let open = editor.ui.scope_menu;
    div()
        .flex()
        .flex_row_reverse()
        .items_center()
        .gap(rpx(4.))
        .h(rpx(49.))
        .px(rpx(8.))
        .py(rpx(4.))
        // Discord rules the scope picker off from the tiles below.
        .border_b_1()
        .border_color(rgba(theme::border_subtle()))
        .child(
            theme::icon_button_with_hover(
                "styles-hide",
                "chevrons-right",
                16.,
                theme::muted(),
                rgba(theme::hover()).into(),
            )
            .size(rpx(32.))
            .rounded(rpx(8.))
            .on_click(on_app(app, |app, cx| app.toggle_styles_panel_animated(cx))),
        )
        .child(
            div().flex_1().min_w_0().py(rpx(4.)).child(
                div()
                    .id("styles-scope")
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .max_w_full()
                    .h(rpx(32.))
                    .pl(rpx(8.))
                    .pr(rpx(4.))
                    .rounded(rpx(8.))
                    .cursor_pointer()
                    .when(open, |this| this.bg(rgba(theme::hover())))
                    .hover(|style| style.bg(rgba(theme::hover())))
                    .on_click(on_app(app, |app, cx| {
                        app.update_styles_ui(|ui| ui.scope_menu = !ui.scope_menu, cx)
                    }))
                    .when(editor.scope != StyleScope::Main, |this| {
                        this.child(scope_icon(app, editor.scope, 20., cx))
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(rpx(16.))
                            .line_height(rpx(20.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::strong()))
                            .child(label),
                    )
                    .child(theme::icon(
                        if open { "chevron-up" } else { "chevron-down" },
                        16.,
                        theme::muted(),
                    )),
            ),
        )
        .into_any_element()
}

fn scope_icon(app: &DiscordApp, scope: StyleScope, size: f32, cx: &App) -> AnyElement {
    let guild = match scope {
        StyleScope::Main => return theme::logo(size).into_any_element(),
        StyleScope::Guild(guild_id) => app.store.guild_by_id(guild_id),
    };
    let Some(guild) = guild else {
        return div().size(rpx(size)).into_any_element();
    };
    if let Some(path) = guild
        .icon_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(size), cx))
    {
        return img(path)
            .size(rpx(size))
            .rounded(rpx(4.))
            .into_any_element();
    }
    div()
        .size(rpx(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rpx(4.))
        .bg(rgb(guild.icon_color))
        .text_size(rpx(size * 0.4))
        .text_color(gpui::black())
        .child(guild.initials.clone())
        .into_any_element()
}

/// The server list under the scope button, drawn over the panel.
pub(super) fn scope_menu(app: &DiscordApp, editor: &StylesEditor, cx: &App) -> Option<AnyElement> {
    if !editor.ui.scope_menu {
        return None;
    }
    let row = |id: ElementId, scope: StyleScope, label: SharedString, icon: AnyElement| {
        let selected = editor.scope == scope;
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(rpx(8.))
            .h(rpx(36.))
            .px(rpx(8.))
            .rounded(rpx(4.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::hover())))
            .on_click(on_app(app, move |app, cx| {
                app.update_styles_ui(|ui| ui.scope_menu = false, cx);
                app.set_style_scope(scope, cx);
            }))
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(label),
            )
            .when(selected, |this| {
                this.child(theme::icon("check", 16., theme::strong()))
            })
    };
    let rows = std::iter::once(
        row(
            "styles-scope-main".into(),
            StyleScope::Main,
            "Main Profile".into(),
            scope_icon(app, StyleScope::Main, 20., cx),
        )
        .into_any_element(),
    )
    .chain(app.store.guilds.iter().map(|guild| {
        row(
            ("styles-scope-guild", guild.id).into(),
            StyleScope::Guild(guild.id),
            guild.name.clone().into(),
            scope_icon(app, StyleScope::Guild(guild.id), 20., cx),
        )
        .into_any_element()
    }));
    Some(
        div()
            .id("styles-scope-menu")
            .without_text_selection()
            .absolute()
            .top(rpx(48.))
            .left(rpx(8.))
            .w(rpx(SCOPE_MENU_WIDTH))
            .max_h(rpx(SCOPE_MENU_HEIGHT))
            .overflow_y_scroll()
            .p(rpx(4.))
            .rounded(rpx(8.))
            .border_1()
            .border_color(rgba(theme::border()))
            .bg(rgb(theme::surface_highest()))
            .shadow(vec![
                BoxShadow::new(px(0.), px(12.), hsla(0., 0., 0., 0.24)).blur_radius(px(24.)),
            ])
            .occlude()
            .on_click(|_, _, cx| cx.stop_propagation())
            .children(rows)
            .into_any_element(),
    )
}

const PANEL_SLIDE: Duration = Duration::from_millis(250);

/// The panel, sliding out from under the card or back behind it. It stays mounted at zero
/// width once hidden, so hiding can animate too.
pub(super) fn sliding_panel(
    app: &DiscordApp,
    editor: &StylesEditor,
    height: f32,
    cx: &App,
) -> Option<AnyElement> {
    let toggles = editor.ui.panel_toggles;
    if !editor.expanded && toggles == 0 {
        return None;
    }
    let expanded = editor.expanded;
    let frame = div()
        .flex_none()
        .h(rpx(height))
        .overflow_hidden()
        .child(editing_panel(app, editor, cx));
    if toggles == 0 {
        return Some(frame.w(rpx(PANEL_WIDTH)).into_any_element());
    }
    Some(
        frame
            .with_animation(
                ("styles-panel-slide", toggles),
                Animation::new(PANEL_SLIDE).with_easing(ease_out_quint()),
                move |frame, delta| {
                    let open = if expanded { delta } else { 1. - delta };
                    frame.w(rpx(PANEL_WIDTH * open)).opacity(open)
                },
            )
            .into_any_element(),
    )
}

/// The tab hanging off the dialog's left edge while the panel is hidden.
pub(super) fn show_styles_tab(
    app: &DiscordApp,
    editor: &StylesEditor,
    background: Background,
) -> AnyElement {
    let toggles = editor.ui.panel_toggles;
    div()
        .id("styles-show")
        .absolute()
        .top(rpx(40.))
        .left(rpx(-18.))
        .w(rpx(32.))
        .h(rpx(36.))
        .flex()
        .items_center()
        .justify_center()
        .pl(rpx(2.))
        .rounded(rpx(12.))
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .bg(background)
        .cursor_pointer()
        .hover(|style| style.opacity(0.85))
        .on_click(on_app(app, |app, cx| app.toggle_styles_panel_animated(cx)))
        .tooltip(|_, cx| cx.new(|_| StylesTooltip).into())
        .child(theme::icon("paintbrush", 16., theme::bright()))
        .with_animation(
            ("styles-show-tab", toggles),
            Animation::new(PANEL_SLIDE).with_easing(ease_out_quint()),
            |tab, delta| tab.opacity(delta),
        )
        .into_any_element()
}

struct StylesTooltip;

impl Render for StylesTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        theme::tooltip_bubble()
            .text_size(rpx(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .child("Show Styles")
    }
}

/// "Don't forget to save your changes!" under the dialog while the styles or widgets have
/// unsaved edits; it saves and resets both.
pub(super) fn save_bar(app: &DiscordApp, editor: &StylesEditor) -> Option<AnyElement> {
    if editor.draft == StyleDraft::default()
        && editor.ui.widgets.draft().is_none()
        && !super::identity_editor::edited(app, editor)
    {
        return None;
    }
    let button = |id: &'static str, label: &'static str, primary: bool| {
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .h(rpx(32.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .border_1()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .map(|this| {
                if primary {
                    this.bg(rgb(theme::BLURPLE))
                        .border_color(rgba(0xFFFFFF14))
                        .text_color(gpui::white())
                        .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
                } else {
                    this.bg(rgba(theme::control_bg()))
                        .border_color(rgba(theme::border_soft()))
                        .text_color(rgb(theme::strong()))
                        .hover(|style| style.bg(rgba(theme::control_selected())))
                }
            })
            .child(label)
    };
    let saving = editor.saving || editor.ui.widgets.saving;
    Some(
        div()
            .id("styles-save-bar")
            .without_text_selection()
            .w(rpx(SAVE_BAR_WIDTH))
            .flex()
            .items_center()
            .justify_between()
            .gap(rpx(32.))
            .py(rpx(8.))
            .pr(rpx(8.))
            .pl(rpx(12.))
            .rounded(rpx(12.))
            .border_1()
            .border_color(rgba(theme::border_subtle()))
            .bg(rgb(theme::surface_highest()))
            .shadow(vec![
                BoxShadow::new(px(0.), px(12.), hsla(0., 0., 0., 0.24)).blur_radius(px(24.)),
            ])
            .occlude()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .text_color(rgb(theme::strong()))
                    .child("Don't forget to save your changes!"),
            )
            .child(
                div()
                    .flex()
                    .gap(rpx(8.))
                    .child(
                        button("styles-reset", "Reset", false)
                            .on_click(on_app(app, |app, cx| app.reset_style_draft(cx))),
                    )
                    .child(
                        button("styles-save", if saving { "Saving…" } else { "Save" }, true)
                            .when(saving, |this| this.opacity(0.5))
                            .on_click(on_app(app, move |app, cx| {
                                if !saving {
                                    app.save_style_draft(cx);
                                    app.save_widget_draft(cx);
                                }
                            })),
                    ),
            )
            .into_any_element(),
    )
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
