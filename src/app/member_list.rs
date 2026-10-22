use gpui::{prelude::*, *};

use crate::{
    app::{DiscordApp, message_body::label},
    store::{MemberRow, User},
    theme::{self, rpx},
};

pub const WIDTH: f32 = 264.;

pub fn render(app: &DiscordApp, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    let entity = cx.entity();
    div()
        .id("members")
        .w(rpx(WIDTH))
        .h_full()
        .flex_none()
        .border_l_1()
        .border_color(rgb(theme::BORDER))
        .child(
            list(app.member_rows.clone(), move |index, _, cx| {
                render_row(entity.read(cx), index, cx)
            })
            .size_full(),
        )
}

fn render_row(app: &DiscordApp, index: usize, cx: &App) -> AnyElement {
    let last = app
        .store
        .member_list(app.selected_guild_id)
        .is_some_and(|list| index + 1 == list.rows.len());
    div()
        .w_full()
        .px_2()
        .when(last, |this| this.pb_4())
        .child(row_content(app, index, cx))
        .into_any_element()
}

fn row_content(app: &DiscordApp, index: usize, cx: &App) -> AnyElement {
    let guild_id = app.selected_guild_id;
    let row = app
        .store
        .member_list(guild_id)
        .and_then(|list| list.rows.get(index))
        .and_then(Option::as_ref);
    match row {
        Some(MemberRow::Group { name, count }) => div()
            .pt(rpx(if index == 0 { 16. } else { 24. }))
            .pb(rpx(4.))
            .px_2()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::MUTED))
            .truncate()
            .child(label(
                ("member-group", index as u64),
                &format!("{name} — {count}"),
            ))
            .into_any_element(),
        Some(MemberRow::Member { user_id }) => match app.store.user(*user_id) {
            Some(user) => member_row(app, guild_id, user, cx).into_any_element(),
            None => placeholder_row(),
        },
        None => placeholder_row(),
    }
}

fn placeholder_row() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .h(rpx(44.))
        .px_2()
        .child(
            div()
                .size(rpx(32.))
                .flex_none()
                .rounded_full()
                .bg(rgb(theme::SELECTED)),
        )
        .child(
            div()
                .h(rpx(12.))
                .w(rpx(110.))
                .rounded(rpx(6.))
                .bg(rgb(theme::SELECTED)),
        )
        .into_any_element()
}

fn member_row(app: &DiscordApp, guild_id: u64, user: &User, cx: &App) -> impl IntoElement + use<> {
    let online = user.status.is_online();
    let name = app
        .store
        .member_name(guild_id, user.id)
        .unwrap_or(&user.name)
        .to_owned();
    let name_color = app
        .store
        .member_color(guild_id, user.id)
        .unwrap_or(theme::TEXT);
    div()
        .id(("member", user.id))
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .h(rpx(44.))
        .px_2()
        .rounded(rpx(8.))
        .overflow_hidden()
        .cursor_pointer()
        .when(!online, |this| this.opacity(0.35))
        .hover(|style| style.bg(rgb(theme::HOVER)).opacity(1.))
        .child(theme::member_avatar(
            &app.store,
            guild_id,
            user.id,
            32.,
            online.then_some((user.status, theme::BG)),
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .min_w_0()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(name_color))
                                .child(label(("member-name", user.id), &name)),
                        )
                        .when(user.bot, |this| this.child(theme::app_tag()))
                        .children(user.tag.as_ref().map(|tag| theme::guild_tag(tag, cx))),
                )
                .children(user.activity.as_ref().filter(|_| online).map(|activity| {
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(4.))
                        .min_w_0()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(theme::MUTED))
                        .when(user.playing, |this| {
                            this.child(theme::icon("gamepad-2", 14., theme::GREEN))
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(label(("member-activity", user.id), activity)),
                        )
                })),
        )
}
