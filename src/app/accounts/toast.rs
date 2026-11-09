use super::*;

pub(crate) fn toast(app: &DiscordApp) -> Option<AnyElement> {
    let (text, success) = app.accounts.toast.clone()?;
    Some(
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(rpx(32.))
            .flex()
            .justify_center()
            .px(rpx(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(8.))
                    .max_w_full()
                    .px(rpx(12.))
                    .py(rpx(10.))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .shadow_lg()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(if success {
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(rpx(18.))
                            .rounded_full()
                            .bg(rgb(theme::text_positive()))
                            .child(theme::icon("check-bold", 10., 0xFFFFFF))
                    } else {
                        div()
                            .flex_none()
                            .child(theme::icon("circle-alert-filled", 18., theme::RED))
                    })
                    .child(div().min_w_0().truncate().child(text))
                    .with_animation(
                        ("account-toast", app.accounts.toast_serial),
                        Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint()),
                        |this, delta| this.opacity(delta),
                    ),
            )
            .into_any_element(),
    )
}
