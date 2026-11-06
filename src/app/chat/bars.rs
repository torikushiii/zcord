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

fn reply_visible(app: &DiscordApp, reply_id: u64) -> bool {
    app.message_index(reply_id).is_some_and(|index| {
        app.messages.item_is_above_viewport(index) == Some(false)
            && app.messages.item_is_below_viewport(index) == Some(false)
    })
}

const BOTTOM_BAR_OFFSET: f32 = 28.;
const BRAND_HOVER: u32 = 0x4752C4;

type BarAction = fn(&mut DiscordApp, &mut Window, &mut Context<DiscordApp>);

enum BarEnd {
    Button(&'static str),
    Text(&'static str),
    Loading,
}

pub(super) fn history_bar(app: &DiscordApp, entity: WeakEntity<DiscordApp>) -> Option<AnyElement> {
    let channel_id = app.selected_channel_id;
    if !app.live || !app.channel().is_some_and(|channel| channel.messages_loaded) {
        return None;
    }
    let loading = app.loading.contains(&channel_id);
    let (id, text, end, action): (_, _, _, BarAction) = if app.history_failed.contains(&channel_id)
    {
        (
            "messages-failed",
            "Messages Failed To Load",
            if loading {
                BarEnd::Loading
            } else {
                BarEnd::Text("Try Again")
            },
            |app, window, cx| app.retry_history(window, cx),
        )
    } else if let Some((_, reply_id)) = app
        .jump_return
        .filter(|(channel, _)| *channel == channel_id)
        && !(loading && app.pending_present.contains(&channel_id))
        && !reply_visible(app, reply_id)
    {
        (
            "jump-to-reply",
            "You Are Viewing a Reply.",
            BarEnd::Button("Jump To Original Message"),
            |app, window, cx| app.jump_back_to_reply(window, cx),
        )
    } else if !is_detached(app) {
        return None;
    } else if loading && app.pending_present.contains(&channel_id) {
        (
            "jump-to-present",
            "You're Viewing Older Messages",
            BarEnd::Loading,
            |_, _, _| {},
        )
    } else {
        (
            "jump-to-present",
            "You're Viewing Older Messages",
            BarEnd::Button("Jump To Present"),
            |app, window, cx| app.jump_to_present(window, cx),
        )
    };
    // The typing indicator overhangs the list's bottom, so the bar rises above it.
    let typing = if app.typing_users().is_empty() {
        0.
    } else {
        super::composer::TYPING_HEIGHT
    };
    let busy = matches!(end, BarEnd::Loading);
    let end = match end {
        BarEnd::Button(label) => div()
            .id("bar-button")
            .flex_none()
            .flex()
            .items_center()
            .h(rpx(32.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgb(theme::BLURPLE))
            .hover(|style| style.bg(rgb(BRAND_HOVER)))
            .active(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(0xFFFFFF))
            .whitespace_nowrap()
            .child(label)
            .into_any_element(),
        BarEnd::Text(label) => div()
            .flex_none()
            .pl(rpx(4.))
            .pr(rpx(12.))
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme::strong()))
            .whitespace_nowrap()
            .child(label)
            .into_any_element(),
        BarEnd::Loading => pulsing_ellipsis().into_any_element(),
    };
    let failed = id == "messages-failed";
    // The full-width wrapper has no listeners, so clicks beside the bar still reach the list.
    Some(
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(rpx(BOTTOM_BAR_OFFSET.max(8. + typing)))
            .flex()
            .justify_center()
            .px(rpx(16.))
            .child(
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .min_w(rpx(284.))
                    .min_h(rpx(44.))
                    .max_w_full()
                    .py(rpx(8.))
                    .when(failed, |bar| bar.pl(rpx(16.)).pr(rpx(8.)))
                    .when(!failed, |bar| bar.px(rpx(12.)))
                    .rounded(rpx(8.))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .bg(rgb(theme::surface_highest()))
                    .shadow_md()
                    .when(!busy, |bar| {
                        bar.cursor_pointer()
                            .active(|style| style.mb(rpx(-1.)))
                            .on_click(move |_, window, cx| {
                                entity.update(cx, |app, cx| action(app, window, cx)).ok();
                            })
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pl(rpx(4.))
                            .pr(rpx(12.))
                            .truncate()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::text()))
                            .child(text),
                    )
                    .child(end),
            )
            .into_any_element(),
    )
}

fn pulsing_ellipsis() -> impl IntoElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .pr(rpx(12.))
        .with_animation(
            "bar-ellipsis",
            Animation::new(Duration::from_millis(1400)).repeat(),
            |dots, delta| {
                dots.children((0..3).map(|index| {
                    let phase = (delta - index as f32 * 0.2).rem_euclid(1.);
                    let pulse = 1. - (phase * 2. - 1.).abs();
                    div()
                        .size(rpx(6.))
                        .rounded_full()
                        .bg(rgb(theme::strong()))
                        .opacity(0.3 + 0.7 * pulse)
                }))
            },
        )
}
