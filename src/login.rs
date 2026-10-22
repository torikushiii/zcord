use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};

use crate::{
    app::DiscordApp,
    theme::{self, rpx},
};

pub struct LoginForm {
    pub token: Entity<InputState>,
    pub error: Option<SharedString>,
    pub busy: bool,
    _submit_on_enter: Subscription,
}

impl LoginForm {
    pub fn new(window: &mut Window, cx: &mut Context<DiscordApp>) -> Self {
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Paste your account token")
        });
        let submit = cx.subscribe_in(&token, window, |app, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                app.submit_login(window, cx);
            }
        });
        token.update(cx, |input, cx| input.focus(window, cx));
        Self {
            token,
            error: None,
            busy: false,
            _submit_on_enter: submit,
        }
    }
}

pub fn render(form: &LoginForm, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .bg(rgb(theme::BG))
        .child(
            div()
                .flex()
                .flex_col()
                .w(rpx(480.))
                .p(rpx(32.))
                .rounded(rpx(8.))
                .bg(rgb(theme::ELEVATED))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .child(
                    div()
                        .flex()
                        .justify_center()
                        .text_size(rpx(24.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::STRONG))
                        .child("Welcome back!"),
                )
                .child(
                    div()
                        .flex()
                        .justify_center()
                        .mt_2()
                        .text_size(rpx(16.))
                        .text_color(rgb(theme::MUTED))
                        .child("We're so excited to see you again!"),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .mt(rpx(20.))
                        .mb_2()
                        .text_size(rpx(12.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(if form.error.is_some() {
                            theme::RED
                        } else {
                            theme::MUTED
                        }))
                        .child("TOKEN")
                        .child(match &form.error {
                            Some(error) => div()
                                .font_weight(FontWeight::NORMAL)
                                .italic()
                                .child(format!("- {error}")),
                            None => div().text_color(rgb(theme::RED)).child("*"),
                        }),
                )
                .child(
                    div()
                        .px_3()
                        .rounded(rpx(8.))
                        .bg(rgb(theme::BG))
                        .border_1()
                        .border_color(rgb(theme::BORDER))
                        .child(
                            Input::new(&form.token)
                                .appearance(false)
                                .mask_toggle()
                                .h(rpx(42.))
                                .px_0()
                                .text_size(rpx(16.))
                                .line_height(rpx(22.)),
                        ),
                )
                .child(
                    div()
                        .id("login")
                        .flex()
                        .items_center()
                        .justify_center()
                        .mt(rpx(20.))
                        .h(rpx(44.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::BLURPLE))
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(white())
                        .map(|button| {
                            if form.busy {
                                button.opacity(0.6).child("Connecting…")
                            } else {
                                button
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(0x4752C4)))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.submit_login(window, cx)
                                    }))
                                    .child("Log In")
                            }
                        }),
                )
                .child(
                    div()
                        .mt_2()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::FAINT))
                        .child(
                            "Stored in your system's credential manager and sent only to Discord.",
                        ),
                )
                .child(
                    div()
                        .id("demo")
                        .mt_3()
                        .text_size(rpx(14.))
                        .text_color(rgb(0x00A8FC))
                        .cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(cx.listener(|this, _, window, cx| this.open_demo(window, cx)))
                        .child("Browse demo data instead"),
                ),
        )
}
