use gpui_component::scroll::ScrollableElement as _;
use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{prelude::*, *};
use gpui_component::{
    ElementExt,
    input::{Textarea, TextareaState},
};

use crate::{
    app::{
        DIALOG_EXIT, DISCORD_EPOCH_MS, DiscordApp, Editing, JUMP_HIGHLIGHT, MESSAGE_LIMIT,
        ReplyTarget, embeds,
        emoji_picker::{self, PickerTarget},
        forum, home, member_list,
        message_body::{BodyContext, label, preview_pieces, render_body, wrapped_label},
        profile,
        rich_text::rich_text,
    },
    media::animated,
    media::images::{self, ImageSpec},
    store::{Attachment, Delivery, Forward, Message, PendingUpload, ReplyState, Sticker},
    text::composer_tokens,
    theme::{self, WithoutTextSelection, rpx},
};

mod actions;
mod bars;
mod composer;
mod dialog;
mod media;
mod message;
mod popouts;
mod rows;
mod skeleton;
mod thread_pane;
mod thread_ui;
use crate::app::invite_card;
pub(super) use actions::{MessageMenu, message_menu};
use bars::{is_detached, jump_missing_notice, new_messages_bar, older_messages_bar};
pub(crate) use composer::composer;
pub(super) use dialog::delete_dialog;
pub(super) use media::file_size;
pub(super) use message::{author_name, body_context};
pub(in crate::app) use popouts::{
    ChannelPopout, PopoutKind, channel_popout, pins_button, threads_button,
};
use rows::{render_row, request_invites, request_thread_preview, welcome};
use skeleton::skeleton;
pub(in crate::app) use thread_pane::ThreadPane;
pub(crate) use thread_pane::{WIDTH as THREAD_PANE_WIDTH, render_thread_pane};
pub(in crate::app) use thread_ui::{CreateThread, ThreadMenu, create_thread_dialog, thread_menu};

const GROUP_MINUTES: u32 = 7;
const GUTTER: f32 = 72.;
const BOTTOM_SPACER: f32 = 24.;
const STICKER_SIZE: f32 = 160.;
pub(crate) const LINK: u32 = theme::LINK;
const DIALOG_ENTER: Duration = Duration::from_millis(200);

#[cfg(test)]
mod reply_tests;

pub(crate) const HEADER_HEIGHT: f32 = 48.;

pub fn header(app: &DiscordApp) -> impl IntoElement + use<> {
    let channel_name = app.channel().map_or("", |channel| &channel.name).to_owned();
    let guild_name = app.guild().map_or("", |guild| &guild.name).to_owned();
    let entity = app.entity.clone();
    let search = div()
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .h(rpx(32.))
        .px_2()
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgba(theme::BORDER))
        .text_size(rpx(14.))
        .text_color(rgb(theme::FAINT))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(format!("Search {guild_name}")),
        )
        .child(theme::icon("search", 16., theme::MUTED));
    div()
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .h(rpx(HEADER_HEIGHT))
        .flex_none()
        .pl_4()
        .border_b_1()
        .border_color(rgba(theme::BORDER))
        .child(theme::icon(header_icon(app), 20., theme::FAINT))
        .children(forum_crumb(app))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::STRONG))
                .child(label(
                    ("header-channel", app.selected_channel_id),
                    &channel_name,
                )),
        )
        .child(div().flex_1())
        .child(threads_button(app))
        .child(theme::icon_button(
            "notifications",
            "bell",
            20.,
            theme::MUTED,
        ))
        .child(pins_button(app))
        .children(thread_ui::thread_menu_button(app))
        .child(
            theme::icon_button(
                "members",
                "users",
                20.,
                if app.show_members {
                    theme::STRONG
                } else {
                    theme::MUTED
                },
            )
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.toggle_members(cx)).ok();
            }),
        )
        .map(|this| {
            if app.show_members {
                this.child(
                    div()
                        .flex_none()
                        .w(rpx(member_list::WIDTH))
                        .px(rpx(8.))
                        .child(search),
                )
            } else {
                this.child(div().flex_none().w(rpx(244.)).ml_2().mr_2().child(search))
            }
        })
}

fn header_icon(app: &DiscordApp) -> &'static str {
    if app.thread_parent().is_some() || app.selected_forum().is_some() {
        return "messages-square";
    }
    match app.channel().map(|channel| channel.kind) {
        Some(crate::store::ChannelKind::Announcement) => "megaphone",
        _ => "hash",
    }
}

fn forum_crumb(app: &DiscordApp) -> Option<impl IntoElement + use<>> {
    let forum_id = app.thread_parent()?;
    let name = app
        .guild()
        .and_then(|guild| guild.channel(forum_id))
        .map(|forum| forum.name.clone())?;
    let entity = app.entity.clone();
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("forum-crumb")
                    .text_size(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::MUTED))
                    .cursor_pointer()
                    .hover(|style| style.text_color(rgb(theme::TEXT)).underline())
                    .child(label(("forum-crumb", forum_id), &name))
                    .on_click(move |_, window, cx| {
                        entity
                            .update(cx, |app, cx| app.back_to_forum(window, cx))
                            .ok();
                    }),
            )
            .child(
                div()
                    .text_size(rpx(18.))
                    .text_color(rgb(theme::FAINT))
                    .child("›"),
            ),
    )
}

/// The composer and member list are separate sibling views.
pub fn messages(app: &DiscordApp, cx: &App) -> AnyElement {
    if let Some(forum_id) = app.selected_forum() {
        return forum::render(app, forum_id, cx).into_any_element();
    }
    let channel = app.channel();
    let list_entity = app.entity.clone();
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .w_full()
        .h_full()
        .map(|this| {
            if channel.is_some_and(|channel| app.live && !channel.messages_loaded) {
                if app.is_loading(app.selected_channel_id) {
                    this.child(skeleton(app.selected_channel_id))
                } else {
                    this.child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .text_color(rgb(theme::MUTED))
                            .child("Couldn't load messages."),
                    )
                }
            } else if channel.is_some_and(|channel| channel.messages.is_empty())
                && app.history_complete.contains(&app.selected_channel_id)
            {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .justify_end()
                        .flex_1()
                        .min_h_0()
                        .pb(rpx(BOTTOM_SPACER))
                        .children(welcome(app, cx)),
                )
            } else {
                let bar = if is_detached(app) {
                    None
                } else {
                    new_messages_bar(app, app.entity.clone())
                };
                let older = older_messages_bar(app, app.entity.clone());
                let missing = jump_missing_notice(app);
                this.child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(
                            list(app.messages.clone(), move |index, _, cx| {
                                let Some(app) = list_entity.upgrade() else {
                                    return div().into_any_element();
                                };
                                request_invites(&app, index, cx);
                                request_thread_preview(&app, index, cx);
                                render_row(app.read(cx), index, cx)
                            })
                            .flex_1()
                            .min_h_0()
                            .w_full(),
                        )
                        .vertical_scrollbar(&app.messages)
                        .children(bar)
                        .children(older)
                        .children(missing),
                )
            }
        })
        .into_any_element()
}
