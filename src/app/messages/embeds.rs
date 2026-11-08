mod components;
mod fields;
mod media;
mod reactions;

pub use components::render_components;
use fields::fields;
pub(crate) use media::{blurred_box, mosaic, spoiler_overlay};
use media::{embed_media, icon_image, lightbox_box};
pub(crate) use reactions::emoji_element;
pub use reactions::render_reactions;

use std::borrow::Cow;
use std::rc::Rc;

use gpui::{prelude::*, *};
use gpui_component::ElementExt as _;

#[cfg(test)]
mod tests;

use crate::{
    app::{
        DiscordApp,
        chat::file_size,
        expressions::emoji_picker::PickerTarget,
        lightbox::LightboxEntry,
        messages::message_body::{BodyColors, BodyContext, render_body, wrapped_label},
    },
    media::images,
    store::{Component, Embed, EmbedField, EmbedKind, GalleryItem, Media, Reaction, ReactionEmoji},
    text::emoji,
    theme::{self, WithoutTextSelection, rpx},
};

const EMBED_WIDTH: f32 = 521.;
const EMBED_PADDING_LEFT: f32 = 15.;
const EMBED_PADDING_RIGHT: f32 = 16.;
const EMBED_HORIZONTAL_INSET: f32 = EMBED_PADDING_LEFT + EMBED_PADDING_RIGHT + 2.;
const EMBED_MEDIA: (f32, f32) = (400., 300.);
const TALL_EMBED_MEDIA: (f32, f32) = (400., 450.);
const INLINE_MEDIA_WIDTH: f32 = 520.;
const GALLERY_CARD_WIDTH: f32 = 520.;
const MEDIA_RADIUS: f32 = 4.;
const PLAIN_MEDIA: (f32, f32) = (550., 350.);
const THUMBNAIL: f32 = 80.;

const DESCRIPTION_SLOT: u64 = 0;
const FIELD_SLOT: u64 = 1;
const EMBED_SLOTS: u64 = 64;
const COMPONENT_SLOT: u64 = 1 << 20;
const SIDE_THUMBNAIL_SLOT: u64 = 51;
const TITLE_SLOT: u64 = 57;
const FIELD_NAME_SLOT: u64 = 1 << 23;
/// Free slot for forwarded content. Its mask has the top bit set, so a key derived
/// from a snowflake is never itself a snowflake.
pub(crate) const FORWARD_SLOT: u64 = 0x50_0000;

pub(crate) fn part_key(message_id: u64, slot: u64) -> u64 {
    message_id ^ slot.wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

pub(crate) fn hides_content(content: &str, embeds: &[Embed]) -> bool {
    crate::settings::display().inline_embed_media
        && crate::store::hides_embed_content(content, embeds)
}

pub(crate) fn visible(embed: &Embed, content: &str) -> bool {
    if matches!(embed.kind, EmbedKind::Image | EmbedKind::Gifv)
        && !crate::settings::display().inline_embed_media
    {
        return false;
    }
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
        reveal_all: app.spoilers_open(),
        entity: app.entity.clone(),
        colors: BodyColors {
            quote: Some(theme::muted()),
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
                .when(crate::settings::underline_links(), |this| this.underline())
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
            Some(width) => this.w(rpx(width + EMBED_HORIZONTAL_INSET)).max_w_full(),
            None => this.max_w_full(),
        })
        .font_weight(FontWeight::NORMAL)
        .text_color(rgb(theme::text()))
        .pt(rpx(10.))
        .pr(rpx(EMBED_PADDING_RIGHT))
        .pb(rpx(16.))
        .pl(rpx(EMBED_PADDING_LEFT))
        .rounded(rpx(4.))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::elevated()))
        .child(
            div()
                .absolute()
                .left(px(-1.))
                .top(px(-1.))
                .bottom(px(-1.))
                .w(rpx(4.))
                .rounded_l(rpx(4.))
                .bg(rgb(color
                    .filter(|&color| color != 0xFFFFFF)
                    .unwrap_or(theme::embed_border()))),
        )
}

fn text_limit() -> f32 {
    EMBED_WIDTH.min(super::super::chat::media_limit()) - EMBED_HORIZONTAL_INSET
}

fn content_row() -> Div {
    div().flex().gap(rpx(16.)).w_full().max_w(rpx(text_limit()))
}

fn fit(media: &Media, max: (f32, f32)) -> (f32, f32) {
    fit_in(media, max, max != PLAIN_MEDIA)
}

fn fit_in(media: &Media, max: (f32, f32), carded: bool) -> (f32, f32) {
    let room = super::super::chat::media_limit() - if carded { EMBED_HORIZONTAL_INSET } else { 0. };
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

fn has_text(embed: &Embed) -> bool {
    embed.title.is_some()
        || embed.description.is_some()
        || embed.author.is_some()
        || !embed.fields.is_empty()
}

fn media_max(embed: &Embed) -> (f32, f32) {
    let provider = embed
        .provider
        .as_ref()
        .map(|provider| provider.text.as_str());
    let youtube = provider == Some("YouTube")
        || embed.video.as_ref().is_some_and(|video| {
            reqwest::Url::parse(&video.source)
                .is_ok_and(|url| url.host_str() == Some("www.youtube.com"))
        });
    let portrait = embed
        .video
        .as_ref()
        .is_some_and(|video| video.height > video.width);
    if provider == Some("TikTok") || (portrait && youtube) {
        TALL_EMBED_MEDIA
    } else {
        EMBED_MEDIA
    }
}

fn inline_media(embed: &Embed) -> bool {
    let media = !embed.images.is_empty() || embed.video.is_some();
    match embed.kind {
        EmbedKind::Gifv => media || embed.thumbnail.is_some(),
        EmbedKind::Rich | EmbedKind::Components => false,
        _ => media && embed.author.is_none() && embed.title.is_none(),
    }
}

fn media_card_width(embed: &Embed, max: (f32, f32)) -> Option<f32> {
    let media = embed.images.first().or(embed.video.as_ref())?;
    let width = fit(media, max).0;
    if embed.kind == EmbedKind::Video || width >= 300. {
        Some(width)
    } else if embed.kind == EmbedKind::Rich && embed.images.len() > 1 {
        Some(GALLERY_CARD_WIDTH - EMBED_HORIZONTAL_INSET)
    } else {
        None
    }
}

fn embed_content_width(embed: &Embed, max: (f32, f32), media_width: Option<f32>) -> Option<f32> {
    media_card_width(embed, max).or_else(|| media_width.filter(|_| !has_text(embed)))
}

fn animated_id(key: &str) -> (&'static str, u64) {
    let mut hasher = std::hash::DefaultHasher::new();
    std::hash::Hash::hash(key, &mut hasher);
    ("animated", std::hash::Hasher::finish(&hasher))
}

pub(crate) fn animated(source: impl Into<ImageSource>, key: &str) -> Stateful<Img> {
    img(source).id(animated_id(key))
}

/// Animated media is served as GIF/WebP/APNG files; the spec's `animated` flag is
/// only set for some embed providers, so the extension is the fallback signal.
pub(crate) fn animated_media(media: &Media) -> bool {
    media.animated
        || media
            .url
            .split('?')
            .next()
            .is_some_and(|path| path.to_ascii_lowercase().ends_with(".gif"))
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
        return render_components(app, key(COMPONENT_SLOT), &embed.components, cx)
            .unwrap_or_else(|| div().into_any_element());
    }
    let max = media_max(embed);
    if inline_media(embed) {
        return div()
            .mt_1()
            .max_w(rpx(INLINE_MEDIA_WIDTH))
            .children(embed_media(app, embed, &key, max, false, cx).map(|(media, _)| media))
            .into_any_element();
    }
    let rich = embed.kind == EmbedKind::Rich;

    let mut body = div().flex().flex_col().flex_1().min_w_0().gap(rpx(8.));
    if let Some(provider) = &embed.provider {
        body = body.child(
            linked(
                div()
                    .id(("embed-provider", key(60)))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(theme::muted())),
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
                            .text_color(rgb(theme::strong())),
                        author.url.as_ref(),
                    )
                    .child(wrapped_label(("embed-author-text", key(61)), &author.text)),
                ),
        );
    }
    if let Some(title) = &embed.title {
        let url = title.url.as_ref().or(embed.url.as_ref());
        let text = if rich {
            markdown(app, key(TITLE_SLOT), &title.text, 16., 22., cx)
        } else {
            wrapped_label(("embed-title-text", key(62)), &title.text)
        };
        body = body.child(
            linked(
                div()
                    .id(("embed-title", key(62)))
                    .text_size(rpx(16.))
                    .line_height(rpx(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if url.is_some() {
                        theme::link()
                    } else {
                        theme::strong()
                    })),
                url,
            )
            .child(text),
        );
    }
    if let Some(description) = embed.description.as_ref().filter(|_| {
        !matches!(
            embed.kind,
            EmbedKind::Image | EmbedKind::Video | EmbedKind::Gifv
        )
    }) {
        body = body.child(if rich {
            markdown(
                app,
                key(DESCRIPTION_SLOT),
                &description_text(description),
                14.,
                18.,
                cx,
            )
        } else {
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .child(wrapped_label(
                    ("embed-description", key(DESCRIPTION_SLOT)),
                    description,
                ))
                .into_any_element()
        });
    }
    if !embed.fields.is_empty() {
        body = body.child(fields(
            app,
            &embed.fields,
            |n| key(FIELD_SLOT + n as u64),
            |n| key(FIELD_NAME_SLOT + n as u64),
            cx,
        ));
    }

    let media = embed_media(app, embed, &key, max, true, cx);
    let side_thumbnail = embed
        .thumbnail
        .as_ref()
        .filter(|_| media.is_none() || (rich && embed.video.is_none()));
    let content_width = embed_content_width(embed, max, media.as_ref().map(|(_, width)| *width));
    let content = content_row()
        .child(body)
        .children(side_thumbnail.map(|thumbnail| {
            let (width, height) = fit(thumbnail, (THUMBNAIL, THUMBNAIL));
            lightbox_box(
                app,
                key(SIDE_THUMBNAIL_SLOT),
                thumbnail,
                width,
                height,
                MEDIA_RADIUS,
                cx,
            )
        }));

    let footer = embed.footer.as_ref().map(|footer| footer.text.clone());
    let stamp = embed.timestamp.map(|timestamp| timestamp.calendar());
    let footer_text = match (footer, stamp) {
        (Some(text), Some(stamp)) => Some(format!("{text} \u{2022} {stamp}")),
        (text, stamp) => text.or(stamp),
    };

    embed_card(embed.color, content_width)
        .child(content)
        .children(media.map(|(media, _)| div().mt(rpx(8.)).child(media)))
        .children(footer_text.map(|text| {
            div()
                .flex()
                .items_center()
                .gap(rpx(8.))
                .max_w(rpx(text_limit()))
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
                        .text_color(rgb(theme::text()))
                        .child(wrapped_label(("embed-footer-text", key(59)), &text)),
                )
        }))
        .into_any_element()
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
