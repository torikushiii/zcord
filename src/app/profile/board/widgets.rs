use super::art::{
    activity_art, app_asset, current_art, media, personal_art, presence_art, widget_art,
};
use super::*;
use crate::app::profile::popout::markdown;

pub(in crate::app::profile) fn board_panel(
    app: &DiscordApp,
    card: &ProfileCard,
    cx: &App,
) -> AnyElement {
    let palette = card.palette();
    let widgets = card.profile.iter().flat_map(|profile| &profile.widgets);
    div()
        .flex()
        .flex_col()
        .gap(rpx(16.))
        .children(widgets.filter_map(|widget| {
            let (title, games) = match &widget.data {
                WidgetData::Application { application_id } => {
                    return Some(application_widget(app, card, *application_id, palette, cx));
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
                WidgetData::Personal { header, sections } => {
                    return Some(personal_widget(
                        card,
                        widget.id,
                        header.as_deref(),
                        sections,
                        palette,
                        cx,
                    ));
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

pub(super) fn game_cover_grid(covers: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .w_full()
        .gap(rpx(COVER_GAP))
        .mt(rpx(12.))
        .children(covers)
}

pub(super) fn game_cover(
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
pub(in crate::app::profile) fn prefetch_images(card: &ProfileCard, current: Option<u64>, cx: &App) {
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
pub(super) fn tag_label(tag: &str) -> String {
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
    app: &DiscordApp,
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
                    image: item.image.as_ref().and_then(|image| {
                        widget_image_art(
                            application_id,
                            image,
                            profile,
                            TILE_IMAGE,
                            8.,
                            "profile-widget-tile",
                            cx,
                        )
                    }),
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
    // `widget_top_contained` layouts show their image as a square tile beside the text instead.
    let contained = config
        .and_then(|config| config.contained.as_ref())
        .and_then(|image| {
            widget_image_art(
                application_id,
                image,
                profile,
                CONTAINED_IMAGE,
                8.,
                "profile-widget-contained",
                cx,
            )
        });
    let key = application_id;
    widget_card(palette)
        .child(
            div()
                .relative()
                .min_h(rpx(HERO_HEIGHT))
                .p(rpx(18.))
                .when(contained.is_some(), |this| {
                    this.pr(rpx(18. + CONTAINED_IMAGE + 16.))
                })
                .children(hero)
                .children(contained.map(|image| {
                    div()
                        .absolute()
                        .top(rpx(18.))
                        .right(rpx(18.))
                        .size(rpx(CONTAINED_IMAGE))
                        .rounded(rpx(8.))
                        .bg(palette.hover)
                        .child(image)
                }))
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
                        // Apps fill subtitles with markdown, e.g. Example Music App's `[@name](profile url)`.
                        .child(markdown(
                            app,
                            card,
                            key.wrapping_add(index as u64 + 1),
                            &text,
                            cx,
                        ))
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
pub(super) fn widget_image_art(
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

/// A user-built card: an optional header, a wide cover image with a caption, and a grid of
/// titled images laid out like an application widget's collection.
fn personal_widget(
    card: &ProfileCard,
    widget_id: u64,
    header: Option<&str>,
    sections: &[PersonalSection],
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    let user_id = card.user_id;
    let mut body = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        let key = widget_id ^ (index as u64 + 1) << 32;
        match section {
            PersonalSection::Cover {
                title,
                subtitle,
                image,
            } => {
                let art = image.as_ref().and_then(|image| {
                    let aspect = hero_aspect(image.width, image.height);
                    let art = personal_art(user_id, image, PERSONAL_COVER, 8., cx)?;
                    Some(
                        div()
                            .w_full()
                            .aspect_ratio(aspect)
                            .rounded(rpx(8.))
                            .bg(palette.hover)
                            .overflow_hidden()
                            .child(art),
                    )
                });
                if art.is_none() && title.is_none() && subtitle.is_none() {
                    continue;
                }
                body.push(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(4.))
                        .px(rpx(18.))
                        .children(art)
                        .children(title.as_ref().map(|title| {
                            div()
                                .mt(rpx(8.))
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.strong))
                                .child(label(("profile-personal-title", key), title))
                        }))
                        .children(subtitle.as_ref().map(|subtitle| {
                            div()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .text_color(rgb(palette.text))
                                .child(label(("profile-personal-subtitle", key), subtitle))
                        }))
                        .into_any_element(),
                );
            }
            PersonalSection::Fields { fields } if !fields.is_empty() => {
                let tiles = fields
                    .iter()
                    .map(|field| Tile {
                        name: field.title.clone(),
                        description: field.description.clone(),
                        image: field
                            .image
                            .as_ref()
                            .and_then(|image| personal_art(user_id, image, TILE_IMAGE, 8., cx)),
                    })
                    .collect();
                body.push(collection(key, tiles, palette).into_any_element());
            }
            PersonalSection::Fields { .. } | PersonalSection::Unknown => {}
        }
    }
    widget_card(palette)
        .pt(rpx(18.))
        .gap(rpx(2.))
        .when(header.is_none() && body.is_empty(), |this| {
            this.pb(rpx(18.))
        })
        .children(header.map(|header| {
            div()
                .px(rpx(18.))
                .pb(rpx(16.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.strong))
                .child(label(("profile-personal-header", widget_id), header))
        }))
        .children(body)
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

pub(super) fn stat_value(value: &StatValue) -> String {
    match value {
        StatValue::Text(text) => text.clone(),
        StatValue::Image { url, .. } => url.clone(),
        StatValue::Number(number) if number.fract() == 0. => format!("{number:.0}"),
        StatValue::Number(number) => number.to_string(),
    }
}

/// Readable fallback for raw stat keys ("achievement_num") when an app has no widget config.
pub(super) fn stat_name(name: &str) -> String {
    let name = name
        .strip_suffix("_num")
        .or_else(|| name.strip_suffix("_count"))
        .unwrap_or(name);
    tag_label(name)
}

/// The popout's compact take on the first game-account widget; the full card lives on the Board.
enum Popout<'a> {
    Application(u64),
    Personal(u64, &'a Option<String>, &'a [PersonalSection]),
    Games,
}

pub(in crate::app::profile) fn popout_widget(
    app: &DiscordApp,
    card: &ProfileCard,
    cx: &App,
) -> Option<AnyElement> {
    let palette = card.palette();
    let profile = card.profile.as_ref()?;
    // Discord summarises the first Board widget it can show in the popout, in Board order:
    // game lists become the Game Collection row, and application widgets count only when their
    // config has a `mini_profile` surface.
    let application_id = profile
        .widgets
        .iter()
        .find_map(|widget| match &widget.data {
            WidgetData::Application { application_id }
                if card
                    .widget_configs
                    .get(application_id)
                    .is_some_and(|config| config.mini.is_some()) =>
            {
                Some(Popout::Application(*application_id))
            }
            WidgetData::Personal { header, sections } => {
                Some(Popout::Personal(widget.id, header, sections))
            }
            WidgetData::FavoriteGames { games }
            | WidgetData::CurrentGames { games }
            | WidgetData::PlayedGames { games }
            | WidgetData::WantToPlayGames { games }
                if !games.is_empty() =>
            {
                Some(Popout::Games)
            }
            _ => None,
        });
    let application_id = match application_id? {
        Popout::Application(application_id) => application_id,
        Popout::Personal(widget_id, header, sections) => {
            return Some(popout_personal(
                app,
                card,
                widget_id,
                header.as_deref(),
                sections,
                cx,
            ));
        }
        Popout::Games => return game_collection(app, card, profile, palette, cx),
    };
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
    // The mini surface may leave out its text or image; the Board's `widget_top` fills them in.
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

fn popout_personal(
    app: &DiscordApp,
    card: &ProfileCard,
    widget_id: u64,
    header: Option<&str>,
    sections: &[PersonalSection],
    cx: &App,
) -> AnyElement {
    let palette = card.palette();
    let cover = sections.iter().find_map(|section| match section {
        PersonalSection::Cover {
            image: Some(image), ..
        } => personal_art(card.user_id, image, POPOUT_COVER_WIDTH, 4., cx),
        _ => None,
    });
    let entity = app.entity.clone();
    div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .p(rpx(8.))
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
                .gap(rpx(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(4.))
                        .min_w_0()
                        .children(header.map(|header| {
                            div()
                                .truncate()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.strong))
                                .child(label(("profile-popout-personal", widget_id), header))
                        })),
                )
                .child(
                    div()
                        .id(("profile-popout-personal-view", widget_id))
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(palette.text))
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
                        })
                        .child("View Widget"),
                ),
        )
        .children(cover.map(|cover| {
            div()
                .flex_none()
                .w(rpx(POPOUT_COVER_WIDTH))
                .h(rpx(POPOUT_COVER_HEIGHT))
                .rounded(rpx(4.))
                .overflow_hidden()
                .child(cover)
        }))
        .into_any_element()
}

/// The popout sums up the Board's game lists as one row of icons.
fn game_collection(
    app: &DiscordApp,
    card: &ProfileCard,
    profile: &crate::discord::model::Profile,
    palette: ProfilePalette,
    cx: &App,
) -> Option<AnyElement> {
    let ids = super::widget_game_ids(profile);
    if ids.is_empty() {
        return None;
    }
    // Discord showed 3 icons and "+13" for 17 games, so the count leaves out one more than shown.
    let hidden = ids.len().saturating_sub(COLLECTION_SHOWN + 1);
    let shown = if hidden == 0 {
        ids.len().min(COLLECTION_SHOWN + 1)
    } else {
        COLLECTION_SHOWN
    };
    let tile = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(COLLECTION_ICON))
            .rounded(rpx(4.))
            .bg(palette.hover)
            .overflow_hidden()
    };
    let icons = ids[..shown].iter().map(|id| {
        let icon = card
            .games
            .get(id)
            .and_then(|game| app_asset(*id, game.icon_hash.as_deref()?, COLLECTION_ICON, cx));
        tile()
            .children(icon.map(|path| {
                img(path)
                    .size_full()
                    .rounded(rpx(4.))
                    .object_fit(ObjectFit::Cover)
            }))
            .into_any_element()
    });
    let entity = app.entity.clone();
    Some(
        div()
            .id(("profile-popout-collection", card.user_id))
            .flex()
            .items_center()
            .gap(rpx(8.))
            .p(rpx(8.))
            .rounded(rpx(8.))
            .bg(palette.panel)
            .border_1()
            .border_color(palette.border)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.strong))
                    .child("Game Collection"),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(rpx(4.))
                    .children(icons)
                    .when(hidden > 0, |this| {
                        this.child(
                            tile()
                                .bg(palette.button)
                                .text_size(rpx(12.))
                                .line_height(rpx(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.strong))
                                .child(format!("+{hidden}")),
                        )
                    }),
            )
            .into_any_element(),
    )
}
