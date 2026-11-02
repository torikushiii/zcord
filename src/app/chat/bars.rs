use super::*;

pub(super) fn is_detached(app: &DiscordApp) -> bool {
    app.channel()
        .is_some_and(|channel| channel.messages_loaded && !app.store.at_present(channel.id))
}

pub(super) fn new_messages_bar(
    app: &DiscordApp,
    entity: WeakEntity<DiscordApp>,
) -> Option<AnyElement> {
    if !app.unread_bar || is_detached(app) {
        return None;
    }
    let since = app.unread_since?;
    let channel = app.channel()?;
    let index = channel.first_unread_index(since, app.store.current_user_id)?;
    if app.messages.item_is_above_viewport(index) != Some(true) {
        return None;
    }
    let truncated = index == 0 && !app.history_complete.contains(&channel.id);
    let count = channel.unread_message_count(since, app.store.current_user_id);
    let timestamp = channel.messages[index].timestamp;
    let today = chrono::Local::now().date_naive();
    let since_text = if (
        u32::from(timestamp.year),
        u32::from(timestamp.month),
        u32::from(timestamp.day),
    ) == (
        chrono::Datelike::year(&today) as u32,
        chrono::Datelike::month(&today),
        chrono::Datelike::day(&today),
    ) {
        timestamp.time()
    } else {
        format!("{} on {}", timestamp.time(), timestamp.date())
    };
    let label = format!(
        "{count}{} new message{} since {since_text}",
        if truncated { "+" } else { "" },
        if count == 1 && !truncated { "" } else { "s" },
    );
    let jump = entity.clone();
    Some(
        div()
            .absolute()
            .top_0()
            .left(rpx(16.))
            .right(rpx(16.))
            .flex()
            .items_center()
            .h(rpx(28.))
            .px(rpx(12.))
            .rounded_b(rpx(8.))
            .bg(rgb(theme::BLURPLE))
            .text_size(rpx(14.))
            .text_color(rgb(theme::strong()))
            .child(
                div()
                    .id("new-messages")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        jump.update(cx, |app, cx| app.jump_to_unread(cx)).ok();
                    })
                    .child(label),
            )
            .child(
                div()
                    .id("mark-as-read")
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .on_click(move |_, window, cx| {
                        entity
                            .update(cx, |app, cx| app.jump_to_present(window, cx))
                            .ok();
                    })
                    .child("Mark As Read"),
            )
            .into_any_element(),
    )
}

pub(super) fn jump_missing_notice(app: &DiscordApp) -> Option<AnyElement> {
    app.jump_missing.then(|| {
        div()
            .absolute()
            .top(rpx(8.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .px(rpx(16.))
                    .py(rpx(6.))
                    .rounded(rpx(12.))
                    .border_1()
                    .border_color(rgb(theme::placeholder_strong()))
                    .bg(rgb(theme::elevated()))
                    .shadow_lg()
                    .text_size(rpx(15.))
                    .line_height(rpx(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::text()))
                    .child("Couldn't find that message"),
            )
            .into_any_element()
    })
}

pub(super) fn older_messages_bar(
    app: &DiscordApp,
    entity: WeakEntity<DiscordApp>,
) -> Option<AnyElement> {
    if !app.live || !is_detached(app) {
        return None;
    }
    let jump = entity;
    // Floating pill centred above the composer; the full-width wrapper has no
    // listeners, so clicks beside the pill still reach the list.
    Some(
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(rpx(8.))
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(12.))
                    .pl(rpx(16.))
                    .pr(rpx(6.))
                    .py(rpx(6.))
                    .rounded(rpx(12.))
                    .border_1()
                    .border_color(rgb(theme::placeholder_strong()))
                    .bg(rgb(theme::elevated()))
                    .shadow_lg()
                    .child(
                        div()
                            .text_size(rpx(15.))
                            .line_height(rpx(20.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::text()))
                            .whitespace_nowrap()
                            .child("You're Viewing Older Messages"),
                    )
                    .child(
                        div()
                            .id("jump-to-present")
                            .flex_none()
                            .px(rpx(12.))
                            .py(rpx(6.))
                            .rounded(rpx(8.))
                            .bg(rgb(theme::BLURPLE))
                            .hover(|style| style.bg(rgb(0x4752C4)))
                            .text_size(rpx(15.))
                            .line_height(rpx(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xFFFFFF))
                            .whitespace_nowrap()
                            .cursor_pointer()
                            .on_click(move |_, window, cx| {
                                jump.update(cx, |app, cx| app.jump_to_present(window, cx))
                                    .ok();
                            })
                            .child("Jump To Present"),
                    ),
            )
            .into_any_element(),
    )
}
