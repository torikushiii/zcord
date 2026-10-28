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

pub(super) fn render_row(app: &DiscordApp, index: usize, cx: &App) -> AnyElement {
    let Some(channel) = app.channel() else {
        return div().into_any_element();
    };
    let messages = &channel.messages;
    let Some(message) = messages.get(index) else {
        return div().into_any_element();
    };
    let previous = index.checked_sub(1).and_then(|index| messages.get(index));
    let new_day = previous.is_some_and(|previous| !previous.timestamp.same_day(message.timestamp));
    let grouped = previous.is_some_and(|previous| {
        !new_day
            && previous.author_id == message.author_id
            && message.timestamp.minutes_since(previous.timestamp) < GROUP_MINUTES
            && message.reply.is_none()
            && !is_system(previous.kind)
            && !is_system(message.kind)
    });
    let start = index == 0 && app.history_complete.contains(&app.selected_channel_id);
    let first_unread = app
        .unread_since
        .is_some_and(|since| is_first_unread(messages, index, since, start));
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

pub(super) fn is_first_unread(messages: &[Message], index: usize, since: u64, start: bool) -> bool {
    let message = &messages[index];
    (index > 0 || start)
        && message.delivery == Delivery::Sent
        && message.id > since
        && messages[..index]
            .iter()
            .rev()
            .find(|previous| previous.delivery == Delivery::Sent)
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
