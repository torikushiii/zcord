use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};
use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, tooltip::Tooltip};

use super::{ProfileCard, ProfilePalette};
use crate::{
    animated,
    app::{DISCORD_EPOCH_MS, DiscordApp, message_body::label},
    discord::model::{
        Game, IdentityProfile, ListenedTrack, OutboxEntry, OutboxKind, OutboxTrait,
        PublicApplication, SkuPrice, StatValue, WidgetAsset, WidgetData, WidgetGame, WidgetImage,
        WidgetText, Wishlist, WishlistItem,
    },
    images::{self, ImageSpec},
    store::GameActivity,
    theme::{self, rpx},
};

const COVER_WIDTH: f32 = 88.;
const COVER_HEIGHT: f32 = COVER_WIDTH * 4. / 3.;
const COVER_GAP: f32 = 16.;
const ACTIVITY_ART: f32 = 64.;
const ACTIVITY_RADIUS: f32 = 8.;
const CURRENT_ART: f32 = 100.;
const CURRENT_BADGE: f32 = 32.;
const POPOUT_ART: f32 = 60.;
const POPOUT_BADGE: f32 = 24.;
const POPOUT_HERO: f32 = 65.;
const HERO_WIDTH: f32 = 240.;
const HERO_HEIGHT: f32 = 160.;
const TILE_IMAGE: f32 = 48.;
/// Share of the hero's width that fades in from transparent at its left edge.
const HERO_FADE: f32 = 0.6;
/// Detailed game lists collapse to two entries; cover grids show the full collection.
const COLLAPSED_GAMES: usize = 2;
/// Two rows of four, as Discord collapses its cover grids.
const COLLAPSED_COVERS: usize = 8;
const COVERS_PER_ROW: usize = 4;
/// How long a widget's "Show more"/"Show less" takes to grow or shrink.
pub(super) const WIDGET_TOGGLE: Duration = Duration::from_millis(250);
const WISHLIST_GAP: f32 = 16.;
const WISHLIST_BADGE: f32 = 26.;
const SHOP_MENU_ENTER: Duration = Duration::from_millis(150);
const MARATHON_SECONDS: u64 = 3 * 60 * 60;

pub(super) fn has_board(card: &ProfileCard) -> bool {
    card.profile.as_ref().is_some_and(|profile| {
        profile
            .widgets
            .iter()
            .any(|widget| !matches!(widget.data, WidgetData::Unknown))
    })
}

pub(super) fn has_wishlist(card: &ProfileCard) -> bool {
    card.wishlist
        .as_ref()
        .is_some_and(|wishlist| !wishlist.items.is_empty())
}

pub(super) fn board_panel(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    let palette = card.palette();
    let widgets = card.profile.iter().flat_map(|profile| &profile.widgets);
    div()
        .flex()
        .flex_col()
        .gap(rpx(16.))
        .children(widgets.filter_map(|widget| {
            let (title, games) = match &widget.data {
                WidgetData::Application { application_id } => {
                    return Some(application_widget(card, *application_id, palette, cx));
                }
                WidgetData::FavoriteGames { games } => ("Favorite Game", games),
                WidgetData::CurrentGames { games } => ("Games in Rotation", games),
                // Discord shows both of these as cover grids rather than game rows.
                WidgetData::PlayedGames { games } => {
                    return (!games.is_empty()).then(|| {
                        covers_widget(app, card, widget.id, "Games I Like", games, palette, cx)
                    });
                }
                WidgetData::WantToPlayGames { games } => {
                    return (!games.is_empty()).then(|| {
                        covers_widget(app, card, widget.id, "Want to Play", games, palette, cx)
                    });
                }
                WidgetData::Unknown => return None,
            };
            // Discord sets the favorite game's name bolder and larger than other lists.
            let featured = matches!(widget.data, WidgetData::FavoriteGames { .. });
            (!games.is_empty())
                .then(|| games_widget(app, card, widget.id, title, games, featured, palette, cx))
        }))
        .into_any_element()
}

pub(super) fn wishlist_panel(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    let palette = card.palette();
    let Some(wishlist) = card.wishlist.as_deref() else {
        return div().into_any_element();
    };
    div()
        .flex()
        .flex_col()
        .gap(rpx(WISHLIST_GAP))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.text))
                        .child(format!(
                            "{} {}",
                            wishlist.items.len(),
                            if wishlist.items.len() == 1 {
                                "Item"
                            } else {
                                "Items"
                            }
                        )),
                )
                .child(shop_menu(app, card, wishlist, palette, cx)),
        )
        .child(
            div().grid().grid_cols(3).gap(rpx(WISHLIST_GAP)).children(
                wishlist
                    .items
                    .iter()
                    .map(|item| wishlist_tile(wishlist, item, palette, cx)),
            ),
        )
        .into_any_element()
}

fn shop_menu(
    app: &DiscordApp,
    card: &ProfileCard,
    wishlist: &Wishlist,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement {
    let open = card.shop_menu_open;
    let entity = app.entity.clone();
    let button = div()
        .id("profile-wishlist-shop")
        .flex()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(36.))
        .px(rpx(14.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(if open { palette.raised } else { palette.button })
        .hover(move |style| style.bg(palette.raised))
        .text_size(rpx(15.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(palette.strong))
        .child("Browse the Shop")
        .child(theme::icon(
            if open { "x" } else { "chevron-down" },
            16.,
            palette.strong,
        ))
        // The menu closes itself on any outside mouse-down, which runs first; acting on the
        // state from this render keeps a click on the button from reopening it.
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.set_shop_menu(!open, cx))
                .ok();
        });
    div().relative().child(button).when(open, |this| {
        let entity = app.entity.clone();
        let mut entries = vec![(
            SharedString::from("Discord Shop"),
            None,
            "https://discord.com/shop".to_owned(),
        )];
        entries.extend(wishlist.applications.iter().map(|application| {
            let icon = application
                .icon
                .as_deref()
                .and_then(|icon| app_asset(application.id, icon, 20., cx));
            (
                SharedString::from(format!("{} Shop", application.name)),
                Some(icon),
                format!(
                    "https://discord.com/application-directory/{}/store",
                    application.id
                ),
            )
        }));
        let close = entity.clone();
        let menu = div()
            .id("profile-wishlist-shop-menu")
            .flex()
            .flex_col()
            .min_w(rpx(220.))
            .p(rpx(8.))
            .rounded(rpx(8.))
            .bg(rgb(theme::ELEVATED))
            .border_1()
            .border_color(hsla(0., 0., 1., 0.06))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(move |_, _, cx| {
                close
                    .update(cx, |app, cx| app.set_shop_menu(false, cx))
                    .ok();
            })
            .children(
                entries
                    .into_iter()
                    .enumerate()
                    .map(|(index, (name, icon, url))| {
                        let entity = entity.clone();
                        div()
                            .id(("profile-wishlist-shop-entry", index))
                            .flex()
                            .items_center()
                            .gap(rpx(10.))
                            .h(rpx(40.))
                            .px(rpx(10.))
                            .rounded(rpx(4.))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(theme::SELECTED)))
                            .text_size(rpx(15.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::STRONG))
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .size(rpx(20.))
                                    .map(|this| match icon {
                                        None => {
                                            this.child(theme::icon("store", 20., theme::STRONG))
                                        }
                                        Some(Some(path)) => {
                                            this.child(img(path).size_full().rounded(rpx(4.)))
                                        }
                                        Some(None) => {
                                            this.rounded(rpx(4.)).bg(rgb(theme::SELECTED))
                                        }
                                    }),
                            )
                            .child(name)
                            .on_click(move |_, _, cx| {
                                cx.open_url(&url);
                                entity
                                    .update(cx, |app, cx| app.set_shop_menu(false, cx))
                                    .ok();
                            })
                    }),
            );
        // A zero-width anchor at the button's bottom-right corner; the menu hangs left from it.
        this.child(
            div().absolute().top(relative(1.)).right_0().child(
                deferred(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .child(menu.with_animation(
                            "profile-wishlist-shop-menu-enter",
                            Animation::new(SHOP_MENU_ENTER).with_easing(ease_out_quint()),
                            |menu, delta| menu.opacity(delta).mt(rpx(2. + 6. * delta)),
                        )),
                )
                .with_priority(2),
            ),
        )
    })
}

fn wishlist_tile(
    wishlist: &Wishlist,
    item: &WishlistItem,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement {
    // Discord requests the scene at 1280px and the cut-out character at 640px.
    let store_art = |asset: Option<u64>, pixels: u32| {
        let spec = ImageSpec::StoreAsset {
            application_id: item.application_id?,
            asset_id: asset?,
        };
        images::resolve(&spec, pixels, cx)
    };
    // Discord layers the cut-out character over its scene; the thumbnail stands in without them.
    let layers: Vec<_> = match (
        store_art(item.card_background_asset_id, 1280),
        store_art(item.card_image_asset_id, 640),
    ) {
        (None, None) => store_art(item.thumbnail_asset_id, 640)
            .into_iter()
            .collect(),
        (background, card) => background.into_iter().chain(card).collect(),
    };
    let application = item.application_id.and_then(|id| {
        wishlist
            .applications
            .iter()
            .find(|application| application.id == id)
    });
    let badge = application
        .and_then(|application| app_asset(application.id, application.icon.as_deref()?, 20., cx));
    let tip = match (&item.price, item.is_owned) {
        (_, true) => format!("{} · Owned", item.name),
        (Some(price), false) => format!("{} · {}", item.name, sku_price(price)),
        (None, false) => item.name.clone(),
    };
    div()
        .id(("profile-wishlist-item", item.sku_id))
        .relative()
        .w_full()
        .aspect_ratio(1.)
        .rounded(rpx(12.))
        .bg(palette.hover)
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .children(layers.into_iter().map(|path| {
            img(path)
                .absolute()
                .inset_0()
                .size_full()
                .rounded(rpx(12.))
                .object_fit(ObjectFit::Cover)
        }))
        .children(badge.map(|path| {
            div()
                .absolute()
                .top(rpx(8.))
                .left(rpx(8.))
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(WISHLIST_BADGE))
                .rounded(rpx(6.))
                .bg(hsla(0., 0., 0., 0.6))
                .child(img(path).size(rpx(18.)).rounded(rpx(4.)))
        }))
}

fn sku_price(price: &SkuPrice) -> String {
    let amount = price.amount as f64 / 10f64.powi(price.exponent as i32);
    let digits = price.exponent as usize;
    match price.currency.as_str() {
        "usd" => format!("${amount:.digits$}"),
        "eur" => format!("€{amount:.digits$}"),
        "gbp" => format!("£{amount:.digits$}"),
        currency => format!("{amount:.digits$} {}", currency.to_uppercase()),
    }
}

fn widget_card(palette: ProfilePalette) -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(rpx(12.))
        .bg(palette.panel)
        .overflow_hidden()
}

/// A grid of game covers, two rows deep until "Show more", each naming its game on hover.
fn covers_widget(
    app: &DiscordApp,
    card: &ProfileCard,
    widget_id: u64,
    title: &'static str,
    games: &[WidgetGame],
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    let expanded = card.expanded_widgets.contains(&widget_id);
    let cover = |entry: &WidgetGame| {
        let game = card.games.get(&entry.game_id).map(Arc::as_ref);
        let name = game.map_or_else(|| "Unknown game".to_owned(), |game| game.name.clone());
        let key = (widget_id, entry.game_id);
        let hovered = card.hovered_cover == Some(key);
        let entity = app.entity.clone();
        game_cover(game, palette, cx)
            .id(("profile-cover-game", widget_id ^ entry.game_id))
            .relative()
            .on_hover(move |hovered, _, cx| {
                entity
                    .update(cx, |app, cx| app.hover_profile_cover(key, *hovered, cx))
                    .ok();
            })
            .when(hovered, |this| this.child(theme::tooltip_above(name)))
            .into_any_element()
    };
    let (first, rest) = games.split_at(games.len().min(COLLAPSED_COVERS));
    // Rows past the first two grow or shrink on "Show more"/"Show less". Covers are a fixed size,
    // so the full height is known up front. The clip exists only while animating: a clipped box
    // that ends at zero height (or scrolls fully out of view) leaks a 1px line of its images.
    let toggling = card
        .widget_toggles
        .get(&widget_id)
        .filter(|(_, at)| at.elapsed() < WIDGET_TOGGLE)
        .map(|(generation, _)| *generation);
    let rest_grid = || {
        div()
            .flex()
            .flex_wrap()
            .w_full()
            .gap(rpx(COVER_GAP))
            .pt(rpx(COVER_GAP))
            .children(rest.iter().map(cover))
    };
    let rest = match (expanded, toggling) {
        (_, Some(generation)) if !rest.is_empty() => {
            let height = rest.len().div_ceil(COVERS_PER_ROW) as f32 * (COVER_HEIGHT + COVER_GAP);
            Some(
                div()
                    .id(("profile-cover-rest", widget_id))
                    .overflow_hidden()
                    .child(rest_grid())
                    .with_animation(
                        ("profile-cover-toggle", widget_id ^ generation),
                        Animation::new(WIDGET_TOGGLE).with_easing(ease_out_quint()),
                        move |this, delta| {
                            let open = if expanded { delta } else { 1. - delta };
                            this.h(rpx(height * open)).opacity(open)
                        },
                    )
                    .into_any_element(),
            )
        }
        (true, _) => Some(rest_grid().into_any_element()),
        (false, _) => None,
    };
    widget_card(palette)
        .id(("profile-cover-games", widget_id))
        .p(rpx(18.))
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.strong))
                .child(title),
        )
        .child(game_cover_grid(first.iter().map(cover)))
        .children(rest)
        .children(show_more(
            app,
            widget_id,
            games.len() > COLLAPSED_COVERS,
            expanded,
            palette,
        ))
        .into_any_element()
}

fn show_more(
    app: &DiscordApp,
    widget_id: u64,
    needed: bool,
    expanded: bool,
    palette: ProfilePalette,
) -> Option<Stateful<Div>> {
    let entity = app.entity.clone();
    needed.then(|| {
        div()
            .id(("profile-widget-more", widget_id))
            .mt(rpx(16.))
            .cursor_pointer()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.strong))
            .hover(|style| style.underline())
            .child(if expanded { "Show less" } else { "Show more" })
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.toggle_profile_widget(widget_id, cx))
                    .ok();
            })
    })
}

fn game_cover_grid(covers: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .w_full()
        .gap(rpx(COVER_GAP))
        .mt(rpx(12.))
        .children(covers)
}

fn game_cover(
    game: Option<&crate::discord::model::Game>,
    palette: ProfilePalette,
    cx: &App,
) -> Div {
    let cover = game.and_then(|game| cover_art(game, cx));
    div()
        .flex_none()
        .w(rpx(COVER_WIDTH))
        .h(rpx(COVER_HEIGHT))
        .rounded(rpx(8.))
        .bg(palette.hover)
        .shadow_md()
        .children(cover.map(|path| {
            img(path)
                .size_full()
                .rounded(rpx(8.))
                .object_fit(ObjectFit::Cover)
        }))
}

fn cover_art(game: &crate::discord::model::Game, cx: &App) -> Option<Arc<Path>> {
    match (
        &game.media.cover_url,
        &game.media.cover_hash,
        &game.icon_hash,
    ) {
        (Some(url), _, _) => media(url, COVER_HEIGHT, cx),
        (None, Some(hash), _) => {
            let spec = ImageSpec::AppCover {
                application_id: game.id,
                hash: hash.clone(),
            };
            images::resolve(&spec, theme::image_px(COVER_HEIGHT), cx)
        }
        // Without a cover, the square icon is cropped into the portrait slot.
        (None, None, Some(hash)) => app_asset(game.id, hash, COVER_HEIGHT, cx),
        (None, None, None) => None,
    }
}

/// Warm the same static image-cache keys the hidden tabs will use, without rendering them.
pub(super) fn prefetch_images(card: &ProfileCard, current: Option<u64>, cx: &App) {
    for id in card
        .profile
        .iter()
        .flat_map(|profile| super::widget_game_ids(profile))
    {
        if let Some(game) = card.games.get(&id) {
            let _ = cover_art(game, cx);
        }
    }
    for entry in card.outbox.iter().flat_map(|outbox| &outbox.entries) {
        let _ = activity_art(card, entry, cx);
    }
    if let Some(id) = current {
        let _ = current_art(card, id, cx);
    }
    for id in card
        .profile
        .iter()
        .flat_map(|profile| super::widget_application_ids(profile))
    {
        if let Some(icon) = card
            .applications
            .get(&id)
            .and_then(|app| app.icon.as_deref())
        {
            let _ = app_asset(id, icon, 18., cx);
        }
    }
}

fn games_widget(
    app: &DiscordApp,
    card: &ProfileCard,
    widget_id: u64,
    title: &'static str,
    games: &[WidgetGame],
    featured: bool,
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    let expanded = card.expanded_widgets.contains(&widget_id);
    let shown = if expanded {
        games.len()
    } else {
        games.len().min(COLLAPSED_GAMES)
    };
    let toggle = show_more(
        app,
        widget_id,
        games.len() > COLLAPSED_GAMES,
        expanded,
        palette,
    );
    widget_card(palette)
        .p(rpx(18.))
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.strong))
                .child(title),
        )
        .child(div().flex().flex_col().gap(rpx(16.)).mt(rpx(16.)).children(
            games[..shown].iter().enumerate().map(|(index, game)| {
                let row = game_row(card, widget_id, game, featured, palette, cx);
                // Rows vary in height, so newly shown ones fade and slide in rather than
                // growing to a precomputed height like the cover grids.
                match card.widget_toggles.get(&widget_id) {
                    Some((generation, at))
                        if expanded && index >= COLLAPSED_GAMES && at.elapsed() < WIDGET_TOGGLE =>
                    {
                        div()
                            .id(("profile-game-row", widget_id ^ game.game_id))
                            .child(row)
                            .with_animation(
                                ("profile-game-row-in", widget_id ^ generation ^ index as u64),
                                Animation::new(WIDGET_TOGGLE).with_easing(ease_out_quint()),
                                |this, delta| this.opacity(delta).mt(rpx(-8. * (1. - delta))),
                            )
                            .into_any_element()
                    }
                    _ => row.into_any_element(),
                }
            }),
        ))
        .children(toggle)
        .into_any_element()
}

fn game_row(
    card: &ProfileCard,
    widget_id: u64,
    entry: &WidgetGame,
    featured: bool,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement {
    let game = card.games.get(&entry.game_id);
    let key = widget_id ^ entry.game_id;
    div()
        .flex()
        .items_center()
        .gap(rpx(20.))
        .child(game_cover(game.map(Arc::as_ref), palette, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(8.))
                .child(
                    div()
                        .map(|this| {
                            if featured {
                                this.text_size(rpx(16.))
                                    .line_height(rpx(20.))
                                    .font_weight(FontWeight::SEMIBOLD)
                            } else {
                                this.text_size(rpx(14.)).line_height(rpx(18.))
                            }
                        })
                        .text_color(rgb(palette.strong))
                        .child(label(
                            ("profile-game", key),
                            game.map_or("Unknown game", |game| game.name.as_str()),
                        )),
                )
                .children(
                    entry
                        .comment
                        .as_deref()
                        .filter(|c| !c.is_empty())
                        .map(|comment| {
                            div()
                                .flex()
                                .items_center()
                                .gap(rpx(6.))
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .text_color(rgb(palette.muted))
                                .child(theme::icon("quote", 14., palette.muted))
                                .child(
                                    div()
                                        .min_w_0()
                                        .child(label(("profile-game-comment", key), comment)),
                                )
                        }),
                )
                .when(!entry.tags.is_empty(), |this| {
                    this.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(rpx(6.))
                            .children(entry.tags.iter().map(|tag| tag_pill(tag, palette))),
                    )
                }),
        )
}

fn tag_pill(tag: &str, palette: ProfilePalette) -> impl IntoElement {
    let icon = match tag {
        "open_to_play" | "open_to_teach" | "looking_for_group" | "lfg" | "looking_for_tips"
        | "looking_to_discuss" | "looking_to_learn" => "users",
        "love_it" | "obsessed" | "kind_of_love_it" | "like_it" | "too_easy" => "thumbs-up",
        "kind_of_hate_it" | "rage_quitting" | "frustrated" => "thumbs-down",
        _ => "award",
    };
    div()
        .flex()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(26.))
        .px(rpx(8.))
        .rounded(rpx(8.))
        .bg(palette.panel)
        .text_size(rpx(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.text))
        .child(theme::icon(icon, 16., palette.text))
        .child(tag_label(tag))
}

/// Discord's tag ids read as sentence case ("better_than_you" → "Better than you").
fn tag_label(tag: &str) -> String {
    if tag == "lfg" {
        return "LFG".to_owned();
    }
    let mut text = tag.replace('_', " ");
    if let Some(first) = text.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    text
}

struct Subtitle {
    label: Option<String>,
    text: String,
    icon: Option<AnyElement>,
}

struct Tile {
    name: Option<String>,
    description: Option<String>,
    image: Option<AnyElement>,
}

fn application_widget(
    card: &ProfileCard,
    application_id: u64,
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    let application = card.applications.get(&application_id);
    let config = card.widget_configs.get(&application_id);
    let profile = card
        .identities
        .iter()
        .flat_map(|identities| &identities.identities)
        .find(|identity| identity.application_id == application_id)
        .and_then(|identity| identity.profile.as_ref());
    let icon = application
        .and_then(|app| app.icon.as_deref())
        .and_then(|hash| app_asset(application_id, hash, 18., cx));
    let name = application
        .map(|app| app.name.clone())
        .or_else(|| config.and_then(|config| config.display_name.clone()))
        .unwrap_or_default();
    let stats = profile
        .map(|profile| profile.stats.as_slice())
        .unwrap_or_default();
    let username = profile.and_then(|profile| profile.username.clone());
    let resolve = |text: &WidgetText| match text {
        WidgetText::Literal(text) => Some(text.clone()),
        WidgetText::Data(key) if key == "username" => username.clone(),
        WidgetText::Data(key) => stats
            .iter()
            .find(|stat| &stat.name == key)
            .map(|stat| stat_value(&stat.value)),
    };
    let art = |asset: &WidgetAsset, size: f32, radius: f32| {
        widget_art(application_id, asset, size, radius, cx)
    };
    let (title, subtitles, grid, tiles) = match config {
        Some(config) => (
            config.title.as_ref().and_then(resolve).or(username.clone()),
            config
                .subtitles
                .iter()
                .filter_map(|subtitle| {
                    Some(Subtitle {
                        label: subtitle.label.as_ref().and_then(resolve),
                        text: resolve(&subtitle.text)?,
                        icon: subtitle.icon.as_ref().and_then(|icon| art(icon, 18., 0.)),
                    })
                })
                .collect(),
            config
                .stats
                .iter()
                .filter_map(|stat| Some((resolve(&stat.value)?, resolve(&stat.label)?)))
                .collect(),
            config
                .collection
                .iter()
                .map(|item| Tile {
                    name: item.name.as_ref().and_then(resolve),
                    description: item.description.as_ref().and_then(resolve),
                    image: item
                        .image
                        .as_ref()
                        .and_then(|image| art(image, TILE_IMAGE, 8.)),
                })
                .collect(),
        ),
        // Without the app's widget config, show the raw stats: the first text one (a region or
        // server) under the username and the rest in the grid.
        None => {
            let subtitle = stats
                .iter()
                .position(|stat| matches!(stat.value, StatValue::Text(_)));
            (
                username.clone(),
                subtitle
                    .map(|index| Subtitle {
                        label: None,
                        text: stat_value(&stats[index].value),
                        icon: None,
                    })
                    .into_iter()
                    .collect::<Vec<_>>(),
                stats
                    .iter()
                    .enumerate()
                    .filter(|(index, stat)| {
                        Some(*index) != subtitle && !matches!(stat.value, StatValue::Image { .. })
                    })
                    .map(|(_, stat)| (stat_value(&stat.value), stat_name(&stat.name)))
                    .collect::<Vec<_>>(),
                Vec::new(),
            )
        }
    };
    // Game widgets point the hero at identity data (a featured character's art) rather than an
    // uploaded asset; both fill the same slot.
    let hero = config
        .and_then(|config| config.hero.as_ref())
        .and_then(|hero| widget_hero(application_id, hero, profile, cx));
    let key = application_id;
    widget_card(palette)
        .child(
            div()
                .relative()
                .min_h(rpx(HERO_HEIGHT))
                .p(rpx(18.))
                .children(hero)
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap(rpx(6.))
                        .children(
                            icon.map(|path| img(path).size(rpx(18.)).flex_none().rounded(rpx(4.))),
                        )
                        .child(
                            div()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .text_color(rgb(palette.strong))
                                .child(label(("profile-app", key), &name)),
                        ),
                )
                .children(title.map(|title| {
                    div()
                        .relative()
                        .mt(rpx(20.))
                        .text_size(rpx(20.))
                        .line_height(rpx(24.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.strong))
                        .child(label(("profile-app-user", key), &title))
                }))
                .children(subtitles.into_iter().enumerate().map(|(index, subtitle)| {
                    let text = match &subtitle.label {
                        Some(label) => format!("{label}: {}", subtitle.text),
                        None => subtitle.text,
                    };
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap(rpx(4.))
                        .mt(rpx(4.))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(palette.text))
                        .child(label(("profile-app-subtitle", key ^ index as u64), &text))
                        .children(
                            subtitle
                                .icon
                                .map(|icon| div().size(rpx(18.)).flex_none().child(icon)),
                        )
                })),
        )
        .when(!grid.is_empty() || !tiles.is_empty(), |this| {
            this.child(div().h(px(1.)).bg(palette.border))
        })
        .when(!grid.is_empty(), |this| {
            this.child(div().flex().flex_col().gap(rpx(20.)).p(rpx(18.)).children(
                grid.chunks(3).map(|row| {
                    div().flex().gap(rpx(8.)).children((0..3).map(|column| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .children(row.get(column).map(|(value, name)| {
                                div()
                                    .child(
                                        div()
                                            .text_size(rpx(14.))
                                            .line_height(rpx(18.))
                                            .text_color(rgb(palette.strong))
                                            .child(value.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(rpx(12.))
                                            .line_height(rpx(16.))
                                            .text_color(rgb(palette.muted))
                                            .child(name.clone()),
                                    )
                            }))
                    }))
                }),
            ))
        })
        .when(!tiles.is_empty(), |this| {
            this.child(collection(key, tiles, palette))
        })
        .into_any_element()
}

/// The URL of an identity-data image: an image stat, or a text stat some apps fill with a URL.
fn data_image(
    profile: Option<&IdentityProfile>,
    key: &str,
) -> Option<(String, Option<u32>, Option<u32>)> {
    let profile = profile?;
    if let Some((url, width, height)) = profile.image(key) {
        return Some((url.to_owned(), width, height));
    }
    profile.stats.iter().find_map(|stat| match &stat.value {
        StatValue::Text(url) if stat.name == key && url.starts_with("https://") => {
            Some((url.clone(), None, None))
        }
        _ => None,
    })
}

/// Art for a widget image field. Data images fall back to the field's `fallback` when the account
/// has no such data, as Example Team Game' default hero does.
fn widget_image_art(
    application_id: u64,
    image: &WidgetImage,
    profile: Option<&IdentityProfile>,
    size: f32,
    radius: f32,
    id: &'static str,
    cx: &App,
) -> Option<AnyElement> {
    match image {
        WidgetImage::Asset(asset) => widget_art(application_id, asset, size, radius, cx),
        WidgetImage::Data { key, fallback } => data_image(profile, key)
            .and_then(|(url, _, _)| presence_art(&url, size, radius, (id, application_id), cx))
            .or_else(|| {
                let fallback = fallback.as_deref()?;
                widget_image_art(application_id, fallback, profile, size, radius, id, cx)
            }),
    }
}

/// Discord pins the hero art to the card's top-right, fills the header's height and fades it
/// in from the left. gpui's `ObjectFit::Cover` always centres and there's no image mask, so the
/// art is sized by hand and painted under an edge fade.
fn widget_hero(
    application_id: u64,
    image: &WidgetImage,
    profile: Option<&IdentityProfile>,
    cx: &App,
) -> Option<AnyElement> {
    match image {
        WidgetImage::Asset(asset) => {
            let aspect = hero_aspect(Some(asset.width), Some(asset.height));
            let art = widget_art(application_id, asset, hero_longest(aspect), 0., cx)?;
            Some(hero_art(art, aspect))
        }
        WidgetImage::Data { key, fallback } => data_image(profile, key)
            .and_then(|(url, width, height)| {
                let aspect = hero_aspect(width, height);
                let id = ("profile-widget-hero", application_id);
                let art = presence_art(&url, hero_longest(aspect), 0., id, cx)?;
                Some(hero_art(art, aspect))
            })
            .or_else(|| widget_hero(application_id, fallback.as_deref()?, profile, cx)),
    }
}

fn hero_aspect(width: Option<u32>, height: Option<u32>) -> f32 {
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => width as f32 / height as f32,
        _ => 1.,
    }
}

/// The size to request the hero art at so it still covers the slot once fitted.
fn hero_longest(aspect: f32) -> f32 {
    HERO_WIDTH.max(HERO_HEIGHT * aspect).max(HERO_HEIGHT)
}

fn hero_art(art: AnyElement, aspect: f32) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .right_0()
        .h_full()
        .w(rpx(HERO_WIDTH))
        .overflow_hidden()
        .child(
            canvas(
                move |bounds, window, cx| {
                    let width = bounds.size.width.max(bounds.size.height * aspect);
                    let fitted = size(width, width / aspect);
                    let origin = point(
                        bounds.origin.x + (bounds.size.width - width) / 2.,
                        bounds.origin.y,
                    );
                    let mut art = div()
                        .w(fitted.width)
                        .h(fitted.height)
                        .child(art)
                        .into_any_element();
                    art.prepaint_as_root(origin, fitted.map(AvailableSpace::Definite), window, cx);
                    art
                },
                |bounds, mut art, window, cx| {
                    // gpui rounds an empty clip out to a 1px strip, which would leak a line
                    // of the art once the card scrolls fully out of view.
                    if window.content_mask().bounds.is_empty() {
                        return;
                    }
                    let fade = EdgeFade {
                        bounds,
                        band: bounds.size.width * HERO_FADE,
                        band_top: None,
                        band_bottom: None,
                        band_left: None,
                        band_right: None,
                        top: false,
                        bottom: false,
                        left: true,
                        right: false,
                    };
                    window.with_edge_fade(Some(fade), |window| art.paint(window, cx));
                },
            )
            .size_full(),
        )
        .into_any_element()
}

fn collection(key: u64, tiles: Vec<Tile>, palette: ProfilePalette) -> impl IntoElement {
    let mut tiles = tiles.into_iter().enumerate();
    let mut rows = Vec::new();
    while let Some(first) = tiles.next() {
        let second = tiles.next();
        rows.push(
            div()
                .flex()
                .gap(rpx(16.))
                .children([Some(first), second].map(|tile| {
                    div().flex_1().min_w_0().children(tile.map(|(index, tile)| {
                        let key = key ^ (index as u64 + 1);
                        div()
                            .flex()
                            .items_center()
                            .gap(rpx(12.))
                            .child(
                                div()
                                    .flex_none()
                                    .size(rpx(TILE_IMAGE))
                                    .rounded(rpx(8.))
                                    .bg(palette.hover)
                                    .children(tile.image),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .children(tile.name.map(|name| {
                                        div()
                                            .truncate()
                                            .text_size(rpx(14.))
                                            .line_height(rpx(18.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette.strong))
                                            .child(label(("profile-app-tile", key), &name))
                                    }))
                                    .children(tile.description.map(|description| {
                                        div()
                                            .truncate()
                                            .text_size(rpx(12.))
                                            .line_height(rpx(16.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(palette.text))
                                            .child(label(
                                                ("profile-app-tile-text", key),
                                                &description,
                                            ))
                                    })),
                            )
                    }))
                })),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(rpx(16.))
        .p(rpx(18.))
        .children(rows)
}

fn stat_value(value: &StatValue) -> String {
    match value {
        StatValue::Text(text) => text.clone(),
        StatValue::Image { url, .. } => url.clone(),
        StatValue::Number(number) if number.fract() == 0. => format!("{number:.0}"),
        StatValue::Number(number) => number.to_string(),
    }
}

/// Readable fallback for raw stat keys ("achievement_num") when an app has no widget config.
fn stat_name(name: &str) -> String {
    let name = name
        .strip_suffix("_num")
        .or_else(|| name.strip_suffix("_count"))
        .unwrap_or(name);
    tag_label(name)
}

fn app_asset(application_id: u64, hash: &str, size: f32, cx: &App) -> Option<Arc<Path>> {
    let spec = ImageSpec::AppImage {
        application_id,
        hash: hash.to_owned(),
        uploaded: false,
        animated: false,
    };
    images::resolve(&spec, theme::image_px(size), cx)
}

fn current_art(card: &ProfileCard, application_id: u64, cx: &App) -> Option<Arc<Path>> {
    let art = card
        .applications
        .get(&application_id)
        .and_then(|app| app.icon.as_deref())
        .and_then(|icon| app_asset(application_id, icon, CURRENT_ART, cx));
    art.or_else(|| {
        let game = card.games.get(&application_id)?;
        app_asset(application_id, game.icon_hash.as_deref()?, CURRENT_ART, cx)
    })
}

fn activity_art(card: &ProfileCard, entry: &OutboxEntry, cx: &App) -> Option<Arc<Path>> {
    match &entry.kind {
        OutboxKind::PlayedGame { application_id, .. }
        | OutboxKind::LaunchedActivity { application_id, .. } => application_id.and_then(|id| {
            let icon = card.applications.get(&id)?.icon.as_deref()?;
            app_asset(id, icon, ACTIVITY_ART, cx)
        }),
        OutboxKind::Listened { tracks, .. } => tracks
            .first()
            .and_then(|track| track.image_url.as_deref())
            .and_then(|url| media(url, ACTIVITY_ART, cx)),
        OutboxKind::Unknown => None,
    }
}

/// Uploaded widget art, animated when the config marks it so; the still frame stands in while
/// the animation decodes.
fn widget_art(
    application_id: u64,
    asset: &WidgetAsset,
    size: f32,
    radius: f32,
    cx: &App,
) -> Option<AnyElement> {
    let still = ImageSpec::AppImage {
        application_id,
        hash: asset.asset_id.to_string(),
        uploaded: true,
        animated: asset.animated,
    };
    let pixels = theme::image_px(size);
    if let Some(frames) = still
        .animated()
        .and_then(|spec| animated::resolve(&spec, pixels, cx))
    {
        return Some(
            animated::image(frames)
                .size_full()
                .rounded(rpx(radius))
                .object_fit(ObjectFit::Cover)
                .id(("widget-art", asset.asset_id))
                .into_any_element(),
        );
    }
    images::resolve(&still, pixels, cx).map(|path| {
        img(path)
            .size_full()
            .rounded(rpx(radius))
            .object_fit(ObjectFit::Cover)
            .into_any_element()
    })
}

fn media(url: &str, size: f32, cx: &App) -> Option<Arc<Path>> {
    let spec = ImageSpec::Media {
        url: url.to_owned(),
        width: 0,
        height: 0,
        animated: false,
    };
    images::resolve(&spec, theme::image_px(size), cx)
}

pub(super) fn activity(app: &DiscordApp, card: &ProfileCard, cx: &App) -> Option<AnyElement> {
    let palette = card.palette();
    let game = app
        .store
        .user(card.user_id)
        .and_then(|user| user.game.as_ref());
    let now = Utc::now();
    let outbox = card
        .outbox
        .as_ref()
        .map_or(&[][..], |outbox| outbox.entries.as_slice());
    let current = game.and_then(|game| current_entry(game, outbox, now));
    let entries: Vec<_> = outbox
        .iter()
        .filter(|entry| !matches!(entry.kind, OutboxKind::Unknown))
        // The live entry for the game being played is the current-activity card instead.
        .filter(|entry| !current.is_some_and(|current| current.id == entry.id && live(entry, now)))
        .collect();
    if game.is_none() && entries.is_empty() {
        return None;
    }
    let heading = |text: &'static str| {
        div()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.strong))
            .child(text)
    };
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(24.))
            .children(game.map(|game| {
                div()
                    .flex()
                    .flex_col()
                    .gap(rpx(10.))
                    .child(heading("Current activity"))
                    .child(current_game(card, game, current, palette, cx))
            }))
            .when(!entries.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(10.))
                        .child(heading("Recent activity"))
                        .children(
                            entries
                                .into_iter()
                                .map(|entry| activity_row(card, entry, now, palette, cx)),
                        ),
                )
            })
            .into_any_element(),
    )
}

fn current_game(
    card: &ProfileCard,
    game: &GameActivity,
    entry: Option<&OutboxEntry>,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement {
    let actions = game.application_id.map_or_else(GameActions::default, |id| {
        GameActions::resolve(id, &card.games, &card.applications)
    });
    let art = game
        .large_image
        .as_ref()
        .and_then(|image| {
            presence_art(
                &image.url,
                CURRENT_ART,
                16.,
                ("profile-current-art", card.user_id),
                cx,
            )
        })
        .or_else(|| {
            let path = game
                .application_id
                .and_then(|id| current_art(card, id, cx))?;
            Some(still_art(path, 16.))
        });
    let badge = game.small_image.as_ref().and_then(|image| {
        presence_art(
            &image.url,
            CURRENT_BADGE,
            CURRENT_BADGE,
            ("profile-current-badge", card.user_id),
            cx,
        )
    });
    let elapsed = game
        .started_at
        .and_then(|started| SystemTime::now().duration_since(started).ok())
        .map(clock);
    let line = |id: &'static str, text: &String| {
        div()
            .truncate()
            .text_size(rpx(13.))
            .line_height(rpx(16.))
            .text_color(rgb(palette.strong))
            .child(label((id, card.user_id), text))
    };
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .p(rpx(14.))
        .rounded(rpx(12.))
        .bg(palette.panel)
        .child(
            playing_heading(game, palette)
                .text_size(rpx(13.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(14.))
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(rpx(CURRENT_ART))
                        .rounded(rpx(16.))
                        .map(|this| match art {
                            Some(art) => this.child(art),
                            None => this.bg(palette.hover).child(theme::icon(
                                "gamepad-2",
                                40.,
                                palette.muted,
                            )),
                        })
                        .children(badge.map(|badge| {
                            // Composite over the modal surface so the artwork cannot show through.
                            let backdrop = card
                                .theme()
                                .and_then(|theme| theme.modal_background().as_solid())
                                .unwrap_or_else(|| rgb(theme::BG).into())
                                .blend(palette.panel);
                            div()
                                .absolute()
                                .right(rpx(-8.))
                                .bottom(rpx(-8.))
                                .size(rpx(CURRENT_BADGE + 8.))
                                .p(rpx(4.))
                                .rounded_full()
                                .bg(backdrop)
                                .child(badge)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(rpx(4.))
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.strong))
                                .child(label(("profile-current-game", card.user_id), &game.name)),
                        )
                        .children(
                            game.details
                                .as_ref()
                                .map(|details| line("profile-current-details", details)),
                        )
                        .children(
                            game.state
                                .as_ref()
                                .map(|state| line("profile-current-state", state)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_x(rpx(10.))
                                .gap_y(rpx(2.))
                                .text_size(rpx(13.))
                                .line_height(rpx(16.))
                                .text_color(rgb(palette.text))
                                .children(elapsed.map(|elapsed| {
                                    activity_badge(timer_icon(game), elapsed, theme::GREEN)
                                        .font_family(cx.theme().mono_font_family.clone())
                                }))
                                .children(
                                    entry.into_iter().flat_map(activity_traits).map(
                                        |(icon, text)| activity_badge(icon, text, palette.text),
                                    ),
                                ),
                        )
                        .children(
                            actions
                                .buttons("profile-current-actions", card.user_id, palette)
                                .map(|buttons| buttons.mt(rpx(4.))),
                        )
                        .children(
                            play_button(game, palette)
                                .map(|play| div().flex().mt(rpx(4.)).child(play)),
                        ),
                ),
        )
}

/// The playing app's `activity_accessory` widget surface ("Current Mood: …"), a strip along the
/// bottom of the popout's activity card. Discord shows "–" while the value is empty.
fn activity_accessory(
    app: &DiscordApp,
    card: &ProfileCard,
    game: &GameActivity,
    palette: ProfilePalette,
    cx: &App,
) -> Option<AnyElement> {
    let application_id = game.application_id?;
    let accessory = card
        .widget_configs
        .get(&application_id)?
        .accessory
        .as_ref()?;
    let profile = card
        .identities
        .iter()
        .flat_map(|identities| &identities.identities)
        .find(|identity| identity.application_id == application_id)
        .and_then(|identity| identity.profile.as_ref());
    let resolve = |text: &WidgetText| match text {
        WidgetText::Literal(text) => Some(text.clone()),
        WidgetText::Data(key) if key == "username" => profile?.username.clone(),
        WidgetText::Data(key) => profile?
            .stats
            .iter()
            .find(|stat| &stat.name == key)
            .map(|stat| stat_value(&stat.value)),
    };
    let label = accessory.label.as_ref().and_then(resolve)?;
    let value = accessory
        .text
        .as_ref()
        .and_then(resolve)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "–".to_owned());
    let icon = accessory.icon.as_ref().and_then(|icon| {
        widget_image_art(
            application_id,
            icon,
            profile,
            18.,
            4.,
            "profile-accessory-icon",
            cx,
        )
    });
    let entity = app.entity.clone();
    Some(
        div()
            .id("profile-activity-accessory")
            .flex()
            .items_center()
            .gap(rpx(8.))
            // Flush with the card's edges, under a divider, like Discord's footer strip.
            .mx(rpx(-8.))
            .mb(rpx(-8.))
            .px(rpx(10.))
            .py(rpx(10.))
            .bg(palette.raised)
            .rounded_b(rpx(8.))
            .cursor_pointer()
            .hover(move |style| style.bg(palette.raised.opacity(1.3)))
            .text_size(rpx(13.))
            .line_height(rpx(16.))
            .text_color(rgb(palette.strong))
            .children(icon.map(|icon| div().flex_none().size(rpx(18.)).child(icon)))
            .child(
                div().flex_1().min_w_0().truncate().child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .child(format!("{label}: {value}")),
                ),
            )
            .child(theme::icon("arrow-right", 16., palette.muted))
            .on_click(move |_, _, cx| {
                entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
            })
            .into_any_element(),
    )
}

/// Rich-presence art is often a GIF behind Discord's media proxy; Discord plays it, so play it
/// here too, falling back to the still frame while it decodes.
fn presence_art(
    url: &str,
    size: f32,
    radius: f32,
    id: (&'static str, u64),
    cx: &App,
) -> Option<AnyElement> {
    let gif = url
        .split('?')
        .next()
        .is_some_and(|path| path.to_ascii_lowercase().ends_with(".gif"));
    let spec = ImageSpec::Media {
        url: url.to_owned(),
        width: 0,
        height: 0,
        animated: true,
    };
    if gif && let Some(frames) = animated::resolve(&spec, theme::image_px(size), cx) {
        return Some(
            animated::image(frames)
                .size_full()
                .rounded(rpx(radius))
                .object_fit(ObjectFit::Cover)
                .id(id)
                .into_any_element(),
        );
    }
    Some(still_art(media(url, size, cx)?, radius))
}

fn still_art(path: Arc<Path>, radius: f32) -> AnyElement {
    img(path)
        .size_full()
        .rounded(rpx(radius))
        .object_fit(ObjectFit::Cover)
        .into_any_element()
}

/// "Playing", or "Playing on Xbox" plus the console's glyph for presences from a console.
fn playing_heading(game: &GameActivity, palette: ProfilePalette) -> Div {
    let platform = match game.platform.as_deref() {
        Some("xbox") => Some(("Xbox", Some("connections/xbox"))),
        Some("ps4" | "ps5" | "playstation") => {
            Some(("PlayStation", Some("connections/playstation")))
        }
        Some("samsung") => Some(("Samsung Galaxy", None)),
        Some("meta_quest") => Some(("Meta Quest", None)),
        _ => None,
    };
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .text_color(rgb(palette.text))
        .child(match platform {
            Some((name, _)) => format!("Playing on {name}"),
            None => "Playing".to_owned(),
        })
        .children(
            platform
                .and_then(|(_, icon)| icon)
                .map(|icon| theme::icon(icon, 14., palette.text)),
        )
}

fn timer_icon(game: &GameActivity) -> &'static str {
    if game.embedded {
        "activity"
    } else {
        "gamepad-2"
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct GameActions {
    steam_url: Option<String>,
    play_url: Option<String>,
}

impl GameActions {
    fn resolve(
        application_id: u64,
        games: &HashMap<u64, Arc<Game>>,
        applications: &HashMap<u64, Arc<PublicApplication>>,
    ) -> Self {
        let game = games.get(&application_id);
        let mut linked_games: Vec<_> = games
            .values()
            .filter(|game| {
                game.id != application_id
                    && game
                        .linked_applications
                        .iter()
                        .any(|linked| linked.id == application_id)
            })
            .collect();
        linked_games.sort_by_key(|game| game.id);
        let mut linked: Vec<_> = applications
            .values()
            .filter(|application| {
                application.id != application_id
                    && (application
                        .linked_games
                        .iter()
                        .any(|linked| linked.id == application_id)
                        || game.is_some_and(|game| {
                            game.linked_applications
                                .iter()
                                .any(|linked| linked.id == application.id)
                        }))
            })
            .collect();
        linked.sort_by_key(|application| application.id);
        let action_applications: Vec<_> = applications
            .get(&application_id)
            .into_iter()
            .chain(linked)
            .collect();
        let mut action_games: Vec<&Game> = game
            .into_iter()
            .chain(linked_games)
            .map(Arc::as_ref)
            .collect();
        for application in &action_applications {
            for linked in application.linked_games.iter().filter(|linked| {
                application.id == application_id || linked.id == application_id
            }) {
                if let Some(game) = games.get(&linked.id) {
                    action_games.push(game.as_ref());
                }
                if let Some(game) = linked.application.as_ref() {
                    action_games.push(game);
                }
            }
        }
        Self {
            steam_url: action_games
                .iter()
                .find_map(|game| game.steam_app_id.as_deref().and_then(steam_store_url))
                .or_else(|| {
                    action_applications.iter().find_map(|application| {
                        application.steam_app_id.as_deref().and_then(steam_store_url)
                    })
                }),
            play_url: action_games
                .iter()
                .find_map(|game| official_game_website(game))
                .or_else(|| {
                    action_applications.iter().find_map(|application| {
                        application
                            .deeplink_uri
                            .as_deref()
                            .and_then(game_launch_url)
                            // Browser deep links may be campaigns; use official-site metadata.
                            .filter(|url| !matches!(url.scheme(), "http" | "https"))
                            .map(|url| url.to_string())
                    })
                }),
        }
    }

    fn buttons(
        self,
        context: &'static str,
        key: u64,
        palette: ProfilePalette,
    ) -> Option<Stateful<Div>> {
        if self.steam_url.is_none() && self.play_url.is_none() {
            return None;
        }
        Some(
            div()
                .id((context, key))
                .flex()
                .flex_col()
                .gap(rpx(8.))
                .children(
                    self.play_url.map(|url| {
                        game_action_button("play-now", "Play Now", None, url, palette)
                    }),
                )
                .children(self.steam_url.map(|url| {
                    game_action_button(
                        "steam",
                        "Get on Steam",
                        Some("connections/steam"),
                        url,
                        palette,
                    )
                })),
        )
    }
}

fn steam_store_url(id: &str) -> Option<String> {
    let id = id.parse::<u64>().ok().filter(|id| *id > 0)?;
    Some(format!("https://store.steampowered.com/app/{id}"))
}

fn official_game_website(game: &Game) -> Option<String> {
    game.websites
        .iter()
        .filter(|website| website.category == 1)
        .find_map(|website| {
            let url = game_launch_url(&website.url)?;
            matches!(url.scheme(), "http" | "https").then(|| url.to_string())
        })
}

fn game_launch_url(raw: &str) -> Option<reqwest::Url> {
    let url = reqwest::Url::parse(raw.trim()).ok()?;
    // Do not hand arbitrary executable/document protocols to the operating system.
    let supported = match url.scheme() {
        "http" | "https" => url.host_str().is_some(),
        "steam" => matches!(url.host_str(), Some("run" | "rungameid")),
        "com.epicgames.launcher" => {
            url.host_str() == Some("apps")
                && url
                    .query_pairs()
                    .any(|(key, value)| key == "action" && value == "launch")
        }
        _ => false,
    };
    if !supported || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(url)
}

fn game_action_button(
    id: &'static str,
    text: &'static str,
    icon: Option<&'static str>,
    url: String,
    palette: ProfilePalette,
) -> Stateful<Div> {
    // Dark profiles use Discord's Midnight hover tint; light profiles use its standard tint.
    let hover = rgba(if palette.light { 0x97979F29 } else { 0x97979F33 });
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(rpx(8.))
        .h(rpx(32.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .bg(rgba(0x97979F1F))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(palette.strong))
        .children(icon.map(|icon| theme::icon(icon, 18., palette.strong)))
        .child(div().min_w_0().truncate().child(text))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            cx.open_url(&url);
        })
}

/// Activities with the PLAY flag retain their separate Discord directory action.
fn play_button(game: &GameActivity, palette: ProfilePalette) -> Option<Stateful<Div>> {
    let application_id = game.application_id.filter(|_| game.playable)?;
    Some(
        div()
            .id("profile-current-play")
            .flex()
            .items_center()
            .justify_center()
            .gap(rpx(8.))
            .h(rpx(36.))
            .px(rpx(16.))
            .rounded(rpx(8.))
            .bg(palette.button)
            .cursor_pointer()
            .hover(move |style| style.bg(palette.raised))
            .text_size(rpx(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.strong))
            .child(theme::icon("activity", 18., palette.strong))
            .child("Play")
            .on_click(move |_, _, cx| {
                cx.open_url(&format!(
                    "https://discord.com/application-directory/{application_id}"
                ))
            }),
    )
}

pub(super) fn popout_playing(app: &DiscordApp, card: &ProfileCard, cx: &App) -> Option<AnyElement> {
    let palette = card.palette();
    let game = app
        .store
        .user(card.user_id)
        .and_then(|user| user.game.as_ref())?;
    let actions = game.application_id.map_or_else(GameActions::default, |id| {
        GameActions::resolve(id, &card.games, &card.applications)
    });
    let now = Utc::now();
    let outbox = card
        .outbox
        .as_ref()
        .map_or(&[][..], |outbox| outbox.entries.as_slice());
    let entry = current_entry(game, outbox, now);
    let art = game
        .large_image
        .as_ref()
        .and_then(|image| {
            presence_art(
                &image.url,
                POPOUT_ART,
                8.,
                ("profile-popout-art", card.user_id),
                cx,
            )
        })
        .or_else(|| {
            let path = game
                .application_id
                .and_then(|id| current_art(card, id, cx))?;
            Some(still_art(path, 8.))
        });
    let badge = game.small_image.as_ref().and_then(|image| {
        presence_art(
            &image.url,
            POPOUT_BADGE,
            POPOUT_BADGE,
            ("profile-popout-badge", card.user_id),
            cx,
        )
    });
    let elapsed = game
        .started_at
        .and_then(|started| SystemTime::now().duration_since(started).ok())
        .map(clock);
    let line = |id: &'static str, text: &String| {
        div()
            .truncate()
            .text_size(rpx(13.))
            .line_height(rpx(16.))
            .text_color(rgb(palette.strong))
            .child(label((id, card.user_id), text))
    };
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(8.))
            .p(rpx(8.))
            .rounded(rpx(8.))
            .bg(palette.panel)
            .child(
                playing_heading(game, palette)
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(12.))
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .size(rpx(POPOUT_ART))
                            .rounded(rpx(8.))
                            .map(|this| match art {
                                Some(art) => this.child(art),
                                None => this
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(palette.hover)
                                    .child(theme::icon("gamepad-2", 28., palette.muted)),
                            })
                            // Discord rings the small image in the card colour so it sits on
                            // the large one's corner.
                            .children(badge.map(|badge| {
                                div()
                                    .absolute()
                                    .right(rpx(-4.))
                                    .bottom(rpx(-4.))
                                    .size(rpx(POPOUT_BADGE + 4.))
                                    .p(rpx(2.))
                                    .rounded_full()
                                    .bg(palette.panel)
                                    .child(badge)
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(rpx(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rpx(16.))
                                    .line_height(rpx(20.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(palette.strong))
                                    .child(label(
                                        ("profile-popout-game", card.user_id),
                                        &game.name,
                                    )),
                            )
                            .children(
                                game.details
                                    .as_ref()
                                    .map(|details| line("profile-popout-details", details)),
                            )
                            .children(
                                game.state
                                    .as_ref()
                                    .map(|state| line("profile-popout-state", state)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_x(rpx(8.))
                                    .text_size(rpx(13.))
                                    .line_height(rpx(16.))
                                    .text_color(rgb(palette.text))
                                    .children(elapsed.map(|elapsed| {
                                        activity_badge(timer_icon(game), elapsed, theme::GREEN)
                                            .font_family(cx.theme().mono_font_family.clone())
                                    }))
                                    .children(entry.into_iter().flat_map(activity_traits).map(
                                        |(icon, text)| activity_badge(icon, text, palette.text),
                                    )),
                            ),
                    ),
            )
            .children(actions.buttons("profile-popout-playing-actions", card.user_id, palette))
            .children(play_button(game, palette).map(|play| play.w_full()))
            .children(activity_accessory(app, card, game, palette, cx))
            .into_any_element(),
    )
}

/// The popout's compact take on the first game-account widget; the full card lives on the Board.
pub(super) fn popout_widget(app: &DiscordApp, card: &ProfileCard, cx: &App) -> Option<AnyElement> {
    let palette = card.palette();
    let application_id =
        card.profile
            .as_ref()?
            .widgets
            .iter()
            .find_map(|widget| match widget.data {
                WidgetData::Application { application_id } => Some(application_id),
                _ => None,
            })?;
    let application = card.applications.get(&application_id);
    let config = card.widget_configs.get(&application_id);
    let profile = card
        .identities
        .iter()
        .flat_map(|identities| &identities.identities)
        .find(|identity| identity.application_id == application_id)
        .and_then(|identity| identity.profile.as_ref());
    let username = profile.and_then(|profile| profile.username.clone());
    let stat = |key: &str| {
        profile?
            .stats
            .iter()
            .find(|stat| stat.name == key)
            .map(|stat| stat_value(&stat.value))
    };
    let resolve = |text: &WidgetText| match text {
        WidgetText::Literal(text) => Some(text.clone()),
        WidgetText::Data(key) if key == "username" => username.clone(),
        WidgetText::Data(key) => stat(key),
    };
    let image = |image: &WidgetImage, size: f32, radius: f32| {
        widget_image_art(
            application_id,
            image,
            profile,
            size,
            radius,
            "profile-popout-widget",
            cx,
        )
    };
    // The popout uses the config's `mini_profile` surface; older configs only have the Board's
    // `widget_top`, so fall back to its title and hero.
    let mini = config.and_then(|config| config.mini.as_ref());
    let title = mini
        .and_then(|mini| mini.text.as_ref())
        .and_then(resolve)
        .or_else(|| {
            config
                .and_then(|config| config.title.as_ref())
                .and_then(resolve)
        })
        .or(username.clone())?;
    let stat_label = mini.and_then(|mini| mini.label.as_ref()).and_then(resolve);
    let stat_icon = mini
        .and_then(|mini| mini.icon.as_ref())
        .and_then(|icon| image(icon, 18., 0.));
    let hero = match mini.and_then(|mini| mini.image.as_ref()) {
        Some(art) => image(art, POPOUT_HERO, 8.),
        None => config
            .and_then(|config| config.hero.as_ref())
            .and_then(|hero| image(hero, POPOUT_HERO, 8.)),
    };
    let name = application
        .map(|app| app.name.clone())
        .or_else(|| config.and_then(|config| config.display_name.clone()))
        .unwrap_or_default();
    let icon = application
        .and_then(|app| app.icon.as_deref())
        .and_then(|hash| app_asset(application_id, hash, 16., cx));
    // Discord stacks a sliver of a second card under this one when the Board holds more widgets.
    let stacked = card.profile.as_ref().is_some_and(|profile| {
        profile
            .widgets
            .iter()
            .filter(|widget| !matches!(widget.data, WidgetData::Unknown))
            .count()
            > 1
    });
    let entity = app.entity.clone();
    let front = div()
        .flex()
        .items_center()
        .rounded(rpx(8.))
        .bg(palette.panel)
        .border_1()
        .border_color(palette.border)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .p(rpx(8.))
                .gap(rpx(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(6.))
                        .min_w_0()
                        .children(
                            icon.map(|path| img(path).size(rpx(16.)).flex_none().rounded(rpx(4.))),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(12.))
                                .line_height(rpx(16.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.strong))
                                .child(label(("profile-popout-app", application_id), &name)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(rpx(4.))
                        .min_w_0()
                        .h(rpx(18.))
                        .mt(rpx(6.))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.strong))
                                .child(label(("profile-popout-app-user", application_id), &title)),
                        )
                        .children(
                            stat_icon.map(|icon| div().size(rpx(18.)).flex_none().child(icon)),
                        ),
                )
                .children(stat_label.map(|text| {
                    div()
                        .truncate()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(palette.muted))
                        .child(text)
                }))
                .child(
                    div()
                        .id(("profile-popout-stats", application_id))
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(rgb(palette.text))
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
                        })
                        .child("View All Stats"),
                ),
        )
        .children(hero.map(|hero| {
            div()
                .flex_none()
                .p(rpx(8.))
                .child(div().size(rpx(POPOUT_HERO)).rounded(rpx(8.)).child(hero))
        }));
    Some(
        div()
            .flex()
            .flex_col()
            .child(front)
            .when(stacked, |this| {
                this.child(
                    div()
                        .mx(rpx(6.))
                        .h(rpx(8.))
                        .rounded_b(rpx(8.))
                        .bg(palette.panel)
                        .border_1()
                        .border_t_0()
                        .border_color(palette.border),
                )
            })
            .into_any_element(),
    )
}

/// Discord's elapsed-time clock: "6:10", "1:02:03".
fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn live(entry: &OutboxEntry, now: DateTime<Utc>) -> bool {
    // A lapsed session keeps its live trait; Discord dates it like any other entry.
    entry.traits.contains(&OutboxTrait::Live)
        && entry
            .expires_at
            .as_deref()
            .and_then(parse)
            .is_none_or(|expires| expires > now)
}

fn current_entry<'a>(
    game: &GameActivity,
    entries: &'a [OutboxEntry],
    now: DateTime<Utc>,
) -> Option<&'a OutboxEntry> {
    entries
        .iter()
        .filter_map(|entry| {
            let OutboxKind::PlayedGame {
                name,
                application_id,
            } = &entry.kind
            else {
                return None;
            };
            let exact = match (game.application_id, *application_id) {
                (Some(expected), Some(actual)) if expected == actual => true,
                (Some(_), Some(_)) => return None,
                _ => {
                    if game.name.trim().is_empty()
                        || !game.name.trim().eq_ignore_ascii_case(name.trim())
                    {
                        return None;
                    }
                    false
                }
            };
            Some((entry, exact))
        })
        .max_by_key(|(entry, exact)| {
            (
                *exact,
                live(entry, now),
                entry
                    .ended_at
                    .as_deref()
                    .or(entry.started_at.as_deref())
                    .and_then(parse),
                entry.id,
            )
        })
        .map(|(entry, _)| entry)
}

fn activity_traits(entry: &OutboxEntry) -> Vec<(&'static str, String)> {
    let mut meta = Vec::new();
    let started = entry.started_at.as_deref().and_then(parse);
    for activity_trait in &entry.traits {
        match activity_trait {
            OutboxTrait::Resurrected { last_played } => {
                if let (Some(started), Some(last)) = (started, parse(last_played)) {
                    meta.push((
                        "rotate-cw",
                        format!("Returning after {}", span(started - last)),
                    ));
                }
            }
            OutboxTrait::Duration { seconds } if *seconds >= MARATHON_SECONDS => {
                meta.push(("timer", format!("{}h Marathon", seconds / 3600)));
            }
            OutboxTrait::Streak { days } if *days >= 2 => {
                meta.push(("zap", format!("{days}x Streak")));
            }
            OutboxTrait::FirstTime => meta.push(("sprout", "New Player".to_owned())),
            OutboxTrait::Trending => meta.push(("trending-up", "Trending".to_owned())),
            _ => {}
        }
    }
    meta
}

fn activity_badge(icon: &str, text: String, color: u32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(4.))
        .text_color(rgb(color))
        .child(theme::icon(icon, 14., color))
        .child(text)
}

fn activity_row(
    card: &ProfileCard,
    entry: &OutboxEntry,
    now: DateTime<Utc>,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement {
    let live = live(entry, now);
    let art = activity_art(card, entry, cx);
    let (title, subtitle, kind_icon) = match &entry.kind {
        OutboxKind::PlayedGame { name, .. } | OutboxKind::LaunchedActivity { name, .. } => {
            (name.clone(), None, "gamepad-2")
        }
        OutboxKind::Listened { tracks, .. } => {
            let track: Option<&ListenedTrack> = tracks.first();
            (
                track.map_or_else(String::new, |track| track.title.clone()),
                track
                    .map(|track| track.artists.join(", "))
                    .filter(|artists| !artists.is_empty()),
                "music",
            )
        }
        OutboxKind::Unknown => (String::new(), None, "gamepad-2"),
    };
    let last_update = match &entry.kind {
        OutboxKind::Listened { last_update, .. }
        | OutboxKind::LaunchedActivity { last_update, .. } => last_update.as_deref(),
        _ => None,
    };
    let when = if live {
        "Now".to_owned()
    } else {
        entry
            .ended_at
            .as_deref()
            .or(last_update)
            .or(entry.started_at.as_deref())
            .and_then(parse)
            // Launched activities carry no timestamps at all; the entry id is still a snowflake.
            .or_else(|| snowflake_time(entry.id))
            .map_or_else(String::new, |at| format!("{} ago", span(now - at)))
    };
    let mut meta = vec![(kind_icon, when)];
    meta.extend(activity_traits(entry));
    div()
        .flex()
        .items_center()
        .gap(rpx(14.))
        .p(rpx(14.))
        .rounded(rpx(12.))
        .bg(palette.panel)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(ACTIVITY_ART))
                .rounded(rpx(ACTIVITY_RADIUS))
                // Transparent icons (NIKKE's wordmark) sit on the card itself, as in Discord.
                .map(|this| match art {
                    Some(path) => this.child(
                        img(path)
                            .size_full()
                            .rounded(rpx(ACTIVITY_RADIUS))
                            .object_fit(ObjectFit::Cover),
                    ),
                    None => {
                        this.bg(palette.hover)
                            .child(theme::icon(kind_icon, 28., palette.muted))
                    }
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(2.))
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.strong))
                        .child(label(("profile-activity", entry.id), &title)),
                )
                .children(subtitle.map(|subtitle| {
                    div()
                        .truncate()
                        .text_size(rpx(13.))
                        .line_height(rpx(16.))
                        .text_color(rgb(palette.text))
                        .child(label(("profile-activity-artist", entry.id), &subtitle))
                }))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(rpx(10.))
                        .gap_y(rpx(2.))
                        .mt(rpx(2.))
                        .text_size(rpx(13.))
                        .line_height(rpx(16.))
                        .text_color(rgb(palette.text))
                        .children(
                            meta.into_iter()
                                .filter(|(_, text)| !text.is_empty())
                                .map(|(icon, text)| activity_badge(icon, text, palette.text)),
                        ),
                ),
        )
}

fn snowflake_time(id: u64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(((id >> 22) + DISCORD_EPOCH_MS) as i64)
}

fn parse(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Spans as Discord words them ("23h" reads "1d", 11 days "2w"): each unit rounds to the
/// nearest whole, and a unit takes over once the smaller one would reach its threshold.
fn span(delta: chrono::TimeDelta) -> String {
    let seconds = delta.num_seconds().max(0) as f64;
    let minutes = (seconds / 60.).round() as i64;
    let hours = (seconds / 3600.).round() as i64;
    let days = (seconds / 86400.).round() as i64;
    let weeks = (days as f64 / 7.).round() as i64;
    let months = (seconds / (86400. * 30.)).round() as i64;
    match () {
        _ if minutes < 45 => format!("{}m", minutes.max(1)),
        _ if hours < 22 => format!("{}h", hours.max(1)),
        _ if days < 7 => format!("{}d", days.max(1)),
        _ if weeks < 4 => format!("{weeks}w"),
        _ if months < 11 => plural(months.max(1), "month"),
        _ => plural(((seconds / (86400. * 365.)).round() as i64).max(1), "year"),
    }
}

fn plural(count: i64, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc, time::Duration};

    use gpui::{Context, IntoElement, Render, Styled, TestAppContext, Window, div, prelude::*, px};

    use super::{
        COVER_GAP, COVER_HEIGHT, COVER_WIDTH, GameActions, ProfilePalette, SkuPrice,
        activity_traits, clock, current_entry, game_cover, game_cover_grid, game_launch_url, parse,
        sku_price, snowflake_time, span, stat_name, tag_label,
    };
    use crate::{
        discord::model::{Game, OutboxEntry, OutboxKind, OutboxTrait, PublicApplication},
        store::GameActivity,
        theme::{self, rpx},
    };

    #[test]
    fn game_actions_use_each_applications_own_targets() {
        for (id, steam_id) in [(7, "480"), (8, "481")] {
            let application: PublicApplication = serde_json::from_value(serde_json::json!({
                "id": id,
                "name": "Game",
                "deeplink_uri": format!("steam://run/{steam_id}"),
                "third_party_skus": [{"distributor": "steam", "id": steam_id}]
            }))
            .unwrap();
            let applications = HashMap::from([(id, Arc::new(application))]);
            assert_eq!(
                GameActions::resolve(id, &HashMap::new(), &applications),
                GameActions {
                    steam_url: Some(format!("https://store.steampowered.com/app/{steam_id}")),
                    play_url: Some(format!("steam://run/{steam_id}")),
                }
            );
        }
    }

    #[test]
    fn partner_widget_and_playing_game_share_linked_actions_without_presence_flags() {
        let application: PublicApplication = serde_json::from_value(serde_json::json!({
            "id": "7",
            "name": "Partner App",
            "deeplink_uri": "https://games.example/play",
            "linked_games": [{
                "id": "42",
                "application": {
                    "id": "42", "name": "Game",
                    "websites": [{"category": 1, "url": "https://games.example/play"}],
                    "third_party_skus": [{"distributor": "steam", "id": "480"}]
                }
            }]
        }))
        .unwrap();
        let applications = HashMap::from([(7, Arc::new(application))]);
        let widget_actions = GameActions::resolve(7, &HashMap::new(), &applications);
        assert_eq!(
            widget_actions,
            GameActions {
                steam_url: Some("https://store.steampowered.com/app/480".into()),
                play_url: Some("https://games.example/play".into()),
            }
        );
        assert_eq!(
            GameActions::resolve(42, &HashMap::new(), &applications),
            widget_actions
        );
    }

    #[test]
    fn game_actions_resolve_missing_embedded_metadata_from_loaded_games() {
        let application: PublicApplication = serde_json::from_value(serde_json::json!({
            "id": "7", "name": "Partner App",
            "deeplink_uri": "https://games.example/play",
            "linked_games": [{"id": "42"}]
        }))
        .unwrap();
        let game: Game = serde_json::from_value(serde_json::json!({
            "id": "42", "name": "Game",
            "third_party_skus": [{"distributor": "steam", "id": "480"}],
            "websites": [{"category": 1, "url": "https://games.example/play"}],
            "linked_applications": [{"id": "7"}]
        }))
        .unwrap();
        let mut applications = HashMap::from([(7, Arc::new(application))]);
        assert_eq!(
            GameActions::resolve(7, &HashMap::new(), &applications),
            GameActions::default()
        );
        let games = HashMap::from([(42, Arc::new(game))]);
        let expected = GameActions {
            steam_url: Some("https://store.steampowered.com/app/480".into()),
            play_url: Some("https://games.example/play".into()),
        };
        assert_eq!(GameActions::resolve(7, &games, &applications), expected);
        Arc::make_mut(applications.get_mut(&7).unwrap())
            .linked_games
            .clear();
        assert_eq!(GameActions::resolve(7, &games, &applications), expected);
        assert_eq!(GameActions::resolve(42, &games, &applications), expected);
    }

    #[test]
    fn game_actions_prefer_official_website_to_campaign_and_store_links() {
        let application: PublicApplication = serde_json::from_value(serde_json::json!({
            "id": "7", "name": "Partner App",
            "deeplink_uri": "https://games.example/campaign/index.html",
            "linked_games": [{"id": "42"}]
        }))
        .unwrap();
        let game: Game = serde_json::from_value(serde_json::json!({
            "id": "42", "name": "Game",
            "websites": [
                {"category": 13, "url": "https://store.steampowered.com/app/480/"},
                {"category": 4, "url": "https://www.facebook.com/example"},
                {"category": 1, "url": "https://games.example/en/"}
            ],
            "linked_applications": [{"id": "7"}]
        }))
        .unwrap();
        let applications = HashMap::from([(7, Arc::new(application))]);
        let games = HashMap::from([(42, Arc::new(game))]);
        for id in [7, 42] {
            assert_eq!(
                GameActions::resolve(id, &games, &applications).play_url.as_deref(),
                Some("https://games.example/en/")
            );
        }
    }

    #[test]
    fn game_actions_do_not_use_a_campaign_when_the_official_site_is_missing() {
        let application: PublicApplication = serde_json::from_value(serde_json::json!({
            "id": "7", "name": "Partner App",
            "deeplink_uri": "https://games.example/campaign/index.html",
            "linked_games": [{
                "id": "42",
                "application": {
                    "id": "42", "name": "Game",
                    "websites": [{"category": 4, "url": "https://www.facebook.com/example"}]
                }
            }]
        }))
        .unwrap();
        let mut applications = HashMap::from([(7, Arc::new(application))]);
        assert_eq!(
            GameActions::resolve(7, &HashMap::new(), &applications).play_url,
            None
        );
        let linked_game = Arc::make_mut(applications.get_mut(&7).unwrap())
            .linked_games[0].application.as_mut().unwrap();
        linked_game.websites.push(crate::discord::model::GameWebsite {
            url: "http://games.example/".into(),
            category: 1,
        });
        assert_eq!(
            GameActions::resolve(7, &HashMap::new(), &applications).play_url.as_deref(),
            Some("http://games.example/")
        );
    }

    #[test]
    fn game_actions_do_not_borrow_unrelated_or_invalid_targets() {
        let application: PublicApplication = serde_json::from_value(serde_json::json!({
            "id": "7", "name": "Invalid Targets",
            "deeplink_uri": "javascript:alert(1)",
            "third_party_skus": [{"distributor": "steam", "id": "../invalid"}],
            "linked_games": [{"id": "42"}]
        }))
        .unwrap();
        let unrelated: Game = serde_json::from_value(serde_json::json!({
            "id": "99", "name": "Other Game",
            "third_party_skus": [{"distributor": "steam", "id": "480"}]
        }))
        .unwrap();
        let applications = HashMap::from([(7, Arc::new(application))]);
        let games = HashMap::from([(99, Arc::new(unrelated))]);
        assert_eq!(
            GameActions::resolve(7, &games, &applications),
            GameActions::default()
        );
        assert_eq!(
            GameActions::resolve(8, &games, &applications),
            GameActions::default()
        );
        assert_eq!(
            GameActions::resolve(99, &games, &applications),
            GameActions {
                steam_url: Some("https://store.steampowered.com/app/480".into()),
                play_url: None,
            }
        );
    }

    #[test]
    fn game_launch_urls_only_accept_supported_launch_protocols() {
        for url in [
            "https://games.example/play",
            "steam://run/480",
            "steam://rungameid/480",
            "com.epicgames.launcher://apps/example?action=launch",
        ] {
            assert!(game_launch_url(url).is_some(), "{url}");
        }
        for url in [
            "",
            "not a URL",
            "javascript:alert(1)",
            "file:///tmp/game",
            "data:text/html,game",
            "https://user:password@games.example/play",
            "steam://uninstall/480",
            "com.epicgames.launcher://apps/example?action=uninstall",
        ] {
            assert!(game_launch_url(url).is_none(), "{url}");
        }
    }

    const COVER_SELECTORS: [&str; 5] = [
        "liked-game-cover-0",
        "liked-game-cover-1",
        "liked-game-cover-2",
        "liked-game-cover-3",
        "liked-game-cover-4",
    ];

    struct CoverGrid {
        width: f32,
    }

    impl Render for CoverGrid {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().w(rpx(self.width)).child(
                game_cover_grid(COVER_SELECTORS.into_iter().map(|selector| {
                    game_cover(None, ProfilePalette::of(None), cx)
                        .debug_selector(move || selector.into())
                        .into_any_element()
                }))
                .debug_selector(|| "liked-games-grid".into()),
            )
        }
    }

    #[gpui::test]
    fn liked_game_covers_wrap_without_collapsing_the_collection(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| CoverGrid { width: 400. });
        for zoom in [1., 1.25, 1.5] {
            for (width, columns) in [(400., 4), (399., 3), (296., 3), (192., 2)] {
                view.update(cx, |grid, cx| {
                    grid.width = width;
                    cx.notify();
                });
                cx.update(|window, cx| {
                    window.set_rem_size(px(theme::BASE_FONT_SIZE * zoom));
                    window.refresh();
                    let _ = window.draw(cx);
                });
                let grid = cx.debug_bounds("liked-games-grid").unwrap();
                for (index, selector) in COVER_SELECTORS.into_iter().enumerate() {
                    let cover = cx.debug_bounds(selector).unwrap();
                    let left = (index % columns) as f32 * (COVER_WIDTH + COVER_GAP) * zoom;
                    let top = (index / columns) as f32 * (COVER_HEIGHT + COVER_GAP) * zoom;
                    assert!((cover.left() - grid.left() - px(left)).abs() <= px(1.));
                    assert!((cover.top() - grid.top() - px(top)).abs() <= px(1.));
                    assert!((cover.size.width - px(COVER_WIDTH * zoom)).abs() <= px(1.));
                    assert!((cover.size.height - px(COVER_HEIGHT * zoom)).abs() <= px(1.));
                    assert!(cover.right() <= grid.right() + px(1.));
                    assert!(cover.bottom() <= grid.bottom() + px(1.));
                }
            }
        }
    }

    fn game_entry(id: u64, name: &str, application_id: Option<u64>) -> OutboxEntry {
        OutboxEntry {
            id,
            started_at: None,
            ended_at: None,
            expires_at: None,
            kind: OutboxKind::PlayedGame {
                name: name.to_owned(),
                application_id,
            },
            traits: Vec::new(),
        }
    }

    #[test]
    fn game_covers_match_the_compact_three_by_four_reference() {
        assert!((COVER_WIDTH - 87.61).abs() < 1.);
        assert!((COVER_HEIGHT - 116.81).abs() < 1.);
        assert!((COVER_WIDTH / COVER_HEIGHT - 3. / 4.).abs() < f32::EPSILON);
    }

    #[test]
    fn current_activity_reuses_the_matching_history_badges() {
        let game = GameActivity {
            name: "Calculate It".into(),
            application_id: Some(7),
            platform: None,
            embedded: false,
            playable: false,
            started_at: None,
            details: None,
            state: None,
            large_image: None,
            small_image: None,
        };
        let mut entry = game_entry(1, &game.name, game.application_id);
        entry.traits = vec![
            OutboxTrait::Live,
            OutboxTrait::FirstTime,
            OutboxTrait::Streak { days: 6 },
        ];
        let entries = [entry];
        let now = parse("2026-10-25T12:00:00Z").unwrap();
        let matched = current_entry(&game, &entries, now).unwrap();
        assert_eq!(matched.id, 1);
        assert_eq!(
            activity_traits(matched),
            [("sprout", "New Player".into()), ("zap", "6x Streak".into())]
        );
        assert!(current_entry(&game, &[], now).is_none());
    }

    #[test]
    fn current_activity_prefers_ids_and_only_falls_back_when_an_id_is_missing() {
        let game = GameActivity {
            name: "Calculate It".into(),
            application_id: Some(7),
            platform: None,
            embedded: false,
            playable: false,
            started_at: None,
            details: None,
            state: None,
            large_image: None,
            small_image: None,
        };
        let entries = [
            game_entry(1, "Localized title", Some(7)),
            game_entry(2, &game.name, Some(8)),
            game_entry(3, "calculate it", None),
        ];
        let now = parse("2026-10-25T12:00:00Z").unwrap();
        assert_eq!(current_entry(&game, &entries, now).unwrap().id, 1);
        assert_eq!(current_entry(&game, &entries[1..], now).unwrap().id, 3);
        assert!(current_entry(&game, &entries[1..2], now).is_none());
    }

    #[test]
    fn current_activity_prefers_a_live_session_until_it_expires() {
        let game = GameActivity {
            name: "Calculate It".into(),
            application_id: Some(7),
            platform: None,
            embedded: false,
            playable: false,
            started_at: None,
            details: None,
            state: None,
            large_image: None,
            small_image: None,
        };
        let mut live = game_entry(1, &game.name, game.application_id);
        live.traits.push(OutboxTrait::Live);
        live.expires_at = Some("2026-10-25T12:01:00Z".into());
        let mut recent = game_entry(2, &game.name, game.application_id);
        recent.ended_at = Some("2026-10-25T11:00:00Z".into());
        let mut old = game_entry(3, &game.name, game.application_id);
        old.ended_at = Some("2026-10-24T11:00:00Z".into());
        let entries = [live, recent, old];
        assert_eq!(
            current_entry(&game, &entries, parse("2026-10-25T12:00:00Z").unwrap())
                .unwrap()
                .id,
            1
        );
        assert_eq!(
            current_entry(&game, &entries, parse("2026-10-25T12:02:00Z").unwrap())
                .unwrap()
                .id,
            2
        );
    }

    #[test]
    fn activity_badges_do_not_invent_traits_or_show_short_marathons_and_streaks() {
        let mut entry = game_entry(1, "Game", Some(7));
        assert!(activity_traits(&entry).is_empty());
        entry.traits = vec![
            OutboxTrait::Live,
            OutboxTrait::Unknown,
            OutboxTrait::Duration { seconds: 60 },
            OutboxTrait::Streak { days: 1 },
        ];
        assert!(activity_traits(&entry).is_empty());
        entry.traits = vec![
            OutboxTrait::Duration { seconds: 3 * 3600 },
            OutboxTrait::Trending,
        ];
        assert_eq!(
            activity_traits(&entry),
            [
                ("timer", "3h Marathon".into()),
                ("trending-up", "Trending".into())
            ]
        );
    }

    #[test]
    fn clocks_read_like_discord() {
        assert_eq!(clock(Duration::from_secs(370)), "6:10");
        assert_eq!(clock(Duration::from_secs(18 * 60)), "18:00");
        assert_eq!(clock(Duration::from_secs(3723)), "1:02:03");
    }

    #[test]
    fn tags_and_stats_read_like_discord() {
        assert_eq!(tag_label("better_than_you"), "Better than you");
        assert_eq!(tag_label("obsessed"), "Obsessed");
        assert_eq!(tag_label("lfg"), "LFG");
        assert_eq!(stat_name("achievement_num"), "Achievement");
    }

    #[test]
    fn spans_round_like_discord() {
        let span_of = |minutes| span(chrono::TimeDelta::minutes(minutes));
        assert_eq!(span_of(0), "1m");
        assert_eq!(span_of(44), "44m");
        assert_eq!(span_of(50), "1h");
        assert_eq!(span_of(75), "1h");
        assert_eq!(span_of(60 * 23), "1d");
        assert_eq!(span_of(60 * 24 * 3), "3d");
        assert_eq!(span_of(60 * 24 * 11), "2w");
        assert_eq!(span_of(60 * 24 * 40), "1 month");
        assert_eq!(span_of(60 * 24 * 800), "2 years");
    }

    #[test]
    fn wishlist_prices_and_launched_activity_times_read_like_discord() {
        let price = |amount, currency: &str, exponent| {
            sku_price(&SkuPrice {
                amount,
                currency: currency.to_owned(),
                exponent,
            })
        };
        assert_eq!(price(1399, "usd", 2), "$13.99");
        assert_eq!(price(2049, "eur", 2), "€20.49");
        assert_eq!(price(150000, "idr", 0), "150000 IDR");
        // A synthetic launched-game entry: no timestamps, only its snowflake id.
        assert_eq!(
            snowflake_time(1191169321074688097).unwrap().to_rfc3339(),
            "2024-01-01T00:01:37+00:00"
        );
    }
}
