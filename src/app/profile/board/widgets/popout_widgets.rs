use super::personal::{stat_value, widget_image_art};
use super::*;

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
    let front = popout_widget_front(app, card, cx)?;
    let stacked = card.profile.as_ref().is_some_and(|profile| {
        profile
            .widgets
            .iter()
            .filter(|widget| !matches!(widget.data, WidgetData::Unknown))
            .count()
            > 1
    });
    Some(if stacked {
        super::card_stack(app, card, Stack::Widget, Tab::Board, front, cx)
    } else {
        front
    })
}

fn popout_widget_front(app: &DiscordApp, card: &ProfileCard, cx: &App) -> Option<AnyElement> {
    let palette = card.palette();
    let profile = card.profile.as_ref()?;
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
    let entity = app.entity.clone();
    let front = super::hover_card(
        app,
        ("profile-popout-widget", application_id),
        Tab::Board,
        palette,
    )
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
                    .children(stat_icon.map(|icon| div().size(rpx(18.)).flex_none().child(icon))),
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
                        cx.stop_propagation();
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
    Some(front.into_any_element())
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
    super::hover_card(
        app,
        ("profile-popout-personal-card", widget_id),
        Tab::Board,
        palette,
    )
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
                        cx.stop_propagation();
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
    Some(
        super::hover_card(
            app,
            ("profile-popout-collection", card.user_id),
            Tab::Board,
            palette,
        )
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
