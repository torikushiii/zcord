use gpui::{prelude::*, *};
use gpui_component::tooltip::Tooltip;

use crate::{
    app::DiscordApp,
    theme::{self, rpx},
};

const WIDTH: f32 = 72.;
const ICON: f32 = 48.;

pub fn render(app: &DiscordApp, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    div()
        .id("guild-rail")
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .w(rpx(WIDTH))
        .h_full()
        .flex_none()
        .pb_3()
        .overflow_y_scroll()
        .child(
            rail_item("home", false, false)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(rpx(ICON))
                        .rounded(rpx(16.))
                        .bg(rgb(theme::ELEVATED))
                        .hover(|style| style.bg(rgb(theme::BLURPLE)))
                        .child(theme::icon("discord", 28., theme::STRONG)),
                )
                .children(corner_badge(app.store.dm_mentions))
                .tooltip(|window, cx| Tooltip::new("Direct Messages").build(window, cx)),
        )
        .child(
            div()
                .w(rpx(32.))
                .h(rpx(1.))
                .flex_none()
                .bg(rgb(theme::BORDER)),
        )
        .children(app.store.guilds.iter().map(|guild| {
            let id = guild.id;
            let selected = app.selected_guild_id == id;
            let name = guild.name.clone();
            rail_item(("guild", id), selected, guild.unread)
                .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
                .on_click(cx.listener(move |this, _, window, cx| this.select_guild(id, window, cx)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(rpx(ICON))
                        .rounded(rpx(if selected { 16. } else { ICON / 2. }))
                        .hover(|style| style.rounded(rpx(16.)))
                        .bg(rgb(guild.icon_color))
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(white())
                        .child(guild.initials.clone()),
                )
                .children(corner_badge(guild.mentions))
        }))
        .child(action_item("add-server", "plus", "Add a Server"))
        .child(action_item("discover", "compass", "Discover"))
}

fn rail_item(id: impl Into<ElementId>, selected: bool, unread: bool) -> Stateful<Div> {
    div()
        .id(id)
        .group("rail-item")
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w_full()
        .h(rpx(ICON))
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .left_0()
                .w(rpx(4.))
                .rounded_r(rpx(4.))
                .bg(rgb(theme::STRONG))
                .map(|pill| {
                    if selected {
                        pill.h(rpx(40.)).top(rpx(4.))
                    } else if unread {
                        pill.h(rpx(8.)).top(rpx(20.))
                    } else {
                        pill.h(rpx(0.)).top(rpx(24.))
                    }
                })
                .when(!selected, |pill| {
                    pill.group_hover("rail-item", |style| style.h(rpx(20.)).top(rpx(14.)))
                }),
        )
}

fn corner_badge(count: u32) -> Option<impl IntoElement> {
    (count > 0).then(|| {
        div()
            .absolute()
            .bottom(rpx(-4.))
            .right(rpx(8.))
            .child(theme::badge(count, theme::BG))
    })
}

fn action_item(id: &'static str, icon: &'static str, label: &'static str) -> impl IntoElement {
    rail_item(id, false, false)
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(ICON))
                .rounded_full()
                .bg(rgb(theme::ELEVATED))
                .child(theme::icon(icon, 24., theme::TEXT)),
        )
}
