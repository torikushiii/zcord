use gpui::{prelude::*, *};
use gpui_component::tooltip::Tooltip;

use crate::{
    app::DiscordApp,
    data::{Channel, ChannelKind},
    theme::{self, rpx},
};

pub const WIDTH: f32 = 303.;

pub fn render(app: &DiscordApp, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    let guild = app.guild();
    div()
        .flex()
        .flex_col()
        .w(rpx(WIDTH))
        .h_full()
        .flex_none()
        .border_r_1()
        .border_color(rgb(theme::BORDER))
        .child(
            div()
                .id("guild-header")
                .flex()
                .items_center()
                .gap_1()
                .h(rpx(48.))
                .flex_none()
                .pl_4()
                .pr_2()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(theme::HOVER)))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::STRONG))
                        .child(guild.map_or("No server", |guild| &guild.name).to_owned()),
                )
                .child(theme::icon("chevron-down", 16., theme::STRONG))
                .child(div().flex_1())
                .child(theme::icon_button("invite", "user-plus", 18., theme::MUTED)),
        )
        .child(
            div()
                .id("channels")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_2()
                .py_2()
                .child(shortcut_row("events", "calendar", "Events"))
                .child(shortcut_row("boosts", "gem", "Server Boosts"))
                .child(div().mx_2().my_2().h(rpx(1.)).bg(rgb(theme::BORDER)))
                .children(
                    guild
                        .into_iter()
                        .flat_map(|guild| &guild.categories)
                        .map(|category| {
                            div()
                                .pt_4()
                                .child(
                                    div()
                                        .id(("category", category.id))
                                        .flex()
                                        .items_center()
                                        .gap(rpx(2.))
                                        .h(rpx(24.))
                                        .px_2()
                                        .text_size(rpx(14.))
                                        .line_height(rpx(18.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(theme::MUTED))
                                        .cursor_pointer()
                                        .hover(|style| style.text_color(rgb(theme::TEXT)))
                                        .child(category.name.clone())
                                        .child(theme::icon("chevron-down", 12., theme::MUTED))
                                        .child(div().flex_1())
                                        .child(theme::icon("plus", 16., theme::MUTED)),
                                )
                                .children(category.channels.iter().map(|channel| {
                                    channel_row(channel, app.selected_channel_id == channel.id, cx)
                                }))
                        }),
                ),
        )
        .child(user_panel(app, cx))
}

fn shortcut_row(id: &'static str, icon: &str, label: &'static str) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(32.))
        .px_2()
        .rounded(rpx(6.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::MUTED))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(theme::HOVER)).text_color(rgb(theme::TEXT)))
        .child(theme::icon(icon, 20., theme::MUTED))
        .child(label)
}

fn channel_row(
    channel: &Channel,
    selected: bool,
    cx: &mut Context<DiscordApp>,
) -> impl IntoElement + use<> {
    let id = channel.id;
    let text = channel.kind == ChannelKind::Text;
    let unread = channel.unread && !selected;
    div()
        .id(("channel", id))
        .relative()
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(32.))
        .mt(rpx(2.))
        .px_2()
        .rounded(rpx(6.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .text_color(rgb(if selected || unread {
            theme::STRONG
        } else {
            theme::MUTED
        }))
        .when(unread, |this| this.font_weight(FontWeight::SEMIBOLD))
        .when(selected, |this| this.bg(rgb(theme::SELECTED)))
        .when(!selected, |this| {
            this.hover(|style| style.bg(rgb(theme::HOVER)).text_color(rgb(theme::TEXT)))
        })
        .when(text, |this| {
            this.on_click(
                cx.listener(move |this, _, window, cx| this.select_channel(id, window, cx)),
            )
        })
        .when(unread, |this| {
            this.child(
                div()
                    .absolute()
                    .left(rpx(-8.))
                    .top(rpx(12.))
                    .w(rpx(4.))
                    .h(rpx(8.))
                    .rounded_r(rpx(4.))
                    .bg(rgb(theme::STRONG)),
            )
        })
        .child(theme::icon(
            if text { "hash" } else { "volume-2" },
            20.,
            if selected || unread {
                theme::STRONG
            } else {
                theme::FAINT
            },
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(channel.name.clone()),
        )
        .when(selected, |this| {
            this.child(theme::icon("user-plus", 14., theme::MUTED))
                .child(theme::icon("settings", 14., theme::MUTED))
        })
}

fn user_panel(app: &DiscordApp, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    let user = app.store.user(app.store.current_user_id);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .h(rpx(58.))
        .m_2()
        .px(rpx(6.))
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .children(user.map(|user| {
            div()
                .id("account")
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .p(rpx(2.))
                .rounded(rpx(6.))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(theme::SELECTED)))
                .child(theme::avatar(
                    &user.initials,
                    theme::user_color(user.id),
                    36.,
                    Some((user.status, theme::ELEVATED)),
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::STRONG))
                                .child(user.name.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(12.))
                                .text_color(rgb(theme::MUTED))
                                .child(user.activity.clone().unwrap_or_else(|| "Online".into())),
                        ),
                )
        }))
        .child(theme::icon_button("mic", "mic", 20., theme::TEXT))
        .child(theme::icon_button("deafen", "headphones", 20., theme::TEXT))
        .child(
            theme::icon_button("user-settings", "settings", 20., theme::TEXT)
                .tooltip(|window, cx| Tooltip::new("Log out").build(window, cx))
                .on_click(cx.listener(|this, _, window, cx| this.log_out(window, cx))),
        )
}
