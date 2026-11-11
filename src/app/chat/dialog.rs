use super::message::render_message;
use super::*;

fn protip() -> StyledText {
    const TIP: &str = "You can hold down shift when clicking delete message to bypass this confirmation entirely.";
    const BOLD: &str = "delete message";
    let start = TIP.find(BOLD).unwrap_or_default();
    StyledText::new(TIP).with_highlights([(
        start..start + BOLD.len(),
        HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        },
    )])
}

fn dialog_button(id: &'static str, text: &'static str, bg: u32, hover: u32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .h(rpx(44.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(rgb(bg))
        .hover(move |style| style.bg(rgb(hover)))
        .text_size(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()))
        .child(text)
}

pub(in crate::app) fn delete_dialog(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let message_id = app.compose.confirm_delete?;
    let message = app
        .channel()?
        .messages
        .iter()
        .find(|message| message.id == message_id)?;
    let closing = app.delete_dialog_closing();
    let (key, duration) = if closing {
        ("delete-dialog-out", DIALOG_EXIT)
    } else {
        ("delete-dialog-in", DIALOG_ENTER)
    };
    let animation = Animation::new(duration).with_easing(ease_out_quint());
    let progress = move |delta: f32| if closing { 1. - delta } else { delta };
    let backdrop = app.entity.clone();
    let close = app.entity.clone();
    let cancel = app.entity.clone();
    let confirm = app.entity.clone();
    let confirm_key = app.entity.clone();
    Some(
        div()
            .id("delete-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .on_click(move |_, _, cx| {
                backdrop.update(cx, |app, cx| app.cancel_delete(cx)).ok();
            })
            .with_animation((key, message_id), animation.clone(), move |this, delta| {
                this.bg(rgba(theme::SCRIM).opacity(progress(delta)))
            })
            .child(theme::selection_guard())
            .child(
                div()
                    .id("delete-dialog")
                    .relative()
                    .flex()
                    .flex_col()
                    .w(rpx(440.))
                    .max_w_full()
                    .p(rpx(24.))
                    .rounded(rpx(12.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .occlude()
                    .track_focus(&app.delete_focus)
                    .on_key_down(move |event, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter") && !closing {
                            confirm_key
                                .update(cx, |app, cx| {
                                    app.request_delete(message_id, true, window, cx)
                                })
                                .ok();
                            cx.stop_propagation();
                        }
                    })
                    .child(
                        theme::icon_button("delete-close", "x", 24., theme::muted())
                            .absolute()
                            .top(rpx(16.))
                            .right(rpx(16.))
                            .on_click(move |_, _, cx| {
                                close.update(cx, |app, cx| app.cancel_delete(cx)).ok();
                            }),
                    )
                    .child(
                        div()
                            .text_size(rpx(20.))
                            .line_height(rpx(24.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::strong()))
                            .child("Delete Message"),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_size(rpx(16.))
                            .text_color(rgb(theme::muted()))
                            .child("Are you sure you want to delete this message?"),
                    )
                    .child(
                        div()
                            .mt_4()
                            .max_h(rpx(300.))
                            .overflow_hidden()
                            .py_3()
                            .rounded(rpx(8.))
                            .bg(rgb(theme::bg()))
                            .border_1()
                            .border_color(rgba(theme::border()))
                            .child(render_message(app, message, false, true, cx)),
                    )
                    .child(
                        div()
                            .mt_4()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .text_color(rgb(theme::text()))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::GREEN))
                                    .child("Protip:"),
                            )
                            .child(protip()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .mt_6()
                            .child(
                                dialog_button(
                                    "delete-cancel",
                                    "Cancel",
                                    theme::button_subtle(),
                                    theme::button_subtle_hover(),
                                )
                                .border_1()
                                .border_color(rgba(theme::border()))
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |app, cx| app.cancel_delete(cx)).ok();
                                }),
                            )
                            .child(
                                dialog_button("delete-confirm", "Delete", 0xD22D39, 0xB82A33)
                                    .on_click(move |_, window, cx| {
                                        confirm
                                            .update(cx, |app, cx| {
                                                app.request_delete(message_id, true, window, cx)
                                            })
                                            .ok();
                                    }),
                            ),
                    )
                    .with_animation((key, message_id), animation, move |this, delta| {
                        let shown = progress(delta);
                        this.opacity(shown).top(rpx(16. * (1. - shown)))
                    }),
            )
            .into_any_element(),
    )
}
