use super::*;

const VIDEO_MIN: (f32, f32) = (150., 144.);
const GALLERY_GAP: f32 = 4.;
const VIDEO_SLOT: u64 = 1 << 22;
const IMAGE_SLOT: u64 = 50;
const IMAGE_GRID_SLOT: u64 = 52;

fn image_box(media: &Media, width: f32, height: f32, radius: f32, cx: &App) -> Div {
    let pixels = theme::image_px(width.max(height));
    let spec = media.spec();
    let frames = animated_media(media)
        .then(|| crate::media::animated::resolve(&spec, pixels, cx))
        .flatten();
    let child = match frames {
        Some(image) => Some(
            crate::media::animated::image(image)
                .absolute()
                .inset_0()
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(rpx(radius))
                .id(animated_id(&media.url))
                .into_any_element(),
        ),
        None if animated_media(media) => {
            crate::media::animated::poster(&spec, pixels, cx).map(|image| {
                image
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded(rpx(radius))
                    .id(animated_id(&media.url))
                    .into_any_element()
            })
        }
        None => None,
    }
    .or_else(|| {
        let fallback = if animated_media(media) {
            crate::media::animated::fallback(&spec, pixels, cx)?
        } else {
            spec.clone()
        };
        images::resolve(&fallback, pixels, cx).map(|path| {
            animated(path, &media.url)
                .absolute()
                .inset_0()
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(rpx(radius))
                .into_any_element()
        })
    });
    div()
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(radius))
        .overflow_hidden()
        .when(child.is_none(), |this| {
            this.bg(rgba(theme::border()))
                .when(!images::failed(&spec, pixels, cx), |this| {
                    this.child(theme::media_loading())
                })
        })
        .children(child)
}

#[allow(clippy::too_many_arguments)]
fn gifv_box(
    app: &DiscordApp,
    key: u64,
    still: &Media,
    video: Option<&Media>,
    width: f32,
    height: f32,
    cx: &App,
) -> Stateful<Div> {
    let pixels = theme::image_px(width.max(height));
    let playing = video.and_then(|video| {
        crate::media::video::resolve(&video.video_spec(), pixels, cx).map(|frames| (video, frames))
    });
    let element = match playing {
        Some((video, frames)) => div()
            .flex_none()
            .w(rpx(width))
            .h(rpx(height))
            .rounded(rpx(MEDIA_RADIUS))
            .overflow_hidden()
            .child(
                crate::media::animated::image(frames)
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded(rpx(MEDIA_RADIUS))
                    .id(animated_id(&video.url)),
            ),
        None => image_box(still, width, height, MEDIA_RADIUS, cx),
    };
    element
        .id(("lightbox-open", key))
        .cursor_pointer()
        .without_text_selection()
        .on_click(crate::app::lightbox::gifv_opener(app, still, video, pixels))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn lightbox_box(
    app: &DiscordApp,
    key: u64,
    media: &Media,
    width: f32,
    height: f32,
    radius: f32,
    cx: &App,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    let spec = media.spec();
    let animated = animated_media(media);
    let url = media.source.clone();
    let preview = theme::image_px(width.max(height));
    let (original_width, original_height) = if media.width > 0 && media.height > 0 {
        (media.width, media.height)
    } else {
        (width.round().max(1.) as u32, height.round().max(1.) as u32)
    };
    image_box(media, width, height, radius, cx)
        .id(("lightbox-open", key))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| {
                    app.open_lightbox(
                        spec.clone(),
                        animated,
                        (original_width, original_height),
                        preview,
                        url.clone(),
                        cx,
                    )
                })
                .ok();
        })
}

pub(super) fn spoiler_cover(app: &DiscordApp, key: u64, spoiler: bool, element: Div) -> AnyElement {
    if !spoiler || app.spoiler_shown(&(key, 0)) {
        return element.into_any_element();
    }
    element
        .child(spoiler_overlay(app, key, 8.).bg(rgb(0x1A1A1E)))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spoiler_media(
    app: &DiscordApp,
    key: u64,
    spoiler: bool,
    media: &Media,
    width: f32,
    height: f32,
    radius: f32,
    cx: &App,
) -> Stateful<Div> {
    if spoiler && !app.spoiler_shown(&(key, 0)) {
        blurred_box(&media.spec(), width, height, radius, cx)
            .id(("hidden-media", key))
            .child(spoiler_overlay(app, key, radius))
    } else {
        lightbox_box(app, key, media, width, height, radius, cx)
    }
}

pub(crate) fn blurred_box(
    spec: &images::ImageSpec,
    width: f32,
    height: f32,
    radius: f32,
    cx: &App,
) -> Div {
    let image = images::blurred(spec, cx);
    div()
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(radius))
        .overflow_hidden()
        .when(image.is_none(), |this| this.bg(rgba(theme::border())))
        .children(image.map(|image| {
            img(image)
                .absolute()
                .inset_0()
                .size_full()
                .rounded(rpx(radius))
                .object_fit(ObjectFit::Cover)
        }))
}

pub(crate) fn spoiler_overlay(app: &DiscordApp, key: u64, radius: f32) -> Stateful<Div> {
    let entity = app.entity.clone();
    div()
        .id(("spoiler", key))
        .absolute()
        .inset_0()
        .rounded(rpx(radius))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x4E505840))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.reveal_spoiler(key, 0, cx))
                .ok();
        })
        .child(
            div()
                .px(rpx(12.))
                .py(rpx(8.))
                .rounded_full()
                .bg(rgba(0x000000B3))
                .text_size(rpx(15.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(theme::strong()))
                .child("SPOILER"),
        )
}

fn play_badge(element: Div, pill: bool) -> Div {
    let badge = div()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x000000A6));
    let badge = if pill {
        badge
            .gap(rpx(12.))
            .h(rpx(48.))
            .px(rpx(14.))
            .rounded(rpx(24.))
            .child(theme::icon("play", 22., theme::strong()))
            .child(theme::icon("external-link", 22., theme::strong()))
    } else {
        badge
            .size(rpx(48.))
            .rounded_full()
            .child(theme::icon("play", 20., theme::strong()))
    };
    element.child(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(badge),
    )
}

pub(super) fn icon_image(media: Option<&Media>, size: f32, cx: &App) -> Option<AnyElement> {
    let media = media?;
    let image = images::resolve(&media.spec(), theme::image_px(size), cx);
    Some(
        div()
            .size(rpx(size))
            .flex_none()
            .rounded_full()
            .overflow_hidden()
            .bg(rgba(theme::border()))
            .children(image.map(|path| img(path).size_full().rounded_full()))
            .into_any_element(),
    )
}

pub(super) fn embed_media(
    app: &DiscordApp,
    embed: &Embed,
    key: &impl Fn(u64) -> u64,
    max: (f32, f32),
    carded: bool,
    cx: &App,
) -> Option<(AnyElement, f32)> {
    if embed.kind == EmbedKind::Gifv {
        let still = embed.thumbnail.as_ref().or(embed.images.first())?;
        let video = embed.video.as_ref();
        let shape = video.filter(|video| video.width > 0).unwrap_or(still);
        let (width, height) = fit_in(shape, max, carded);
        let element = gifv_box(app, key(IMAGE_SLOT), still, video, width, height, cx);
        return Some((element.into_any_element(), width));
    }
    if let Some(found) = video_media(app, embed, key, max, carded, cx) {
        return Some(found);
    }
    match embed.images.as_slice() {
        [] => None,
        [single] => {
            let (width, height) = fit_in(single, max, carded);
            let element = lightbox_box(
                app,
                key(IMAGE_SLOT),
                single,
                width,
                height,
                MEDIA_RADIUS,
                cx,
            );
            Some((element.into_any_element(), width))
        }
        many => {
            let room =
                crate::app::chat::media_limit() - if carded { EMBED_HORIZONTAL_INSET } else { 0. };
            let width = if carded {
                media_card_width(embed, max).unwrap_or(max.0)
            } else {
                INLINE_MEDIA_WIDTH
            }
            .min(room);
            let element = gallery_grid(app, many, width, max.1, key, cx);
            Some((element, width))
        }
    }
}

fn video_media(
    app: &DiscordApp,
    embed: &Embed,
    key: &impl Fn(u64) -> u64,
    max: (f32, f32),
    carded: bool,
    cx: &App,
) -> Option<(AnyElement, f32)> {
    let video = embed.video.as_ref()?;
    let still = embed.thumbnail.as_ref().or(embed.images.first());
    let shape = Some(video)
        .filter(|video| video.width > 0 && video.height > 0)
        .or(still);
    let (width, height) = shape.map_or(max, |shape| fit_in(shape, max, carded));
    let (width, height) = (width.max(VIDEO_MIN.0), height.max(VIDEO_MIN.1));
    if let Some(url) = images::trusted_video_url(&video.url) {
        let poster = still
            .map(Media::spec)
            .unwrap_or_else(|| images::ImageSpec::VideoPoster {
                id: key(VIDEO_SLOT),
                url: video.url.clone(),
                width: video.width,
                height: video.height,
            });
        let player = crate::media::player::video_box(
            app,
            crate::media::player::Video {
                key: key(VIDEO_SLOT),
                url: &url,
                poster: Some(poster),
                width,
                height,
                radius: MEDIA_RADIUS,
                fullscreen: false,
            },
            cx,
        );
        return Some((player, width));
    }
    let badge = play_badge(
        image_box(still?, width, height, MEDIA_RADIUS, cx),
        embed.url.is_some(),
    );
    let element = match embed.url.clone() {
        Some(url) => div()
            .id(("embed-video", key(63)))
            .cursor_pointer()
            .on_click(move |_, _, cx| cx.open_url(&url))
            .child(badge)
            .into_any_element(),
        None => badge.into_any_element(),
    };
    Some((element, width))
}

fn gallery_grid(
    app: &DiscordApp,
    media: &[Media],
    width: f32,
    height: f32,
    key: &impl Fn(u64) -> u64,
    cx: &App,
) -> AnyElement {
    let column = ((width - GALLERY_GAP) / 2.).floor();
    let half = ((height - GALLERY_GAP) / 2.).floor();
    let entries: Rc<[LightboxEntry]> = media
        .iter()
        .map(|media| LightboxEntry {
            spec: media.spec(),
            animated: animated_media(media),
            size: if media.width > 0 && media.height > 0 {
                (media.width, media.height)
            } else {
                (column.round().max(1.) as u32, half.round().max(1.) as u32)
            },
            preview: theme::image_px(column.max(height)),
            url: media.source.clone(),
        })
        .collect();
    let (left, right) = match media.len().min(4) {
        2 => (vec![(0, height)], vec![(1, height)]),
        3 => (vec![(0, height)], vec![(1, half), (2, half)]),
        _ => (vec![(0, half), (2, half)], vec![(1, half), (3, half)]),
    };
    let side = |tiles: Vec<(usize, f32)>| {
        div()
            .flex()
            .flex_col()
            .gap(rpx(GALLERY_GAP))
            .children(tiles.into_iter().map(|(slot, tile_height)| {
                let entity = app.entity.clone();
                let entries = entries.clone();
                image_box(&media[slot], column, tile_height, MEDIA_RADIUS, cx)
                    .id(("lightbox-open", key(IMAGE_GRID_SLOT + slot as u64)))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        entity
                            .update(cx, |app, cx| {
                                app.open_lightbox_gallery(entries.clone(), slot, cx)
                            })
                            .ok();
                    })
            }))
    };
    div()
        .flex()
        .gap(rpx(GALLERY_GAP))
        .child(side(left))
        .child(side(right))
        .into_any_element()
}

pub(crate) fn mosaic(
    count: usize,
    width: f32,
    (lead_width, lead_height): (u32, u32),
    mut tile: impl FnMut(usize, f32, f32) -> AnyElement,
) -> Div {
    const GAP: f32 = 4.;
    const ROW_MAX: f32 = 280.;
    const BLOCK_MAX: f32 = 350.;
    let half = ((width - GAP) / 2.).floor();
    let third = ((width - GAP * 2.) / 3.).floor();
    let row = || div().flex().gap(rpx(GAP));
    let column = || div().flex().flex_col().gap(rpx(GAP)).w(rpx(width));
    match count {
        3 => {
            let solo = ((width - GAP) * 2. / 3.).floor();
            let duo = width - GAP - solo;
            let height = BLOCK_MAX.min(solo).floor();
            let stacked = ((height - GAP) / 2.).floor();
            return column().child(
                row().child(tile(0, solo, height)).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(GAP))
                        .child(tile(1, duo, stacked))
                        .child(tile(2, duo, stacked)),
                ),
            );
        }
        4 => {
            let height = ((BLOCK_MAX - GAP) / 2.).min(half).floor();
            return column()
                .child(
                    row()
                        .child(tile(0, half, height))
                        .child(tile(1, half, height)),
                )
                .child(
                    row()
                        .child(tile(2, half, height))
                        .child(tile(3, half, height)),
                );
        }
        _ => {}
    }
    let leading = if count == 2 { 2 } else { count % 3 };
    let mut rows = Vec::new();
    match leading {
        1 => {
            let natural = if lead_width > 0 && lead_height > 0 {
                width * lead_height as f32 / lead_width as f32
            } else {
                width * 9. / 16.
            };
            rows.push(row().child(tile(0, width, ROW_MAX.min(natural).floor())));
        }
        2 => {
            let height = ROW_MAX.min(half).floor();
            rows.push(
                row()
                    .child(tile(0, half, height))
                    .child(tile(1, half, height)),
            );
        }
        _ => {}
    }
    let mut slot = leading;
    while slot < count {
        let mut thirds = row();
        for slot in slot..(slot + 3).min(count) {
            thirds = thirds.child(tile(slot, third, third));
        }
        rows.push(thirds);
        slot += 3;
    }
    column().children(rows)
}
