use super::requests::{RequestButton, mutual_servers, request_button};
use super::*;

pub(crate) fn dm_header(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let channel_id = app.selected_channel_id;
    let dm = app.store.private_channel(channel_id);
    let name = app.store.private_channel_name(channel_id);
    let user = dm
        .filter(|dm| !dm.group)
        .and_then(|dm| dm.recipient_ids.first())
        .and_then(|id| app.store.user(*id));
    let game = user
        .filter(|user| user.status.is_online() && user.playing)
        .and_then(|user| user.activity.clone());
    let group = dm.is_some_and(|dm| dm.group);
    let toggle = app.entity.clone();
    div()
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .h(rpx(chat::HEADER_HEIGHT))
        .flex_none()
        .pl_4()
        .pr_2()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .children(dm.map(|dm| {
            div()
                .flex_none()
                .size(rpx(24.))
                .child(header_avatar(app, dm, cx))
        }))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(label(("header-dm", channel_id), &name)),
        )
        .children(game.map(|game| {
            div()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .min_w_0()
                .text_size(rpx(14.))
                .text_color(rgb(theme::muted()))
                .child(theme::icon("gamepad-2", 14., theme::GREEN))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(label(("header-game", channel_id), &game)),
                )
        }))
        .children(dm_menu::header_error(app, channel_id))
        .child(div().flex_1())
        .child(dm_menu::header_actions(app, channel_id))
        .child(chat::pins_button(app))
        .child(
            theme::icon_button(
                "dm-members",
                if group { "users" } else { "user" },
                20.,
                if app.show_members {
                    theme::strong()
                } else {
                    theme::muted()
                },
            )
            .on_click(move |_, _, cx| {
                toggle.update(cx, |app, cx| app.toggle_members(cx)).ok();
            }),
        )
        .child(
            div()
                .flex_none()
                .w(rpx(244.))
                .ml_2()
                .mr_2()
                .child(search::search_box(app, cx)),
        )
}

fn header_avatar(app: &DiscordApp, dm: &PrivateChannel, cx: &App) -> AnyElement {
    if dm.group {
        return div()
            .flex()
            .items_center()
            .justify_center()
            .size(rpx(24.))
            .rounded_full()
            .bg(theme::user_color(dm.channel.id))
            .child(theme::icon("users", 14., theme::strong()))
            .into_any_element();
    }
    let user_id = dm.recipient_ids.first().copied().unwrap_or(0);
    theme::user_avatar(app.store.user(user_id), user_id, 24., None, false, cx).into_any_element()
}

pub(crate) fn dm_welcome(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    dm_intro(app, app.selected_channel_id, cx)
}

/// The top of a DM's history. A message request also gets Discord's sidebar controls under
/// the greeting: the sender's mutual servers, then Accept DM, Ignore and Report Spam.
pub(crate) fn dm_intro(app: &DiscordApp, channel_id: u64, cx: &App) -> Option<AnyElement> {
    let dm = app.store.private_channel(channel_id)?;
    let name = app.store.private_channel_name(channel_id);
    let user_id = dm.recipient_ids.first().copied().unwrap_or(0);
    let user = (!dm.group).then(|| app.store.user(user_id)).flatten();
    let text = if dm.group {
        format!("Welcome to the beginning of the {name} group.")
    } else {
        format!("This is the beginning of your direct message history with {name}.")
    };
    Some(
        div()
            .flex()
            .flex_col()
            .mx_4()
            .mt_4()
            .child(if dm.group {
                header_avatar(app, dm, cx)
            } else {
                theme::user_avatar(user, user_id, 80., None, false, cx).into_any_element()
            })
            .child(
                div()
                    .mt_2()
                    .text_size(rpx(32.))
                    .line_height(rpx(40.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(rgb(theme::strong()))
                    .child(label(("welcome-dm", channel_id), &name)),
            )
            .children(user.map(|user| {
                div()
                    .text_size(rpx(20.))
                    .text_color(rgb(theme::text()))
                    .child(crate::settings::shown_username(&user.username))
            }))
            .child(
                div()
                    .mt_2()
                    .text_size(rpx(16.))
                    .text_color(rgb(theme::muted()))
                    .child(text),
            )
            .when(dm.message_request && !dm.group, |this| {
                this.child(request_intro_controls(app, dm, user_id, cx))
            })
            .into_any_element(),
    )
}

fn request_intro_controls(
    app: &DiscordApp,
    dm: &PrivateChannel,
    user_id: u64,
    cx: &App,
) -> impl IntoElement {
    let id = dm.channel.id;
    let buttons = if dm.spam {
        [RequestButton::NotSpam, RequestButton::Report].as_slice()
    } else {
        [
            RequestButton::Accept,
            RequestButton::Ignore,
            RequestButton::Report,
        ]
        .as_slice()
    };
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(rpx(8.))
        .mt(rpx(12.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(theme::muted()))
        .children(
            app.home
                .mutuals
                .get(&user_id)
                .map(|guild_ids| mutual_servers(app, guild_ids, cx)),
        )
        .when(app.home.mutuals.contains_key(&user_id), |this| {
            this.child(div().text_color(rgb(theme::faint())).child("•"))
        })
        .children(buttons.iter().map(|kind| request_button(app, id, *kind)))
}
