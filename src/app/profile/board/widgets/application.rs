use super::personal::{collection, stat_name, stat_value, widget_hero, widget_image_art};
use super::*;

pub(super) fn tag_pill(tag: &str, palette: ProfilePalette) -> impl IntoElement {
    let icon = tag_icon(tag);
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

pub(in crate::app::profile) fn tag_icon(tag: &str) -> &'static str {
    match tag {
        "open_to_play" | "open_to_teach" | "looking_for_group" | "lfg" | "looking_for_tips"
        | "looking_to_discuss" | "looking_to_learn" => "users",
        "love_it" | "obsessed" | "kind_of_love_it" | "like_it" | "too_easy" => "thumbs-up",
        "kind_of_hate_it" | "rage_quitting" | "frustrated" => "thumbs-down",
        _ => "award",
    }
}

/// Discord's tag ids read as sentence case ("better_than_you" → "Better than you").
pub(in crate::app::profile) fn tag_label(tag: &str) -> String {
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

pub(super) struct Tile {
    pub(super) name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) image: Option<AnyElement>,
}

pub(super) fn application_widget(
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
                        .gap(rpx(2.))
                        .mt(rpx(4.))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(palette.text))
                        // Apps fill subtitles with markdown, e.g. Example Music App's `[@name](profile url)`.
                        // The body fills its parent, so this wrapper sizes it to the text and
                        // the icon follows straight after, as on Discord.
                        .child(div().flex_initial().min_w_0().child(markdown(
                            app,
                            card,
                            key.wrapping_add(index as u64 + 1),
                            &text,
                            cx,
                        )))
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
