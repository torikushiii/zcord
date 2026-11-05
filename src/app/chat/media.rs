use super::*;
use crate::app::attachments::UploadButton;
use crate::app::expression_info::{Expression, HoverOwner};
use crate::store::AttachmentExt as _;

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
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .child(theme::icon("sticker", 48., theme::muted()))
        .child(
            div()
                .max_w(rpx(STICKER_SIZE - 16.))
                .truncate()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
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

/// Two or more images and videos share Discord's mosaic; other files follow as cards.
pub(super) fn render_attachments(
    app: &DiscordApp,
    attachments: &[Attachment],
    cx: &App,
) -> Vec<AnyElement> {
    let (visual, files): (Vec<_>, Vec<_>) = attachments.iter().partition(|attachment| {
        if attachment.is_video() {
            attachment.video_url().is_some()
        } else {
            attachment.image_spec().is_some()
        }
    });
    let [lead, _, ..] = visual.as_slice() else {
        return attachments
            .iter()
            .map(|attachment| render_attachment(app, attachment, cx))
            .collect();
    };
    let lead = (lead.width.unwrap_or(0), lead.height.unwrap_or(0));
    let width = MEDIA_MAX.0.min(media_limit());
    let grid = embeds::mosaic(visual.len(), width, lead, |slot, width, height| {
        attachment_box(app, visual[slot], width, height, 4., cx)
    });
    std::iter::once(div().mt_1().child(grid).into_any_element())
        .chain(
            files
                .into_iter()
                .map(|attachment| file_card(attachment).into_any_element()),
        )
        .collect()
}

fn render_attachment(app: &DiscordApp, attachment: &Attachment, cx: &App) -> AnyElement {
    let visual = if attachment.is_video() {
        attachment.video_url().is_some()
    } else {
        attachment.image_spec().is_some()
    };
    if !visual {
        return file_card(attachment).into_any_element();
    }
    let (width, height) = (
        attachment.width.unwrap_or(1).max(1) as f32,
        attachment.height.unwrap_or(1).max(1) as f32,
    );
    let scale = (MEDIA_MAX.0.min(media_limit()) / width)
        .min(MEDIA_MAX.1 / height)
        .min(1.);
    let (width, height) = ((width * scale).round(), (height * scale).round());
    div()
        .mt_1()
        .child(attachment_box(app, attachment, width, height, 8., cx))
        .into_any_element()
}

fn attachment_box(
    app: &DiscordApp,
    attachment: &Attachment,
    width: f32,
    height: f32,
    radius: f32,
    cx: &App,
) -> AnyElement {
    if attachment.is_video() {
        let Some(url) = attachment.video_url() else {
            return file_card(attachment).into_any_element();
        };
        let poster = attachment.video_poster_spec();
        if attachment.spoiler() && !app.revealed_spoilers.contains(&(attachment.id, 0)) {
            let media = poster.as_ref().map_or_else(
                || {
                    div()
                        .relative()
                        .flex_none()
                        .w(rpx(width))
                        .h(rpx(height))
                        .rounded(rpx(radius))
                        .overflow_hidden()
                        .bg(rgba(theme::border()))
                },
                |spec| embeds::blurred_box(spec, width, height, radius, cx),
            );
            return media
                .child(embeds::spoiler_overlay(app, attachment.id, radius))
                .into_any_element();
        }
        return crate::media::player::video_box(
            app,
            crate::media::player::Video {
                key: attachment.id,
                url: &url,
                poster,
                width,
                height,
                radius,
                fullscreen: false,
            },
            cx,
        )
        .into_any_element();
    }
    let Some(spec) = attachment.image_spec() else {
        return file_card(attachment).into_any_element();
    };
    let (original_width, original_height) = (
        attachment.width.unwrap_or(1).max(1),
        attachment.height.unwrap_or(1).max(1),
    );
    if attachment.spoiler() && !app.revealed_spoilers.contains(&(attachment.id, 0)) {
        return embeds::blurred_box(&spec, width, height, radius, cx)
            .child(embeds::spoiler_overlay(app, attachment.id, radius))
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
                .rounded(rpx(radius))
                .id(("animated", attachment.id))
                .into_any_element(),
        ),
        None if animate => animated::poster(&spec, preview, cx).map(|image| {
            image
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(rpx(radius))
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
                .rounded(rpx(radius))
                .into_any_element()
        })
    });
    div()
        .id(("lightbox-open", attachment.id))
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(radius))
        .overflow_hidden()
        .bg(rgb(theme::elevated()))
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
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .child(theme::icon("file", 32., theme::muted()))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_color(rgb(theme::link()))
                        .child(attachment.filename.clone()),
                )
                .child(
                    div()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::faint()))
                        .child(file_size(attachment.size)),
                ),
        )
}

pub(super) fn upload_card(
    app: &DiscordApp,
    nonce: u64,
    uploads: &[PendingUpload],
) -> Option<impl IntoElement + use<>> {
    let title = match uploads {
        [] => return None,
        [upload] => upload.name.clone(),
        _ => format!("Uploading {} Files", uploads.len()),
    };
    let size: u64 = uploads.iter().map(|upload| upload.size).sum();
    let fraction = app.upload_fraction(nonce);
    let cancel = app.entity.clone();
    let card = div()
        .flex()
        .items_center()
        .gap_3()
        .mt_1()
        .max_w(rpx(432.))
        .p_3()
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .child(theme::icon("file", 32., theme::muted()))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .child(div().truncate().child(title))
                        .child(
                            div()
                                .flex_none()
                                .text_color(rgb(theme::faint()))
                                .child(format!(" — {}", file_size(size))),
                        ),
                )
                .children(fraction.map(|fraction| {
                    div()
                        .h(rpx(4.))
                        .rounded(rpx(2.))
                        .overflow_hidden()
                        .bg(rgba(theme::border()))
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
                    theme::icon_button(("cancel-upload", nonce), "x", 16., theme::muted()),
                )
                .on_click(move |_, _, cx| {
                    cancel.update(cx, |app, _| app.cancel_upload(nonce)).ok();
                }),
            )
        });
    Some(card)
}

pub(in crate::app) fn file_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{:.2} KB", bytes as f64 / 1024.),
        _ => format!("{:.2} MB", bytes as f64 / 1_048_576.),
    }
}
