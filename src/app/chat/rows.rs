use super::message::render_message;
use super::*;

/// Rendering only reads, so fetch a row's unseen invite links after this frame.
pub(super) fn request_invites(app: &Entity<DiscordApp>, index: usize, cx: &mut App) {
    let content = {
        let app = app.read(cx);
        let Some(message) = app
            .channel()
            .and_then(|channel| channel.messages.get(index))
        else {
            return;
        };
        if !crate::discord::model::invite_codes(&message.content)
            .iter()
            .any(|code| !app.invites.contains_key(code))
        {
            return;
        }
        message.content.clone()
    };
    let app = app.downgrade();
    cx.defer(move |cx| {
        app.update(cx, |app, cx| app.prefetch_invites(&content, cx))
            .ok();
    });
}

pub(super) fn request_thread_preview(app: &Entity<DiscordApp>, index: usize, cx: &mut App) {
    let thread_id = {
        let app = app.read(cx);
        let Some(thread) = app
            .channel()
            .and_then(|channel| channel.messages.get(index))
            .and_then(|message| message.thread.as_ref())
            .filter(|thread| thread.recent.is_none() && !app.thread_previews.contains(&thread.id))
        else {
            return;
        };
        thread.id
    };
    let app = app.downgrade();
    cx.defer(move |cx| {
        app.update(cx, |app, cx| app.load_thread_preview(thread_id, cx))
            .ok();
    });
}

fn row_grouping(messages: &[Message], index: usize) -> (bool, bool) {
    let message = &messages[index];
    let previous = index.checked_sub(1).and_then(|index| messages.get(index));
    let new_day = previous.is_some_and(|previous| !previous.timestamp.same_day(message.timestamp));
    let grouped = previous.is_some_and(|previous| {
        !new_day
            && previous.same_author(message)
            && message.timestamp.minutes_since(previous.timestamp) < GROUP_MINUTES
            && message.reply.is_none()
            && !is_system(previous.kind)
            && !is_system(message.kind)
    });
    (new_day, grouped)
}

pub(super) fn render_pane_row(
    app: &DiscordApp,
    channel_id: u64,
    index: usize,
    cx: &App,
) -> AnyElement {
    let Some(channel) = app.channel_by_id(channel_id) else {
        return div().into_any_element();
    };
    let messages = &channel.messages;
    let Some(message) = messages.get(index) else {
        return div().into_any_element();
    };
    let (new_day, grouped) = row_grouping(messages, index);
    let start = index == 0 && app.history_complete.contains(&channel_id);
    let post = app.store.forum_post(channel_id);
    let intro = start
        .then_some(post)
        .flatten()
        .map(|post| post_intro(app, post, &channel.name, cx));
    let first_unread = app
        .thread_pane_unread_since(channel_id)
        .is_some_and(|since| {
            is_first_unread(messages, index, since, start, app.store.current_user_id)
        });
    // A forum post's starter message shares the thread's id.
    let starter = post.is_some() && message.id == channel_id;
    div()
        .w_full()
        .children(intro)
        .map(|this| {
            if new_day || start {
                this.child(date_divider(message, first_unread))
            } else if first_unread {
                this.child(unread_divider())
            } else {
                this
            }
        })
        .when(!grouped, |this| this.pt_4())
        .when(index + 1 == messages.len(), |this| {
            this.pb(rpx(BOTTOM_SPACER))
        })
        .child(render_message(app, message, grouped, true, cx))
        .when(starter, |this| this.child(starter_actions(app, channel_id)))
        .into_any_element()
}

pub(super) fn render_row(app: &DiscordApp, index: usize, cx: &App) -> AnyElement {
    let Some(channel) = app.channel() else {
        return div().into_any_element();
    };
    let messages = &channel.messages;
    let Some(message) = messages.get(index) else {
        return div().into_any_element();
    };
    let (new_day, grouped) = row_grouping(messages, index);
    let start = index == 0 && app.history_complete.contains(&app.selected_channel_id);
    let first_unread = app.unread_since.is_some_and(|since| {
        is_first_unread(messages, index, since, start, app.store.current_user_id)
    });
    div()
        .w_full()
        .when(start, |this| this.children(welcome(app, cx)))
        .map(|this| {
            if new_day || start {
                this.child(date_divider(message, first_unread))
            } else if first_unread {
                this.child(unread_divider())
            } else {
                this
            }
        })
        .when(index + 1 == messages.len(), |this| {
            this.pb(rpx(BOTTOM_SPACER))
        })
        .child(render_message(app, message, grouped, false, cx))
        .into_any_element()
}

pub(super) fn is_first_unread(
    messages: &[Message],
    index: usize,
    since: u64,
    start: bool,
    me: u64,
) -> bool {
    let message = &messages[index];
    (index > 0 || start)
        && message.delivery == Delivery::Sent
        && message.author_id != me
        && message.id > since
        && messages[..index]
            .iter()
            .rev()
            .find(|previous| previous.delivery == Delivery::Sent && previous.author_id != me)
            .is_none_or(|previous| previous.id <= since)
}

pub(super) fn is_system(kind: u8) -> bool {
    !matches!(kind, 0 | 19 | 20 | 23)
}

pub(super) fn welcome(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    if app.is_home() {
        return home::dm_welcome(app, cx);
    }
    let channel = app.channel()?;
    let subtitle = || div().mt_2().text_color(rgb(0xB5BAC1));
    if let Some(guild) = app.guild().filter(|guild| guild.id == channel.id) {
        return Some(
            div()
                .flex()
                .flex_col()
                .items_center()
                .mx_4()
                .mt_4()
                .pt(rpx(32.))
                .text_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .text_size(rpx(32.))
                        .line_height(rpx(40.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::STRONG))
                        .child("Welcome to")
                        .child(wrapped_label(("welcome-guild", guild.id), &guild.name)),
                )
                .child(
                    subtitle()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .child("This is the beginning of this server."),
                )
                .into_any_element(),
        );
    }
    Some(
        div()
            .flex()
            .flex_col()
            .mx_4()
            .mt_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(68.))
                    .rounded_full()
                    .bg(rgb(0x1F1F23))
                    .child(theme::icon("hash", 42., theme::STRONG)),
            )
            .child(
                div()
                    .mt_2()
                    .text_size(rpx(32.))
                    .line_height(rpx(40.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_color(rgb(theme::STRONG))
                    .child(wrapped_label(
                        ("welcome-title", channel.id),
                        &format!("Welcome to #{}!", channel.name),
                    )),
            )
            .child(
                subtitle()
                    .text_size(rpx(16.))
                    .line_height(rpx(20.))
                    .child(wrapped_label(
                        ("welcome-subtitle", channel.id),
                        &format!("This is the start of the #{} channel.", channel.name),
                    )),
            )
            .into_any_element(),
    )
}

fn starter_actions(app: &DiscordApp, thread_id: u64) -> impl IntoElement {
    let joined = app
        .store
        .thread_row_by_id(thread_id)
        .is_some_and(|row| row.joined);
    let follow = app.entity.clone();
    let link = format!(
        "https://discord.com/channels/{}/{thread_id}",
        app.selected_guild_id
    );
    let button = |id: &'static str| {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(32.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgba(0x97979F1F))
            .border_1()
            .border_color(rgba(theme::BORDER_SUBTLE))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::HOVER)))
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::STRONG))
    };
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .justify_end()
                .gap(rpx(8.))
                .px_4()
                .py(rpx(8.))
                .child(
                    button("post-follow-button")
                        .on_click(move |_, _, cx| {
                            follow
                                .update(cx, |app, cx| {
                                    app.join_or_leave_thread(thread_id, joined, cx)
                                })
                                .ok();
                        })
                        .child(theme::icon("bell", 16., theme::STRONG))
                        .child(if joined { "Following" } else { "Follow" }),
                )
                .child(
                    button("post-link-button")
                        .px(rpx(8.))
                        .relative()
                        .child(theme::hover_tooltip("post-link-tip", "Copy Link"))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                        })
                        .child(theme::icon("link", 16., theme::STRONG)),
                ),
        )
        .child(div().h(rpx(4.)).bg(rgba(theme::BORDER_SUBTLE)))
}

fn post_intro(
    app: &DiscordApp,
    post: &crate::store::ForumPost,
    name: &str,
    cx: &App,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .mx_4()
        .pt(rpx(32.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(56.))
                .child(theme::icon("message-circle", 40., theme::STRONG)),
        )
        .child(
            div()
                .text_size(rpx(32.))
                .line_height(rpx(40.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::STRONG))
                .child(wrapped_label(("post-intro", post.id), name)),
        )
        .children(forum::post_tags(app, post, cx))
}

fn date_divider(message: &Message, unread: bool) -> impl IntoElement {
    let color = if unread {
        rgb(theme::RED)
    } else {
        rgba(theme::BORDER)
    };
    let line = || div().flex_1().h(rpx(1.)).bg(color);
    div()
        .flex()
        .items_center()
        .gap_2()
        .mx_4()
        .mt_6()
        .mb_2()
        .child(line())
        .child(
            div()
                .text_size(rpx(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(if unread { theme::RED } else { theme::FAINT }))
                .child(message.timestamp.date()),
        )
        .child(line())
        .when(unread, |this| this.child(new_badge()))
}

fn unread_divider() -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .mx_4()
        .my_2()
        .child(div().flex_1().h(rpx(1.)).bg(rgb(theme::RED)))
        .child(new_badge())
}

fn new_badge() -> impl IntoElement {
    div()
        .flex_none()
        .h(rpx(14.))
        .px(rpx(4.))
        .rounded(rpx(4.))
        .bg(rgb(theme::RED))
        .text_size(rpx(10.))
        .line_height(rpx(14.))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(theme::STRONG))
        .child("NEW")
}
