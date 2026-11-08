use super::panel::panel_header;
use super::tiles::{
    banner_color, banner_image, effect_thumbnail, frame_thumbnail, nameplate_tile, theme_tile,
};
use super::*;

const PANEL_RADIUS: f32 = 16.;
const PANEL_PADDING: f32 = 16.;

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
pub(super) const TILE_GROUP: &str = "styles-tile";

pub(super) fn tile(id: &'static str, width: f32, height: f32) -> Stateful<Div> {
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
pub(super) fn removable<T>(draft: &Option<Option<T>>, saved: &Option<T>) -> bool {
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
