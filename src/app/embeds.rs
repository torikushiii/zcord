use std::borrow::Cow;

use gpui::{prelude::*, *};

#[cfg(test)]
mod tests;

use crate::{
    app::{
        DiscordApp,
        chat::file_size,
        emoji_picker::PickerTarget,
        message_body::{BodyColors, BodyContext, render_body, wrapped_label},
    },
    media::images,
    store::{Component, Embed, EmbedField, EmbedKind, GalleryItem, Media, Reaction, ReactionEmoji},
    text::emoji,
    theme::{self, WithoutTextSelection, rpx},
};

const EMBED_WIDTH: f32 = 516.;
const EMBED_PADDING_LEFT: f32 = 15.;
const EMBED_PADDING_RIGHT: f32 = 16.;
const EMBED_HORIZONTAL_INSET: f32 = EMBED_PADDING_LEFT + EMBED_PADDING_RIGHT + 2.;
const CONTAINER_WIDTH: f32 = 600.;
const EMBED_MEDIA: (f32, f32) = (400., 300.);
const PLAIN_MEDIA: (f32, f32) = (550., 350.);
const THUMBNAIL: f32 = 80.;
const SECTION_THUMBNAIL: f32 = 85.;
const LINK: u32 = theme::LINK;
const EMBED_BORDER: u32 = 0x2E2E36;

const DESCRIPTION_SLOT: u64 = 0;
const FIELD_SLOT: u64 = 1;
const EMBED_SLOTS: u64 = 64;
const COMPONENT_SLOT: u64 = 1 << 20;
const SPOILER_SLOT: u64 = 1 << 21;
const VIDEO_SLOT: u64 = 1 << 22;
const IMAGE_SLOT: u64 = 50;
const SIDE_THUMBNAIL_SLOT: u64 = 51;
const IMAGE_GRID_SLOT: u64 = 52;
/// Free slot for forwarded content. Its mask has the top bit set, so a key derived
/// from a snowflake is never itself a snowflake.
pub(super) const FORWARD_SLOT: u64 = 0x50_0000;

pub(super) fn part_key(message_id: u64, slot: u64) -> u64 {
    message_id ^ slot.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

pub use crate::store::hides_embed_content as hides_content;

pub(super) fn visible(embed: &Embed, content: &str) -> bool {
    crate::settings::link_previews() || !link_preview(&embed.kind, embed.url.as_deref(), content)
}

fn link_preview(kind: &EmbedKind, url: Option<&str>, content: &str) -> bool {
    if !matches!(
        kind,
        EmbedKind::Rich | EmbedKind::Link | EmbedKind::Article | EmbedKind::Video
    ) {
        return false;
    }
    let Some(url) = url else {
        return false;
    };
    content
        .split_whitespace()
        .any(|word| url_matches(word, url))
}

fn url_matches(word: &str, url: &str) -> bool {
    let word = word.trim_matches(|character: char| {
        matches!(
            character,
            '<' | '>'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '"'
                | '\''
                | ','
                | '.'
                | '!'
                | '?'
                | ';'
        )
    });
    word == url || word.trim_end_matches('/') == url.trim_end_matches('/')
}

fn markdown(
    app: &DiscordApp,
    key: u64,
    content: &str,
    size: f32,
    line: f32,
    cx: &App,
) -> AnyElement {
    let parsed = app.parsed(key, content);
    let context = BodyContext {
        store: &app.store,
        guild_id: app.selected_guild_id,
        message_id: key,
        revealed: &app.revealed_spoilers,
        entity: app.entity.clone(),
        colors: BodyColors {
            quote: Some(theme::MUTED),
            ..Default::default()
        },
    };
    div()
        .w_full()
        .min_w_0()
        .text_size(rpx(size))
        .line_height(rpx(line))
        .child(render_body(&parsed.blocks, false, false, &context, cx))
        .into_any_element()
}

fn linked(element: Stateful<Div>, url: Option<&String>) -> Stateful<Div> {
    match url {
        Some(url) => {
            let url = url.clone();
            element
                .cursor_pointer()
                .hover(|style| style.underline())
                .on_click(move |_, _, cx| cx.open_url(&url))
        }
        None => element,
    }
}

fn description_text(text: &str) -> Cow<'_, str> {
    // FxEmbed puts unattached variation selectors at line starts, including after `> `. Some
    // font backends give them width; selectors attached to a real glyph stay.
    let selector = |c: char| matches!(c, '\u{FE00}'..='\u{FE0F}');
    let unattached = |previous: Option<char>| previous.is_none_or(char::is_whitespace);
    let mut previous = None;
    if !text.chars().any(|c| {
        let stray = selector(c) && unattached(previous);
        previous = Some(c);
        stray
    }) {
        return Cow::Borrowed(text);
    }
    let mut kept = String::with_capacity(text.len());
    let mut previous = None;
    for c in text.chars() {
        // A dropped selector leaves `previous` alone, so a whole run of them goes.
        if selector(c) && unattached(previous) {
            continue;
        }
        kept.push(c);
        previous = Some(c);
    }
    Cow::Owned(kept)
}

fn embed_card(color: Option<u32>, content_width: Option<f32>) -> Div {
    div()
        .relative()
        .self_start()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(rpx(8.))
        .mt_1()
        .map(|this| match content_width {
            // Resolve the card width before its percent-width text row is measured.
            Some(width) => this.w(rpx(width + EMBED_HORIZONTAL_INSET)).max_w_full(),
            None => this.max_w(rpx(EMBED_WIDTH.min(super::chat::media_limit()))),
        })
        .font_weight(FontWeight::NORMAL)
        .text_color(rgb(theme::TEXT))
        .pt(rpx(8.))
        .pr(rpx(EMBED_PADDING_RIGHT))
        .pb(rpx(16.))
        // 1px border + 15px = Discord's 4px accent + 12px gutter.
        .pl(rpx(EMBED_PADDING_LEFT))
        .rounded(rpx(4.))
        .border_1()
        .border_color(rgba(theme::BORDER))
        .bg(rgb(theme::ELEVATED))
        .child(
            div()
                .absolute()
                .left(px(-1.))
                .top(px(-1.))
                .bottom(px(-1.))
                .w(rpx(4.))
                .rounded_l(rpx(4.))
                .bg(rgb(color.unwrap_or(EMBED_BORDER))),
        )
}

fn fit(media: &Media, max: (f32, f32)) -> (f32, f32) {
    let room = super::chat::media_limit()
        - if max == PLAIN_MEDIA {
            0.
        } else {
            EMBED_HORIZONTAL_INSET
        };
    let max = (max.0.min(room), max.1);
    if media.width == 0 || media.height == 0 {
        return max;
    }
    let (width, height) = (media.width as f32, media.height as f32);
    let scale = (max.0 / width).min(max.1 / height).min(1.);
    (
        (width * scale).round().max(1.),
        (height * scale).round().max(1.),
    )
}

/// Like Discord, a card with text fits its content (up to the embed width), so neither short
/// text beside wide media nor long text beside portrait media is forced to the media's width.
fn card_content_width(embed: &Embed, media: (f32, f32)) -> Option<f32> {
    let has_text = embed.title.is_some()
        || embed.description.is_some()
        || embed.author.is_some()
        || !embed.fields.is_empty();
    (!has_text).then_some(media.0)
}

fn embed_content_width(
    embed: &Embed,
    side_thumbnail: bool,
    media: Option<(f32, f32)>,
) -> Option<f32> {
    if !embed.fields.is_empty() {
        // Inline fields need a resolved width before their percent-width children measure.
        Some(EMBED_WIDTH - EMBED_HORIZONTAL_INSET)
    } else if side_thumbnail {
        None
    } else {
        media.and_then(|dimensions| card_content_width(embed, dimensions))
    }
}

fn animated_id(key: &str) -> (&'static str, u64) {
    let mut hasher = std::hash::DefaultHasher::new();
    std::hash::Hash::hash(key, &mut hasher);
    ("animated", std::hash::Hasher::finish(&hasher))
}

pub(super) fn animated(source: impl Into<ImageSource>, key: &str) -> Stateful<Img> {
    img(source).id(animated_id(key))
}

/// Animated media is served as GIF/WebP/APNG files; the spec's `animated` flag is
/// only set for some embed providers, so the extension is the fallback signal.
pub(super) fn animated_media(media: &Media) -> bool {
    media.animated
        || media
            .url
            .split('?')
            .next()
            .is_some_and(|path| path.to_ascii_lowercase().ends_with(".gif"))
}

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
            this.bg(rgba(theme::BORDER))
                .when(!images::failed(&spec, pixels, cx), |this| {
                    this.child(theme::media_loading())
                })
        })
        .children(child)
}

fn gifv_box(
    app: &DiscordApp,
    key: u64,
    still: &Media,
    video: Option<&Media>,
    cx: &App,
) -> Stateful<Div> {
    let (width, height) = fit(
        video.filter(|video| video.width > 0).unwrap_or(still),
        PLAIN_MEDIA,
    );
    let pixels = theme::image_px(width.max(height));
    let playing = video.and_then(|video| {
        crate::media::video::resolve(&video.video_spec(), pixels, cx).map(|frames| (video, frames))
    });
    let element = match playing {
        Some((video, frames)) => div()
            .flex_none()
            .w(rpx(width))
            .h(rpx(height))
            .rounded(rpx(8.))
            .overflow_hidden()
            .child(
                crate::media::animated::image(frames)
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded(rpx(8.))
                    .id(animated_id(&video.url)),
            ),
        None => image_box(still, width, height, 8., cx),
    };
    element
        .id(("lightbox-open", key))
        .cursor_pointer()
        .without_text_selection()
        .on_click(super::lightbox::gifv_opener(app, still, video, pixels))
}

fn fitted(app: &DiscordApp, key: u64, media: &Media, max: (f32, f32), cx: &App) -> Stateful<Div> {
    let (width, height) = fit(media, max);
    lightbox_box(app, key, media, width, height, 8., cx)
}

#[allow(clippy::too_many_arguments)]
fn lightbox_box(
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

fn spoiler_cover(app: &DiscordApp, key: u64, spoiler: bool, element: Div) -> AnyElement {
    if !spoiler || app.revealed_spoilers.contains(&(key, 0)) {
        return element.into_any_element();
    }
    element
        .child(spoiler_overlay(app, key, 8.).bg(rgb(0x1A1A1E)))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn spoiler_media(
    app: &DiscordApp,
    key: u64,
    spoiler: bool,
    media: &Media,
    width: f32,
    height: f32,
    radius: f32,
    cx: &App,
) -> Stateful<Div> {
    if spoiler && !app.revealed_spoilers.contains(&(key, 0)) {
        blurred_box(&media.spec(), width, height, radius, cx)
            .id(("hidden-media", key))
            .child(spoiler_overlay(app, key, radius))
    } else {
        lightbox_box(app, key, media, width, height, radius, cx)
    }
}

pub(super) fn blurred_box(
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
        // The placeholder would show as a pale rim around the image's rounded edge.
        .when(image.is_none(), |this| this.bg(rgba(theme::BORDER)))
        // GPUI clips children to the parent's rectangle, not its rounded corners,
        // so the image must carry the radius itself and stay inside the box.
        .children(image.map(|image| {
            img(image)
                .absolute()
                .inset_0()
                .size_full()
                .rounded(rpx(radius))
                .object_fit(ObjectFit::Cover)
        }))
}

pub(super) fn spoiler_overlay(app: &DiscordApp, key: u64, radius: f32) -> Stateful<Div> {
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
                .text_color(rgb(theme::STRONG))
                .child("SPOILER"),
        )
}

/// A play button over a video thumbnail. Linked videos get Discord's pill with an extra
/// open-in-browser icon.
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
            .child(theme::icon("play", 22., theme::STRONG))
            .child(theme::icon("external-link", 22., theme::STRONG))
    } else {
        badge
            .size(rpx(48.))
            .rounded_full()
            .child(theme::icon("play", 20., theme::STRONG))
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

fn icon_image(media: Option<&Media>, size: f32, cx: &App) -> Option<AnyElement> {
    let media = media?;
    let image = images::resolve(&media.spec(), theme::image_px(size), cx);
    Some(
        div()
            .size(rpx(size))
            .flex_none()
            .rounded_full()
            .overflow_hidden()
            .bg(rgba(theme::BORDER))
            .children(image.map(|path| img(path).size_full().rounded_full()))
            .into_any_element(),
    )
}

pub fn render_embed(
    app: &DiscordApp,
    message_id: u64,
    index: usize,
    embed: &Embed,
    cx: &App,
) -> AnyElement {
    let key = |slot: u64| part_key(message_id, index as u64 * EMBED_SLOTS + slot);
    if embed.kind == EmbedKind::Components {
        // Link-preview components live inside embeds, not message.components. Give
        // each embed its own keys for markdown, spoilers, media, and link controls.
        return render_components(app, key(COMPONENT_SLOT), &embed.components, cx)
            .unwrap_or_else(|| div().into_any_element());
    }
    let media = embed.thumbnail.as_ref().or(embed.images.first());
    let bare = embed.title.is_none() && embed.description.is_none() && embed.author.is_none();
    match embed.kind {
        EmbedKind::Gifv => {
            let Some(media) = media else {
                return div().into_any_element();
            };
            return div()
                .flex()
                .flex_col()
                .mt_1()
                .child(gifv_box(
                    app,
                    key(IMAGE_SLOT),
                    media,
                    embed.video.as_ref(),
                    cx,
                ))
                .child(
                    div()
                        .mt(rpx(4.))
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(theme::MUTED))
                        .child("GIF"),
                )
                .into_any_element();
        }
        EmbedKind::Image if bare => {
            let Some(media) = media else {
                return div().into_any_element();
            };
            return div()
                .mt_1()
                .child(fitted(app, key(IMAGE_SLOT), media, PLAIN_MEDIA, cx))
                .into_any_element();
        }
        _ => {}
    }

    // Mastodon-style embeds (e.g. FxTwitter) carry a playable `video` without being
    // typed as video embeds, so a trusted video makes the embed a video one.
    let video_url = embed
        .video
        .as_ref()
        .and_then(|media| images::trusted_video_url(&media.url));
    let video = embed.kind == EmbedKind::Video || video_url.is_some();
    let side_thumbnail = embed
        .thumbnail
        .as_ref()
        .filter(|_| !video && !matches!(embed.kind, EmbedKind::Image | EmbedKind::Article));
    let big_media: Vec<&Media> = if video {
        embed
            .thumbnail
            .iter()
            .chain(embed.images.first())
            .take(1)
            .collect()
    } else if matches!(embed.kind, EmbedKind::Image | EmbedKind::Article) && embed.images.is_empty()
    {
        embed.thumbnail.iter().collect()
    } else {
        embed.images.iter().collect()
    };
    let video_dimensions = video_url.as_ref().map(|_| {
        embed
            .thumbnail
            .as_ref()
            .or(embed.images.first())
            .or(embed.video.as_ref())
            .map_or(EMBED_MEDIA, |media| fit(media, EMBED_MEDIA))
    });
    let content_width = embed_content_width(
        embed,
        side_thumbnail.is_some(),
        video_dimensions.or_else(|| match big_media.as_slice() {
            [] => None,
            [single] => Some(fit(single, EMBED_MEDIA)),
            _ => Some((
                EMBED_MEDIA
                    .0
                    .min(super::chat::media_limit() - EMBED_HORIZONTAL_INSET),
                EMBED_MEDIA.1,
            )),
        }),
    );

    let mut body = div().flex().flex_col().flex_1().min_w_0().gap(rpx(8.));
    if let Some(provider) = &embed.provider {
        body = body.child(
            linked(
                div()
                    .id(("embed-provider", key(60)))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(theme::MUTED)),
                provider.url.as_ref(),
            )
            .child(wrapped_label(
                ("embed-provider-text", key(60)),
                &provider.text,
            )),
        );
    }
    if let Some(author) = &embed.author {
        body = body.child(
            div()
                .flex()
                .items_center()
                .gap(rpx(8.))
                .min_w_0()
                .children(icon_image(author.icon.as_ref(), 24., cx))
                .child(
                    linked(
                        div()
                            .id(("embed-author", key(61)))
                            .min_w_0()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::STRONG)),
                        author.url.as_ref(),
                    )
                    .child(wrapped_label(("embed-author-text", key(61)), &author.text)),
                ),
        );
    }
    if let Some(title) = &embed.title {
        let url = title.url.as_ref().or(embed.url.as_ref());
        body = body.child(
            linked(
                div()
                    .id(("embed-title", key(62)))
                    .text_size(rpx(16.))
                    .line_height(rpx(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if url.is_some() { LINK } else { theme::STRONG })),
                url,
            )
            .child(wrapped_label(("embed-title-text", key(62)), &title.text)),
        );
    }
    // Discord leaves the description off video embeds such as YouTube's.
    if let Some(description) = embed
        .description
        .as_ref()
        .filter(|_| embed.kind != EmbedKind::Video)
    {
        let description = description_text(description);
        body = body.child(markdown(
            app,
            key(DESCRIPTION_SLOT),
            &description,
            14.,
            18.,
            cx,
        ));
    }
    if !embed.fields.is_empty() {
        body = body.child(fields(
            app,
            &embed.fields,
            |n| key(FIELD_SLOT + n as u64),
            cx,
        ));
    }

    let content = div()
        .flex()
        .gap(rpx(16.))
        .w_full()
        .child(body)
        .children(side_thumbnail.map(|thumbnail| {
            let (width, height) = fit(thumbnail, (THUMBNAIL, THUMBNAIL));
            lightbox_box(
                app,
                key(SIDE_THUMBNAIL_SLOT),
                thumbnail,
                width,
                height,
                4.,
                cx,
            )
        }));

    let media_max = EMBED_MEDIA;
    let media = if let Some(url) = video_url.as_deref() {
        // Direct-video links (e.g. kkinstagram) come without a thumbnail; the media proxy
        // renders the video's first frame instead.
        let poster = embed
            .thumbnail
            .as_ref()
            .or(embed.images.first())
            .map(Media::spec)
            .or_else(|| {
                embed
                    .video
                    .as_ref()
                    .map(|video| images::ImageSpec::VideoPoster {
                        id: key(VIDEO_SLOT),
                        url: video.url.clone(),
                        width: video.width,
                        height: video.height,
                    })
            });
        let dimensions = video_dimensions.expect("playable video has dimensions");
        let player = crate::media::player::video_box(
            app,
            crate::media::player::Video {
                key: key(VIDEO_SLOT),
                url,
                poster,
                width: dimensions.0,
                height: dimensions.1,
                radius: 8.,
                fullscreen: false,
            },
            cx,
        );
        // Discord drops the card around a video with nothing else to show.
        if embed.provider.is_none()
            && embed.title.is_none()
            && embed.description.is_none()
            && embed.author.is_none()
            && embed.fields.is_empty()
            && embed.footer.is_none()
        {
            return div().mt_1().child(player).into_any_element();
        }
        Some(player)
    } else {
        match big_media.as_slice() {
            [] => None,
            [single] if video => {
                // The thumbnail can be letterboxed (YouTube's hqdefault is 4:3 with bars), so
                // size the box from the video and let the thumbnail cover it.
                let shape = embed
                    .video
                    .as_ref()
                    .filter(|video| video.width > 0 && video.height > 0)
                    .unwrap_or(single);
                let (width, height) = fit(shape, media_max);
                let pill = embed.url.is_some();
                Some(play_badge(image_box(single, width, height, 8., cx), pill).into_any_element())
            }
            [single] => {
                Some(fitted(app, key(IMAGE_SLOT), single, media_max, cx).into_any_element())
            }
            many => Some(
                media_grid(
                    app,
                    many,
                    media_max.0,
                    |slot| key(IMAGE_GRID_SLOT + slot as u64),
                    cx,
                )
                .into_any_element(),
            ),
        }
    };
    let media = media.map(
        |media| match (video, video_url.is_none(), embed.url.clone()) {
            (true, true, Some(url)) => div()
                .id(("embed-video", key(63)))
                .cursor_pointer()
                .on_click(move |_, _, cx| cx.open_url(&url))
                .child(media)
                .into_any_element(),
            _ => media,
        },
    );

    let footer = embed.footer.as_ref().map(|footer| footer.text.clone());
    let stamp = embed.timestamp.map(|timestamp| timestamp.full());
    let footer_text = match (footer, stamp) {
        (Some(text), Some(stamp)) => Some(format!("{text} • {stamp}")),
        (text, stamp) => text.or(stamp),
    };

    embed_card(embed.color, content_width)
        .child(content)
        .children(media)
        .children(footer_text.map(|text| {
            div()
                .flex()
                .items_center()
                .gap(rpx(8.))
                .children(icon_image(
                    embed
                        .footer
                        .as_ref()
                        .and_then(|footer| footer.icon.as_ref()),
                    20.,
                    cx,
                ))
                .child(
                    div()
                        .min_w_0()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::MUTED))
                        .child(wrapped_label(("embed-footer-text", key(59)), &text)),
                )
        }))
        .into_any_element()
}

fn fields(app: &DiscordApp, fields: &[EmbedField], key: impl Fn(usize) -> u64, cx: &App) -> Div {
    field_grid(fields, |index, field| {
        div()
            .flex()
            .flex_col()
            .gap(rpx(2.))
            .child(
                div()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::STRONG))
                    .child(wrapped_label(
                        ("embed-field-name", key(index) ^ 1),
                        &field.name,
                    )),
            )
            .child(markdown(app, key(index), &field.value, 14., 18., cx))
            .into_any_element()
    })
}

fn field_grid(fields: &[EmbedField], render: impl Fn(usize, &EmbedField) -> AnyElement) -> Div {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        match rows.last_mut() {
            Some(row) if field.inline && row.len() < 3 && fields[row[0]].inline => row.push(index),
            _ => rows.push(vec![index]),
        }
    }
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .w_full()
        .children(rows.into_iter().map(|row| {
            div()
                .flex()
                .flex_wrap()
                .gap(rpx(8.))
                .w_full()
                .min_w_0()
                .children(row.into_iter().map(|index| {
                    div()
                        .flex_1()
                        .min_w(rpx(96.))
                        .max_w_full()
                        .child(render(index, &fields[index]))
                }))
        }))
}

fn media_grid(
    app: &DiscordApp,
    media: &[&Media],
    width: f32,
    key: impl Fn(usize) -> u64,
    cx: &App,
) -> Div {
    mosaic(media.len(), width, media[0], |slot, w, h| {
        lightbox_box(app, key(slot), media[slot], w, h, 4., cx).into_any_element()
    })
}

/// Discord's media mosaic (MediaMosaic CSS): two side by side, three as a large tile beside a
/// stacked pair, four as a 2x2 grid, and more as rows of square thirds led by a full-width
/// tile or a pair when the count isn't a multiple of three.
fn mosaic(
    count: usize,
    width: f32,
    lead: &Media,
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
            let natural = if lead.width > 0 && lead.height > 0 {
                width * lead.height as f32 / lead.width as f32
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

pub fn render_components(
    app: &DiscordApp,
    message_id: u64,
    components: &[Component],
    cx: &App,
) -> Option<AnyElement> {
    if components.is_empty() {
        return None;
    }
    let mut counter = 0;
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(8.))
            .mt_1()
            .w_full()
            .children(
                components.iter().map(|component| {
                    render_component(app, message_id, component, &mut counter, cx)
                }),
            )
            .into_any_element(),
    )
}

fn render_component(
    app: &DiscordApp,
    message_id: u64,
    component: &Component,
    counter: &mut u64,
    cx: &App,
) -> AnyElement {
    *counter += 1;
    let slot = *counter;
    match component {
        Component::Row(children) => div()
            .flex()
            .flex_wrap()
            .gap(rpx(8.))
            .children(
                children
                    .iter()
                    .map(|child| render_component(app, message_id, child, counter, cx)),
            )
            .into_any_element(),
        Component::Button {
            style,
            label,
            emoji,
            url,
            disabled,
        } => button(
            part_key(message_id, COMPONENT_SLOT + slot),
            *style,
            label.as_deref(),
            emoji.as_ref(),
            url.as_ref(),
            *disabled,
            cx,
        ),
        Component::Select {
            placeholder,
            disabled,
        } => div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .max_w(rpx(400.))
            .h(rpx(40.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .border_1()
            .border_color(rgba(theme::BORDER))
            .bg(rgb(theme::BG))
            .when(*disabled, |this| this.opacity(0.5))
            .child(
                div()
                    .truncate()
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::MUTED))
                    .child(
                        placeholder
                            .clone()
                            .unwrap_or_else(|| "Make a selection".into()),
                    ),
            )
            .child(theme::icon("chevron-down", 18., theme::MUTED))
            .into_any_element(),
        Component::Section {
            components,
            accessory,
        } => div()
            .flex()
            .items_start()
            .gap(rpx(16.))
            .w_full()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(rpx(8.))
                    .children(
                        components
                            .iter()
                            .map(|child| render_component(app, message_id, child, counter, cx)),
                    ),
            )
            .children(accessory.as_deref().map(|accessory| match accessory {
                Component::Thumbnail { media, spoiler, .. } => {
                    *counter += 1;
                    let key = part_key(message_id, SPOILER_SLOT + *counter);
                    let (width, height) = fit(media, (SECTION_THUMBNAIL, SECTION_THUMBNAIL));
                    let (width, height) = if media.width == 0 {
                        (SECTION_THUMBNAIL, SECTION_THUMBNAIL)
                    } else {
                        (width, height)
                    };
                    spoiler_media(app, key, *spoiler, media, width, height, 8., cx)
                        .into_any_element()
                }
                other => render_component(app, message_id, other, counter, cx),
            }))
            .into_any_element(),
        Component::Text(content) => markdown(
            app,
            part_key(message_id, COMPONENT_SLOT + slot),
            content,
            14.,
            20.,
            cx,
        ),
        Component::Thumbnail { media, spoiler, .. } => {
            let (width, height) = fit(media, (SECTION_THUMBNAIL, SECTION_THUMBNAIL));
            spoiler_media(
                app,
                part_key(message_id, SPOILER_SLOT + slot),
                *spoiler,
                media,
                width,
                height,
                8.,
                cx,
            )
            .into_any_element()
        }
        Component::Gallery(items) => gallery(app, message_id, items, counter, cx),
        Component::File {
            name,
            size,
            spoiler,
        } => div()
            .flex()
            .items_center()
            .gap_3()
            .max_w(rpx(432.))
            .p_3()
            .rounded(rpx(8.))
            .bg(rgb(theme::BG))
            .border_1()
            .border_color(rgba(theme::BORDER))
            .when(*spoiler, |this| this.opacity(0.6))
            .child(theme::icon("file", 32., theme::MUTED))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(div().truncate().text_color(rgb(LINK)).child(name.clone()))
                    .child(
                        div()
                            .text_size(rpx(12.))
                            .text_color(rgb(theme::FAINT))
                            .child(file_size(*size)),
                    ),
            )
            .into_any_element(),
        Component::Separator { divider, large } => {
            let space = if *large { 16. } else { 8. };
            div()
                .w_full()
                .py(rpx(space / 2.))
                .when(*divider, |this| {
                    this.child(div().w_full().h(px(1.)).bg(rgba(theme::BORDER)))
                })
                .into_any_element()
        }
        Component::Container {
            accent,
            spoiler,
            components,
        } => {
            let key = part_key(message_id, SPOILER_SLOT + slot);
            let element = component_container(
                *accent,
                components
                    .iter()
                    .map(|child| render_component(app, message_id, child, counter, cx)),
            );
            spoiler_cover(app, key, *spoiler, element)
        }
    }
}

fn component_container(accent: Option<u32>, children: impl IntoIterator<Item = AnyElement>) -> Div {
    // Resolve the constrained column width before measuring long wrapped text.
    // Nested flex rows can retain a height measured at the wider message width.
    div()
        .relative()
        .grid()
        .grid_cols(1)
        .flex_shrink_0()
        .w_full()
        .max_w(rpx(CONTAINER_WIDTH))
        .gap(rpx(8.))
        .p(rpx(16.))
        .when(accent.is_some(), |this| this.pl(rpx(20.)))
        .rounded(rpx(8.))
        .overflow_hidden()
        .border_1()
        .border_color(rgba(theme::BORDER))
        .bg(rgb(theme::ELEVATED))
        .children(accent.map(|color| {
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(rpx(4.))
                .bg(rgb(color))
        }))
        .children(children)
}

fn single_gallery_frame(element: Stateful<Div>, width: f32, height: f32) -> Stateful<Div> {
    element.max_w_full().h_auto().aspect_ratio(width / height)
}

fn gallery(
    app: &DiscordApp,
    message_id: u64,
    items: &[GalleryItem],
    counter: &mut u64,
    cx: &App,
) -> AnyElement {
    let mut keyed = Vec::with_capacity(items.len());
    for item in items {
        *counter += 1;
        keyed.push((part_key(message_id, SPOILER_SLOT + *counter), item));
    }
    if let [(key, item)] = keyed.as_slice() {
        let (width, height) = fit(&item.media, PLAIN_MEDIA);
        return single_gallery_frame(
            spoiler_media(app, *key, item.spoiler, &item.media, width, height, 8., cx),
            width,
            height,
        )
        .into_any_element();
    }
    let Some((_, lead)) = keyed.first() else {
        return div().into_any_element();
    };
    mosaic(keyed.len(), PLAIN_MEDIA.0, &lead.media, |slot, w, h| {
        let (key, item) = keyed[slot];
        spoiler_media(app, key, item.spoiler, &item.media, w, h, 4., cx).into_any_element()
    })
    .into_any_element()
}

fn button(
    key: u64,
    style: u8,
    label: Option<&str>,
    emoji: Option<&ReactionEmoji>,
    url: Option<&String>,
    disabled: bool,
    cx: &App,
) -> AnyElement {
    let (background, border) = match style {
        1 | 6 => (theme::BLURPLE, theme::BLURPLE),
        3 => (0x00863A, 0x00863A),
        4 => (0xD22D39, 0xD22D39),
        _ => (0x1F1F23, 0x34343B),
    };
    let url = url.filter(|_| style == 5 && !disabled).cloned();
    div()
        .id(("component-button", key))
        .flex()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(32.))
        .px(rpx(16.))
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(background))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(0xFFFFFF))
        .when(disabled, |this| this.opacity(0.5))
        .children(emoji.map(|emoji| emoji_element(emoji, 20., cx)))
        .children(label.map(|label| wrapped_label(("component-label", key), label)))
        .when(style == 5, |this| {
            this.child(theme::icon("external-link", 16., 0xFFFFFF))
        })
        .when_some(url, |this, url| {
            this.cursor_pointer()
                .on_click(move |_, _, cx| cx.open_url(&url))
        })
        .into_any_element()
}

pub(crate) fn emoji_element(emoji: &ReactionEmoji, size: f32, cx: &App) -> AnyElement {
    match emoji.id {
        Some(id) => {
            let spec = images::ImageSpec::Emoji {
                id,
                animated: emoji.animated,
            };
            let pixels = theme::image_px(size);
            let frames = emoji
                .animated
                .then(|| crate::media::animated::resolve(&spec, pixels, cx))
                .flatten();
            let still_spec = images::ImageSpec::Emoji {
                id,
                animated: false,
            };
            let child = match frames {
                Some(image) => Some(
                    crate::media::animated::image(image)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .id(("emoji", id))
                        .into_any_element(),
                ),
                None => images::resolve(&still_spec, pixels, cx).map(|path| {
                    img(path)
                        .id(("emoji", id))
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element()
                }),
            };
            div()
                .size(rpx(size))
                .flex_none()
                .children(child)
                .into_any_element()
        }
        None => match emoji::image(&emoji.name) {
            Some(image) => img(image)
                .size(rpx(size))
                .flex_none()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => div()
                .flex_none()
                .text_size(rpx(size * 0.85))
                .line_height(rpx(size))
                .child(emoji.name.clone())
                .into_any_element(),
        },
    }
}

pub fn render_reactions(
    app: &DiscordApp,
    message_id: u64,
    reactions: &[Reaction],
    cx: &App,
) -> Option<AnyElement> {
    if reactions.is_empty() {
        return None;
    }
    let entity = app.entity.clone();
    let open = app.entity.clone();
    let tip_shown = app.hovered_add_reaction == Some(message_id);
    let picking = app.picking_reaction_for(message_id);
    let pill = || {
        div()
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(30.))
            .px(rpx(8.))
            .rounded(rpx(8.))
            .border_1()
    };
    Some(
        div()
            .flex()
            .flex_wrap()
            .gap(rpx(4.))
            .mt(rpx(4.))
            .children(reactions.iter().enumerate().map(|(index, reaction)| {
                let entity = entity.clone();
                let emoji = reaction.emoji.clone();
                pill()
                    .id(format!("reaction-{message_id}-{index}"))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        entity
                            .update(cx, |app, cx| {
                                app.toggle_reaction(message_id, emoji.clone(), cx)
                            })
                            .ok();
                    })
                    .map(|this| {
                        if reaction.me {
                            this.bg(rgba(0x5865F226)).border_color(rgb(theme::BLURPLE))
                        } else {
                            this.bg(rgb(theme::ELEVATED))
                                .border_color(rgb(theme::ELEVATED))
                        }
                    })
                    .child(emoji_element(&reaction.emoji, 20., cx))
                    .child(
                        div()
                            .text_size(rpx(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(if reaction.me { 0xC9CDFB } else { theme::MUTED }))
                            .child(reaction.count.to_string()),
                    )
            }))
            .child(
                pill()
                    .id(("add-reaction", message_id))
                    .bg(rgb(theme::ELEVATED))
                    .border_color(rgb(theme::ELEVATED))
                    .opacity(if picking { 1. } else { 0. })
                    .group_hover("message", |style| style.opacity(1.))
                    .hover(|style| style.bg(rgba(theme::SELECTED)))
                    .cursor_pointer()
                    .relative()
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        let position = event.position - point(px(0.), px(8.));
                        open.update(cx, |app, cx| {
                            app.toggle_emoji_picker(
                                PickerTarget::Reaction(message_id),
                                position,
                                window,
                                cx,
                            )
                        })
                        .ok();
                    })
                    .on_hover(move |hovered, _, cx| {
                        entity
                            .update(cx, |app, cx| {
                                app.set_add_reaction_hover(message_id, *hovered, cx)
                            })
                            .ok();
                    })
                    .child(theme::icon("smile-plus", 20., theme::MUTED))
                    .when(tip_shown && !picking, |this| {
                        this.child(theme::tooltip_above("Add Reaction"))
                    }),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod link_preview_tests {
    use super::link_preview;
    use crate::store::EmbedKind;

    #[test]
    fn url_embeds_are_classified_as_link_previews() {
        assert!(link_preview(
            &EmbedKind::Rich,
            Some("https://example.com/story"),
            "read https://example.com/story for details"
        ));
        assert!(link_preview(
            &EmbedKind::Article,
            Some("https://example.com/story"),
            "read (https://example.com/story)."
        ));
        assert!(link_preview(
            &EmbedKind::Video,
            Some("https://example.com/clip"),
            "<https://example.com/clip>"
        ));
        assert!(link_preview(
            &EmbedKind::Link,
            Some("https://example.com/story/"),
            "https://example.com/story"
        ));
    }

    #[test]
    fn bot_rich_embeds_and_non_link_kinds_are_kept() {
        assert!(!link_preview(
            &EmbedKind::Rich,
            Some("https://example.com/story"),
            "a bot-authored embed"
        ));
        assert!(!link_preview(
            &EmbedKind::Rich,
            None,
            "https://example.com/story"
        ));
        for kind in [EmbedKind::Image, EmbedKind::Gifv, EmbedKind::Components] {
            assert!(!link_preview(
                &kind,
                Some("https://example.com/story"),
                "https://example.com/story"
            ));
        }
    }
}
