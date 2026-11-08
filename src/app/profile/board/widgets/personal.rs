use super::application::{Tile, tag_label};
use super::*;

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

pub(crate) fn widget_image_art(
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

/// gpui's `ObjectFit::Cover` always centres and there's no image mask, so the art is sized by
/// hand and painted under an edge fade.
pub(super) fn widget_hero(
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

pub(super) fn personal_widget(
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

pub(super) fn collection(key: u64, tiles: Vec<Tile>, palette: ProfilePalette) -> impl IntoElement {
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

pub(crate) fn stat_value(value: &StatValue) -> String {
    match value {
        StatValue::Text(text) => text.clone(),
        StatValue::Image { url, .. } => url.clone(),
        StatValue::Number(number) if number.fract() == 0. => format!("{number:.0}"),
        StatValue::Number(number) => number.to_string(),
    }
}

pub(crate) fn stat_name(name: &str) -> String {
    let name = name
        .strip_suffix("_num")
        .or_else(|| name.strip_suffix("_count"))
        .unwrap_or(name);
    tag_label(name)
}
