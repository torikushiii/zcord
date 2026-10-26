use super::message::author_name;
use super::*;

pub(crate) fn composer(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let can_send = app.store.can_send(app.selected_channel_id);
    // Discord counts the sent form, so tokens count as their `<:name:id>` markup.
    let text = app.composer.read(cx).text();
    let length = if text.chars().any(composer_tokens::is_token_char) {
        composer_tokens::to_wire(&text.to_string()).chars().count()
    } else {
        text.len_chars()
    };
    let remaining = MESSAGE_LIMIT as i64 - length as i64;
    let picker_anchor = Rc::new(Cell::new(None));
    div()
        .relative()
        .px_2()
        .pb_2()
        .flex_none()
        .on_prepaint({
            let anchor = picker_anchor.clone();
            move |bounds, window, _| {
                let gap = rpx(8.).to_pixels(window.rem_size());
                let right_inset = rpx(24.).to_pixels(window.rem_size());
                anchor.set(Some(point(
                    bounds.right() - right_inset,
                    bounds.top() - gap,
                )));
            }
        })
        .children(app.reply.map(|reply| reply_banner(app, reply)))
        .child(
            div()
                .relative()
                .flex()
                .items_start()
                .gap_1()
                .min_h(rpx(56.))
                .pl(rpx(12.))
                .pr(rpx(8.))
                .map(|this| {
                    if app.reply.is_some() {
                        this.rounded_b(rpx(8.))
                    } else {
                        this.rounded(rpx(8.))
                    }
                })
                .bg(rgb(theme::ELEVATED))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .children(emoji_picker::suggestions(app, cx))
                .child(div().my(rpx(12.)).child(theme::icon_button(
                    "attach",
                    "circle-plus",
                    24.,
                    theme::MUTED,
                )))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .ml_4()
                        .child(
                            flush_textarea(Textarea::new(&app.composer))
                                .disabled(!can_send)
                                .py(rpx(17.))
                                .text_size(rpx(16.))
                                .line_height(rpx(22.))
                                .text_color(rgb(theme::TEXT)),
                        )
                        .child(emoji_token_overlay(&app.composer)),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .my(rpx(12.))
                        .child({
                            let entity = app.entity.clone();
                            let picker_anchor = picker_anchor.clone();
                            theme::icon_button("gif", "image-play", 24., theme::MUTED)
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    let Some(position) = picker_anchor.get() else {
                                        return;
                                    };
                                    entity
                                        .update(cx, |app, cx| {
                                            app.toggle_gif_picker(position, window, cx)
                                        })
                                        .ok();
                                })
                        })
                        // .child(theme::icon_button("stickers", "sticker", 24., theme::MUTED))
                        .child({
                            let entity = app.entity.clone();
                            theme::icon_button("emoji", "smile", 24., theme::MUTED).on_mouse_down(
                                MouseButton::Left,
                                move |_, window, cx| {
                                    let Some(position) = picker_anchor.get() else {
                                        return;
                                    };
                                    entity
                                        .update(cx, |app, cx| {
                                            app.toggle_emoji_picker(
                                                PickerTarget::Composer,
                                                position,
                                                window,
                                                cx,
                                            )
                                        })
                                        .ok();
                                },
                            )
                        }),
                )
                .when(remaining < 200, |this| {
                    this.child(
                        div()
                            .absolute()
                            .right(rpx(12.))
                            .bottom(rpx(4.))
                            .text_size(rpx(12.))
                            .text_color(rgb(if remaining < 0 {
                                theme::RED
                            } else {
                                theme::FAINT
                            }))
                            .child(remaining.to_string()),
                    )
                }),
        )
        .children(typing_indicator(app))
}

/// Paints custom emoji over their invisible composer tokens (see `composer_tokens`).
///
/// Token positions come from the textarea's layout, which it records while painting, so this
/// canvas must come after the textarea to read the current frame's layout instead of the last.
pub(super) fn emoji_token_overlay(input: &Entity<TextareaState>) -> impl IntoElement + use<> {
    let input = input.clone();
    canvas(
        |_, _, _| {},
        move |_, _, window, cx| {
            let (tokens, clip, selected) = {
                let state = input.read(cx);
                // Paint runs every frame; skip the text copy unless a token can be present.
                if !state.text().chars().any(composer_tokens::is_token_char) {
                    return;
                }
                let tokens: Vec<_> = composer_tokens::tokens_in(&state.value())
                    .into_iter()
                    .filter_map(|(range, token)| {
                        let bounds = state.range_to_bounds(&range)?;
                        Some((range, bounds, token))
                    })
                    .collect();
                (tokens, state.input_bounds(), state.selected_range())
            };
            if tokens.is_empty() {
                return;
            }
            let size = rpx(18.).to_pixels(window.rem_size());
            window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                for (range, bounds, token) in tokens {
                    // The token's variation selectors have no glyph in the UI font and draw as
                    // small boxes, so cover the whole token with the input background first.
                    window.paint_quad(fill(bounds, rgb(theme::ELEVATED)));
                    if range.start >= selected.start && range.end <= selected.end {
                        window.paint_quad(fill(bounds, rgba(0x5865F24D)));
                    }
                    let spec = ImageSpec::Emoji {
                        id: token.id,
                        animated: false,
                    };
                    let Some(image) = animated::resolve(&spec, theme::image_px(22.), cx) else {
                        continue;
                    };
                    // Centred on the whole token: the EM SPACE plus whatever width the
                    // selector boxes take.
                    let frame = image.size(0);
                    let scale = size / px(frame.width.0.max(frame.height.0) as f32);
                    let fitted = gpui::size(
                        px(frame.width.0 as f32) * scale,
                        px(frame.height.0 as f32) * scale,
                    );
                    let origin = bounds.center() - point(fitted.width / 2., fitted.height / 2.);
                    let target = Bounds::new(origin, fitted);
                    window
                        .paint_image(target, Corners::default(), image, 0, false)
                        .ok();
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

const TEXTAREA_BUILTIN_INSET_X: f32 = 10.;
const TEXTAREA_BUILTIN_INSET_Y: f32 = 8.;

pub(super) fn flush_textarea(textarea: Textarea) -> Textarea {
    textarea
        .appearance(false)
        .bordered(false)
        .w_full()
        .px_0()
        .ml(px(-TEXTAREA_BUILTIN_INSET_X))
        .my(px(-TEXTAREA_BUILTIN_INSET_Y))
}

fn reply_banner(app: &DiscordApp, reply: ReplyTarget) -> impl IntoElement {
    let entity = app.entity.clone();
    let own = reply.author_id == app.store.current_user_id;
    div()
        .flex()
        .items_center()
        .h(rpx(34.))
        .pl(rpx(16.))
        .pr(rpx(8.))
        .rounded_t(rpx(8.))
        .bg(rgb(theme::HOVER))
        .border_1()
        .border_b_0()
        .border_color(rgb(theme::BORDER))
        .text_size(rpx(14.))
        .text_color(rgb(theme::MUTED))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .gap(rpx(4.))
                .child("Replying to")
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::TEXT))
                        .child(label(
                            ("reply-target", reply.message_id),
                            &author_name(app, reply.author_id),
                        )),
                ),
        )
        .when(!own, |this| {
            let toggle = entity.clone();
            this.child(
                div()
                    .id("reply-mention")
                    .flex()
                    .items_center()
                    .gap(rpx(2.))
                    .px_2()
                    .cursor_pointer()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(if reply.mention { LINK } else { theme::MUTED }))
                    .tooltip(|window, cx| Tooltip::new("Mention the author").build(window, cx))
                    .on_click(move |_, _, cx| {
                        toggle
                            .update(cx, |app, cx| app.toggle_reply_mention(cx))
                            .ok();
                    })
                    .child("@")
                    .child(if reply.mention { "ON" } else { "OFF" }),
            )
            .child(div().w(rpx(1.)).h(rpx(16.)).mx_1().bg(rgb(theme::BORDER)))
        })
        .child(
            theme::icon_button("reply-cancel", "x", 16., theme::MUTED).on_click(move |_, _, cx| {
                entity.update(cx, |app, cx| app.cancel_reply(cx)).ok();
            }),
        )
}

fn typing_indicator(app: &DiscordApp) -> Option<impl IntoElement> {
    let names: Vec<String> = app
        .typing_users()
        .iter()
        .map(|(user_id, _)| author_name(app, *user_id))
        .collect();
    let bold = |name: &String| {
        div()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(theme::TEXT))
            .child(name.clone())
    };
    let text = div().flex().min_w_0().overflow_hidden().whitespace_nowrap();
    let text = match names.as_slice() {
        [] => text,
        [one] => text.child(bold(one)).child("\u{a0}is typing…"),
        [one, two] => text
            .child(bold(one))
            .child("\u{a0}and\u{a0}")
            .child(bold(two))
            .child("\u{a0}are typing…"),
        [one, two, three] => text
            .child(bold(one))
            .child(",\u{a0}")
            .child(bold(two))
            .child(", and\u{a0}")
            .child(bold(three))
            .child("\u{a0}are typing…"),
        _ => text.child("Several people are typing…"),
    };
    if names.is_empty() {
        return None;
    }
    Some(
        div()
            .absolute()
            .bottom(relative(1.))
            .left(rpx(8.))
            .right(rpx(8.))
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(24.))
            .pl(rpx(8.))
            .bg(rgb(theme::BG))
            .text_size(rpx(12.))
            .text_color(rgb(theme::MUTED))
            .child(
                div()
                    .flex()
                    .gap(rpx(2.))
                    .children(
                        (0..3).map(|_| div().size(rpx(4.)).rounded_full().bg(rgb(theme::TEXT))),
                    )
                    .with_animation(
                        "typing-dots",
                        Animation::new(Duration::from_millis(1200))
                            .repeat()
                            .with_easing(pulsating_between(0.3, 1.)),
                        |dots, delta| dots.opacity(delta),
                    ),
            )
            .child(text),
    )
}
