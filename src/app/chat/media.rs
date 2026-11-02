use super::*;
use crate::app::attachments::UploadButton;
use crate::app::expression_info::{Expression, HoverOwner};

const MEDIA_MAX: (f32, f32) = (550., 350.);

thread_local! {
    static MEDIA_LIMIT: std::cell::Cell<f32> = const { std::cell::Cell::new(f32::INFINITY) };
}

/// The widest message media may be, for columns narrower than the chat.
pub(in crate::app) fn media_limit() -> f32 {
    MEDIA_LIMIT.get()
}

pub(in crate::app) fn with_media_limit<T>(width: f32, build: impl FnOnce() -> T) -> T {
    let outer = MEDIA_LIMIT.replace(width);
    let built = build();
    MEDIA_LIMIT.set(outer);
    built
}

fn sticker_placeholder(sticker: &Sticker) -> AnyElement {
    let name = if sticker.name.is_empty() {
        "Sticker".to_owned()
    } else {
        sticker.name.clone()
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rpx(8.))
        .size_full()
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgba(theme::BORDER))
        .child(theme::icon("sticker", 48., theme::MUTED))
        .child(
            div()
                .max_w(rpx(STICKER_SIZE - 16.))
                .truncate()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::MUTED))
                .child(name),
        )
        .into_any_element()
}

pub(super) fn render_sticker(
    app: &DiscordApp,
    message_id: u64,
    sticker: &Sticker,
    cx: &App,
) -> AnyElement {
    let pixels = theme::image_px(STICKER_SIZE);
    let image = sticker.image_spec().and_then(|spec| {
        let still = || {
            images::resolve(&spec, pixels, cx).map(|path| {
                img(path)
                    .id((
                        ElementId::from(("sticker-still", message_id)),
                        sticker.id.to_string(),
                    ))
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .into_any_element()
            })
        };
        match sticker.format {
            2 | 4 => animated::resolve(&spec, pixels, cx)
                .map(|image| {
                    animated::image(image)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .id((
                            ElementId::from(("sticker", message_id)),
                            sticker.id.to_string(),
                        ))
                        .into_any_element()
                })
                .or_else(still),
            1 => still(),
            _ => None,
        }
    });
    let entity = app.entity.clone();
    let sticker_id = sticker.id;
    let expression = Expression::Sticker(sticker.clone());
    let click_expression = expression.clone();
    let click_entity = entity.clone();
    let bounds = Rc::new(Cell::new(None));
    let painted_bounds = bounds.clone();
    div()
        .id((
            ElementId::from(("sticker-hover", message_id)),
            sticker.id.to_string(),
        ))
        .relative()
        .flex_none()
        .w(rpx(STICKER_SIZE))
        .h(rpx(STICKER_SIZE))
        .mt_1()
        .cursor_pointer()
        .on_prepaint(move |bounds, _, _| painted_bounds.set(Some(bounds)))
        .on_hover(move |hovered, _, cx| {
            let Some(bounds) = bounds.get() else {
                return;
            };
            entity
                .update(cx, |app, cx| {
                    app.set_expression_hover(
                        HoverOwner::Sticker(message_id, sticker_id),
                        expression.clone(),
                        bounds,
                        *hovered,
                        cx,
                    );
                })
                .ok();
        })
        .on_click(move |_, window, cx| {
            click_entity
                .update(cx, |app, cx| {
                    app.open_expression_info(click_expression.clone(), window.mouse_position(), cx);
                })
                .ok();
            cx.stop_propagation();
        })
        .child(image.unwrap_or_else(|| sticker_placeholder(sticker)))
        .into_any_element()
}

pub(super) fn render_attachment(app: &DiscordApp, attachment: &Attachment, cx: &App) -> AnyElement {
    if attachment.is_video() {
        let Some(url) = attachment.video_url() else {
            return file_card(attachment).into_any_element();
        };
        let (width, height) = (
            attachment.width.unwrap_or(1).max(1) as f32,
            attachment.height.unwrap_or(1).max(1) as f32,
        );
        let scale = (MEDIA_MAX.0.min(media_limit()) / width)
            .min(MEDIA_MAX.1 / height)
            .min(1.);
        let (width, height) = ((width * scale).round(), (height * scale).round());
        let poster = attachment.video_poster_spec();
        if attachment.spoiler() && !app.revealed_spoilers.contains(&(attachment.id, 0)) {
            let media = poster.as_ref().map_or_else(
                || {
                    div()
                        .relative()
                        .flex_none()
                        .w(rpx(width))
                        .h(rpx(height))
                        .rounded(rpx(8.))
                        .overflow_hidden()
                        .bg(rgba(theme::BORDER))
                },
                |spec| embeds::blurred_box(spec, width, height, 8., cx),
            );
            return media
                .mt_1()
                .child(embeds::spoiler_overlay(app, attachment.id, 8.))
                .into_any_element();
        }
        return div()
            .mt_1()
            .child(crate::media::player::video_box(
                app,
                crate::media::player::Video {
                    key: attachment.id,
                    url: &url,
                    poster,
                    width,
                    height,
                    radius: 8.,
                    fullscreen: false,
                },
                cx,
            ))
            .into_any_element();
    }
    let Some(spec) = attachment.image_spec() else {
        return file_card(attachment).into_any_element();
    };
    let (original_width, original_height) = (
        attachment.width.unwrap_or(1).max(1),
        attachment.height.unwrap_or(1).max(1),
    );
    let (width, height) = (original_width as f32, original_height as f32);
    let scale = (MEDIA_MAX.0.min(media_limit()) / width)
        .min(MEDIA_MAX.1 / height)
        .min(1.);
    let (width, height) = ((width * scale).round(), (height * scale).round());
    if attachment.spoiler() && !app.revealed_spoilers.contains(&(attachment.id, 0)) {
        return embeds::blurred_box(&spec, width, height, 8., cx)
            .mt_1()
            .child(embeds::spoiler_overlay(app, attachment.id, 8.))
            .into_any_element();
    }
    let preview = theme::image_px(width.max(height));
    let animate = animated_attachment(attachment);
    let frames = animate
        .then(|| animated::resolve(&spec, preview, cx))
        .flatten();
    let entity = app.entity.clone();
    let url = attachment.url.clone();
    let loading = !images::failed(&spec, preview, cx);
    let child = match frames {
        Some(image) => Some(
            animated::image(image)
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(rpx(8.))
                .id(("animated", attachment.id))
                .into_any_element(),
        ),
        None if animate => animated::poster(&spec, preview, cx).map(|image| {
            image
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(rpx(8.))
                .id(("attachment-still", attachment.id))
                .into_any_element()
        }),
        None => None,
    }
    .or_else(|| {
        let fallback = if animate {
            animated::fallback(&spec, preview, cx)?
        } else {
            spec.clone()
        };
        images::resolve(&fallback, preview, cx).map(|path| {
            embeds::animated(path, &attachment.url)
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(rpx(8.))
                .into_any_element()
        })
    });
    div()
        .id(("lightbox-open", attachment.id))
        .relative()
        .mt_1()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(rgb(theme::ELEVATED))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| {
                    app.open_lightbox(
                        spec.clone(),
                        animate,
                        (original_width, original_height),
                        preview,
                        url.clone(),
                        cx,
                    )
                })
                .ok();
        })
        .when(child.is_none() && loading, |this| {
            this.child(theme::media_loading())
        })
        .children(child)
        .into_any_element()
}

fn animated_attachment(attachment: &Attachment) -> bool {
    let gif_type = attachment
        .content_type
        .as_deref()
        .and_then(|mime| mime.split(';').next())
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("image/gif"));
    let gif_extension = attachment
        .filename
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("gif"));
    gif_type || gif_extension
}

fn file_card(attachment: &Attachment) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .gap_3()
        .mt_1()
        .max_w(rpx(432.))
        .p_3()
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgba(theme::BORDER))
        .child(theme::icon("file", 32., theme::MUTED))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_color(rgb(LINK))
                        .child(attachment.filename.clone()),
                )
                .child(
                    div()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::FAINT))
                        .child(file_size(attachment.size)),
                ),
        )
}

pub(super) fn upload_card(
    app: &DiscordApp,
    nonce: u64,
    upload: &PendingUpload,
) -> impl IntoElement + use<> {
    let fraction = app.upload_fraction(nonce);
    let cancel = app.entity.clone();
    div()
        .flex()
        .items_center()
        .gap_3()
        .mt_1()
        .max_w(rpx(432.))
        .p_3()
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgba(theme::BORDER))
        .child(theme::icon("file", 32., theme::MUTED))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().truncate().child(upload.name.clone()))
                .child(
                    div()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::FAINT))
                        .child(file_size(upload.size)),
                )
                .children(fraction.map(|fraction| {
                    div()
                        .h(rpx(4.))
                        .rounded(rpx(2.))
                        .overflow_hidden()
                        .bg(rgba(theme::BORDER))
                        .child(
                            div()
                                .h_full()
                                .w(relative(fraction.clamp(0., 1.)))
                                .bg(rgb(theme::BLURPLE)),
                        )
                })),
        )
        .when(fraction.is_some(), |this| {
            this.child(
                app.upload_tip(
                    UploadButton::Cancel(nonce),
                    "Cancel Upload",
                    theme::icon_button(("cancel-upload", nonce), "x", 16., theme::MUTED),
                )
                .on_click(move |_, _, cx| {
                    cancel.update(cx, |app, _| app.cancel_upload(nonce)).ok();
                }),
            )
        })
}

pub(in crate::app) fn file_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{:.2} KB", bytes as f64 / 1024.),
        _ => format!("{:.2} MB", bytes as f64 / 1_048_576.),
    }
}
