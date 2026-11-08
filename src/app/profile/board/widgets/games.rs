use super::application::tag_pill;
use super::*;

pub(super) fn covers_widget(
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

pub(crate) fn game_cover_grid(covers: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .w_full()
        .gap(rpx(COVER_GAP))
        .mt(rpx(12.))
        .children(covers)
}

pub(crate) fn game_cover(
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
        (None, None, Some(hash)) => app_asset(game.id, hash, COVER_HEIGHT, cx),
        (None, None, None) => None,
    }
}

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

#[allow(clippy::too_many_arguments)]
pub(super) fn games_widget(
    app: &DiscordApp,
    card: &ProfileCard,
    widget_id: u64,
    title: &'static str,
    games: &[WidgetGame],
    featured: bool,
    editing: bool,
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    let expanded = editing || card.expanded_widgets.contains(&widget_id);
    let shown = if expanded {
        games.len()
    } else {
        games.len().min(COLLAPSED_GAMES)
    };
    let toggle = show_more(
        app,
        widget_id,
        !editing && games.len() > COLLAPSED_GAMES,
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
                let row = game_row(app, card, widget_id, game, featured, editing, palette, cx);
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

#[allow(clippy::too_many_arguments)]
fn game_row(
    app: &DiscordApp,
    card: &ProfileCard,
    widget_id: u64,
    entry: &WidgetGame,
    featured: bool,
    editing: bool,
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    let game = card.games.get(&entry.game_id);
    let key = widget_id ^ entry.game_id;
    let name = div()
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
        ));
    if editing {
        return crate::app::profile::widget_editor::game_row(
            app,
            widget_id,
            entry,
            game_cover(game.map(Arc::as_ref), palette, cx).into_any_element(),
            name,
            palette,
        );
    }
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
                .child(name)
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
        .into_any_element()
}
