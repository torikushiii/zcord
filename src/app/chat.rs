use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_component::{input::Textarea, tooltip::Tooltip};

use crate::{
    app::{
        DIALOG_EXIT, DISCORD_EPOCH_MS, DiscordApp, Editing, MESSAGE_LIMIT, ReplyTarget, embeds,
        emoji_picker::{self, PickerTarget},
        member_list,
        message_body::{BodyContext, label, preview_pieces, render_body, wrapped_label},
        profile,
        rich_text::rich_text,
    },
    images,
    store::{Attachment, Delivery, Forward, Message, ReplyState},
    theme::{self, rpx},
};

const GROUP_MINUTES: u32 = 7;
const GUTTER: f32 = 72.;
const BOTTOM_SPACER: f32 = 24.;
const LINK: u32 = 0x00A8FC;
const DIALOG_ENTER: Duration = Duration::from_millis(200);

#[cfg(test)]
mod reply_tests;

pub fn render(app: &DiscordApp, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    let channel = app.channel();
    let channel_name = channel.map_or("", |channel| &channel.name).to_owned();
    let guild_name = app.guild().map_or("", |guild| &guild.name).to_owned();
    let entity = cx.entity();
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
        .border_color(rgb(theme::BORDER))
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
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .h(rpx(48.))
                .flex_none()
                .pl_4()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .child(theme::icon("hash", 20., theme::FAINT))
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
                .child(theme::icon_button(
                    "threads",
                    "messages-square",
                    20.,
                    theme::MUTED,
                ))
                .child(theme::icon_button(
                    "notifications",
                    "bell",
                    20.,
                    theme::MUTED,
                ))
                .child(theme::icon_button("pins", "pin", 20., theme::MUTED))
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
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_members(cx))),
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
                }),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(messages_column(app, entity, cx))
                .when(app.show_members, |this| {
                    this.child(member_list::render(app, cx))
                }),
        )
}

fn messages_column(
    app: &DiscordApp,
    entity: Entity<DiscordApp>,
    cx: &App,
) -> impl IntoElement + use<> {
    let channel = app.channel();
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
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
                        .children(welcome(app)),
                )
            } else {
                let bar = if is_detached(app) {
                    None
                } else {
                    new_messages_bar(app, entity.clone())
                };
                let older = older_messages_bar(app, entity.clone());
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
                                render_row(entity.read(cx), index, cx)
                            })
                            .flex_1()
                            .min_h_0()
                            .w_full(),
                        )
                        .children(bar)
                        .children(older),
                )
            }
        })
        .child(composer(app, cx))
}

fn composer(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let can_send = app.store.can_send(app.selected_channel_id);
    let length = app.composer.read(cx).value().chars().count();
    let remaining = MESSAGE_LIMIT as i64 - length as i64;
    div()
        .relative()
        .px_2()
        .pb_2()
        .flex_none()
        .children(app.reply.map(|reply| reply_banner(app, reply)))
        .child(
            div()
                .relative()
                .flex()
                .items_start()
                .gap_1()
                .min_h(rpx(56.))
                .pl(rpx(12.))
                .pr(rpx(8.))
                .map(|this| {
                    if app.reply.is_some() {
                        this.rounded_b(rpx(8.))
                    } else {
                        this.rounded(rpx(8.))
                    }
                })
                .bg(rgb(theme::ELEVATED))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .children(emoji_picker::suggestions(app, cx))
                .child(div().my(rpx(12.)).child(theme::icon_button(
                    "attach",
                    "circle-plus",
                    24.,
                    theme::MUTED,
                )))
                .child(
                    div().flex_1().min_w_0().ml_4().child(
                        flush_textarea(Textarea::new(&app.composer))
                            .disabled(!can_send)
                            .py(rpx(17.))
                            .text_size(rpx(16.))
                            .line_height(rpx(22.))
                            .text_color(rgb(theme::TEXT)),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .my(rpx(12.))
                        .child(theme::icon_button("gift", "gift", 24., theme::MUTED))
                        .child(theme::icon_button(
                            "translate",
                            "languages",
                            24.,
                            theme::MUTED,
                        ))
                        .child(theme::icon_button("gif", "image-play", 24., theme::MUTED))
                        .child(theme::icon_button("stickers", "sticker", 24., theme::MUTED))
                        .child({
                            let entity = app.entity.clone();
                            theme::icon_button("emoji", "smile", 24., theme::MUTED).on_mouse_down(
                                MouseButton::Left,
                                move |event, window, cx| {
                                    let position = event.position - point(px(0.), px(16.));
                                    entity
                                        .update(cx, |app, cx| {
                                            app.toggle_emoji_picker(
                                                PickerTarget::Composer,
                                                position,
                                                window,
                                                cx,
                                            )
                                        })
                                        .ok();
                                },
                            )
                        })
                        .child(theme::icon_button("apps", "layout-grid", 24., theme::MUTED)),
                )
                .when(remaining < 200, |this| {
                    this.child(
                        div()
                            .absolute()
                            .right(rpx(12.))
                            .bottom(rpx(4.))
                            .text_size(rpx(12.))
                            .text_color(rgb(if remaining < 0 {
                                theme::RED
                            } else {
                                theme::FAINT
                            }))
                            .child(remaining.to_string()),
                    )
                }),
        )
        .children(typing_indicator(app))
}

const TEXTAREA_BUILTIN_INSET_X: f32 = 10.;
const TEXTAREA_BUILTIN_INSET_Y: f32 = 8.;

fn flush_textarea(textarea: Textarea) -> Textarea {
    textarea
        .appearance(false)
        .bordered(false)
        .w_full()
        .px_0()
        .ml(px(-TEXTAREA_BUILTIN_INSET_X))
        .my(px(-TEXTAREA_BUILTIN_INSET_Y))
}

fn reply_banner(app: &DiscordApp, reply: ReplyTarget) -> impl IntoElement {
    let entity = app.entity.clone();
    let own = reply.author_id == app.store.current_user_id;
    div()
        .flex()
        .items_center()
        .h(rpx(34.))
        .pl(rpx(16.))
        .pr(rpx(8.))
        .rounded_t(rpx(8.))
        .bg(rgb(theme::HOVER))
        .border_1()
        .border_b_0()
        .border_color(rgb(theme::BORDER))
        .text_size(rpx(14.))
        .text_color(rgb(theme::MUTED))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .gap(rpx(4.))
                .child("Replying to")
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::TEXT))
                        .child(label(
                            ("reply-target", reply.message_id),
                            &author_name(app, reply.author_id),
                        )),
                ),
        )
        .when(!own, |this| {
            let toggle = entity.clone();
            this.child(
                div()
                    .id("reply-mention")
                    .flex()
                    .items_center()
                    .gap(rpx(2.))
                    .px_2()
                    .cursor_pointer()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(if reply.mention { LINK } else { theme::MUTED }))
                    .tooltip(|window, cx| Tooltip::new("Mention the author").build(window, cx))
                    .on_click(move |_, _, cx| {
                        toggle
                            .update(cx, |app, cx| app.toggle_reply_mention(cx))
                            .ok();
                    })
                    .child("@")
                    .child(if reply.mention { "ON" } else { "OFF" }),
            )
            .child(div().w(rpx(1.)).h(rpx(16.)).mx_1().bg(rgb(theme::BORDER)))
        })
        .child(
            theme::icon_button("reply-cancel", "x", 16., theme::MUTED).on_click(move |_, _, cx| {
                entity.update(cx, |app, cx| app.cancel_reply(cx)).ok();
            }),
        )
}

fn typing_indicator(app: &DiscordApp) -> Option<impl IntoElement> {
    let names: Vec<String> = app
        .typing_users()
        .iter()
        .map(|(user_id, _)| author_name(app, *user_id))
        .collect();
    let bold = |name: &String| {
        div()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(theme::TEXT))
            .child(name.clone())
    };
    let text = div().flex().min_w_0().overflow_hidden().whitespace_nowrap();
    let text = match names.as_slice() {
        [] => text,
        [one] => text.child(bold(one)).child("\u{a0}is typing…"),
        [one, two] => text
            .child(bold(one))
            .child("\u{a0}and\u{a0}")
            .child(bold(two))
            .child("\u{a0}are typing…"),
        [one, two, three] => text
            .child(bold(one))
            .child(",\u{a0}")
            .child(bold(two))
            .child(", and\u{a0}")
            .child(bold(three))
            .child("\u{a0}are typing…"),
        _ => text.child("Several people are typing…"),
    };
    if names.is_empty() {
        return None;
    }
    Some(
        div()
            .absolute()
            .bottom(relative(1.))
            .left(rpx(8.))
            .right(rpx(8.))
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(24.))
            .pl(rpx(8.))
            .bg(rgb(theme::BG))
            .text_size(rpx(12.))
            .text_color(rgb(theme::MUTED))
            .child(
                div()
                    .flex()
                    .gap(rpx(2.))
                    .children(
                        (0..3).map(|_| div().size(rpx(4.)).rounded_full().bg(rgb(theme::TEXT))),
                    )
                    .with_animation(
                        "typing-dots",
                        Animation::new(Duration::from_millis(1200))
                            .repeat()
                            .with_easing(pulsating_between(0.3, 1.)),
                        |dots, delta| dots.opacity(delta),
                    ),
            )
            .child(text),
    )
}

struct Shuffle(u64);

impl Shuffle {
    fn next(&mut self, range: std::ops::Range<u32>) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        range.start + ((self.0 >> 33) as u32) % (range.end - range.start)
    }
}

const SKELETON_GROUPS: usize = 14;

fn skeleton(seed: u64) -> impl IntoElement {
    let mut shuffle = Shuffle(seed ^ 0x9E37_79B9_7F4A_7C15);
    let bar = |width: f32, height: f32, color: u32| {
        div()
            .flex_none()
            .w(rpx(width))
            .h(rpx(height))
            .rounded(rpx(height / 2.))
            .bg(rgb(color))
    };
    let groups: Vec<_> = (0..SKELETON_GROUPS)
        .map(|_| {
            let name = shuffle.next(60..140) as f32;
            let lines: Vec<_> = (0..shuffle.next(1..4))
                .map(|_| {
                    let words: Vec<_> = (0..shuffle.next(2..9))
                        .map(|_| bar(shuffle.next(24..90) as f32, 14., theme::SELECTED))
                        .collect();
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(rpx(4.))
                        .min_h(rpx(22.))
                        .children(words)
                })
                .collect();
            let image = (shuffle.next(0..6) == 0).then(|| {
                div()
                    .mt_1()
                    .w(rpx(shuffle.next(200..320) as f32))
                    .h(rpx(shuffle.next(120..200) as f32))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::SELECTED))
            });
            div()
                .flex()
                .flex_none()
                .w_full()
                .mt_4()
                .py(rpx(2.))
                .pr_4()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_center()
                        .w(rpx(GUTTER))
                        .child(
                            div()
                                .mt(rpx(2.))
                                .size(rpx(40.))
                                .rounded_full()
                                .bg(rgb(theme::SELECTED)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().flex().items_center().h(rpx(22.)).child(bar(
                            name,
                            16.,
                            theme::PLACEHOLDER_STRONG,
                        )))
                        .children(lines)
                        .children(image),
                )
        })
        .collect();
    div()
        .flex()
        .flex_col()
        .justify_end()
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_hidden()
        .pb(rpx(BOTTOM_SPACER))
        .children(groups)
}

fn render_row(app: &DiscordApp, index: usize, cx: &App) -> AnyElement {
    let Some(messages) = app.channel().map(|channel| &channel.messages) else {
        return div().into_any_element();
    };
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
    let first_unread = app.unread_since.is_some_and(|since| {
        message.id > since && previous.map_or(start, |previous| previous.id <= since)
    });
    div()
        .w_full()
        .when(start, |this| this.children(welcome(app)))
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

fn is_system(kind: u8) -> bool {
    !matches!(kind, 0 | 19 | 20 | 23)
}

fn welcome(app: &DiscordApp) -> Option<AnyElement> {
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
    let color = if unread { theme::RED } else { theme::BORDER };
    let line = || div().flex_1().h(rpx(1.)).bg(rgb(color));
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

fn is_detached(app: &DiscordApp) -> bool {
    app.channel()
        .is_some_and(|channel| channel.messages_loaded && !app.store.at_present(channel.id))
}

fn new_messages_bar(app: &DiscordApp, entity: Entity<DiscordApp>) -> Option<AnyElement> {
    if !app.unread_bar || is_detached(app) {
        return None;
    }
    let since = app.unread_since?;
    let channel = app.channel()?;
    let index = channel
        .messages
        .iter()
        .position(|message| message.id > since)?;
    if app.messages.item_is_above_viewport(index) != Some(true) {
        return None;
    }
    let truncated = index == 0 && !app.history_complete.contains(&channel.id);
    let count = channel.messages.len() - index;
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
            .text_color(rgb(theme::STRONG))
            .child(
                div()
                    .id("new-messages")
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        jump.update(cx, |app, cx| app.jump_to_unread(cx));
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
                        entity.update(cx, |app, cx| app.jump_to_present(window, cx));
                    })
                    .child("Mark As Read"),
            )
            .into_any_element(),
    )
}

fn older_messages_bar(app: &DiscordApp, entity: Entity<DiscordApp>) -> Option<AnyElement> {
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
                    .border_color(rgb(theme::PLACEHOLDER_STRONG))
                    .bg(rgb(theme::ELEVATED))
                    .shadow_lg()
                    .child(
                        div()
                            .text_size(rpx(15.))
                            .line_height(rpx(20.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::TEXT))
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
                                jump.update(cx, |app, cx| app.jump_to_present(window, cx));
                            })
                            .child("Jump To Present"),
                    ),
            )
            .into_any_element(),
    )
}

fn body_context<'a>(app: &'a DiscordApp, message_id: u64) -> BodyContext<'a> {
    BodyContext {
        store: &app.store,
        guild_id: app.selected_guild_id,
        message_id,
        revealed: &app.revealed_spoilers,
        entity: app.entity.clone(),
        colors: Default::default(),
    }
}

fn forward_key(message_id: u64) -> u64 {
    embeds::part_key(message_id, embeds::FORWARD_SLOT)
}

fn author_name(app: &DiscordApp, user_id: u64) -> String {
    app.store
        .member_name(app.selected_guild_id, user_id)
        .unwrap_or("Unknown user")
        .to_owned()
}

fn author_label(app: &DiscordApp, message: &Message) -> AnyElement {
    let name = author_name(app, message.author_id);
    let style = app
        .store
        .member_name_style(app.selected_guild_id, message.author_id)
        .filter(|_| {
            app.store
                .member_color(app.selected_guild_id, message.author_id)
                .is_none()
        });
    match style {
        Some(style) => theme::display_name(&name, Some(style), theme::STRONG),
        None => label(("author", message.id), &name),
    }
}

fn author_color(app: &DiscordApp, user_id: u64) -> Hsla {
    app.store
        .member_color(app.selected_guild_id, user_id)
        .map_or(rgb(theme::STRONG).into(), |color| rgb(color).into())
}

const JOIN_MESSAGES: [&str; 13] = [
    "{} joined the party.",
    "{} is here.",
    "Welcome, {}. We hope you brought pizza.",
    "A wild {} appeared.",
    "{} just landed.",
    "{} just slid into the server.",
    "{} just showed up!",
    "Welcome {}. Say hi!",
    "{} hopped into the server.",
    "Everyone welcome {}!",
    "Glad you're here, {}.",
    "Good to see you, {}.",
    "Yay you made it, {}!",
];

fn system_text(message: &Message) -> Option<(&'static str, u32, String)> {
    let template = match message.kind {
        7 => {
            let created = (message.id >> 22) + DISCORD_EPOCH_MS;
            return Some((
                "arrow-right",
                theme::GREEN,
                JOIN_MESSAGES[(created % JOIN_MESSAGES.len() as u64) as usize].into(),
            ));
        }
        6 => (
            "pin",
            theme::MUTED,
            "{} pinned a message to this channel.".into(),
        ),
        8 => ("gem", 0xFF73FA, "{} just boosted the server!".into()),
        9..=11 => (
            "gem",
            0xFF73FA,
            format!(
                "{{}} just boosted the server! This server has achieved Level {}!",
                message.kind - 8
            ),
        ),
        18 => (
            "messages-square",
            theme::MUTED,
            "{} started a thread.".into(),
        ),
        _ => return None,
    };
    Some(template)
}

fn render_system(app: &DiscordApp, message: &Message) -> Option<AnyElement> {
    let (icon, color, template) = system_text(message)?;
    let (before, after) = template.split_once("{}").unwrap_or((&template, ""));
    Some(
        div()
            .id(("message", message.id))
            .flex()
            .items_center()
            .w_full()
            .pr_4()
            .py(rpx(2.))
            .mt(rpx(4.))
            .hover(|style| style.bg(rgb(theme::HOVER)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_center()
                    .w(rpx(GUTTER))
                    .child(theme::icon(icon, 16., color)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_baseline()
                    .gap(rpx(4.))
                    .min_w_0()
                    .text_size(rpx(16.))
                    .line_height(rpx(22.))
                    .text_color(rgb(theme::MUTED))
                    .child(
                        div()
                            .flex()
                            .child(before.to_owned())
                            .child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(author_color(app, message.author_id))
                                    .child(label(
                                        ("system-author", message.id),
                                        &author_name(app, message.author_id),
                                    )),
                            )
                            .child(after.to_owned()),
                    )
                    .child(
                        div()
                            .text_size(rpx(12.))
                            .text_color(rgb(theme::FAINT))
                            .child(message.timestamp.full()),
                    ),
            )
            .into_any_element(),
    )
}

fn attachment_reply_preview() -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .min_w_0()
        .child(
            div()
                .min_w_0()
                .truncate()
                .italic()
                .debug_selector(|| "reply-attachment-label".into())
                .child("Click to see attachment"),
        )
        .child(
            div()
                .flex_none()
                .debug_selector(|| "reply-attachment-icon".into())
                .child(theme::icon("image", 18., theme::MUTED)),
        )
}

fn reply_bar(app: &DiscordApp, message: &Message, reply: &ReplyState, cx: &App) -> AnyElement {
    let spine = div()
        .absolute()
        .left(rpx(GUTTER / 2.))
        .top(rpx(11.))
        .w(rpx(GUTTER / 2. - 6.))
        .h(rpx(13.))
        .border_l_2()
        .border_t_2()
        .rounded_tl(rpx(6.))
        .border_color(rgb(0x4E5058));
    let content = div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(theme::MUTED));
    let content = match reply {
        ReplyState::Loaded(reply) => {
            let parsed = app.parsed(reply.message_id, &reply.content);
            let context = body_context(app, reply.message_id);
            let mut counter = 0;
            let pieces = preview_pieces(&parsed.blocks, &context, &mut counter, cx);
            let preview: AnyElement = if pieces.is_empty() {
                if reply.attachments || reply.embeds {
                    attachment_reply_preview().into_any_element()
                } else {
                    div().into_any_element()
                }
            } else {
                rich_text(("reply", message.id), pieces)
                    .single_line()
                    .line_height(rpx(18.))
                    .emoji_size(rpx(16.))
                    .into_any_element()
            };
            let target = reply.message_id;
            let entity = app.entity.clone();
            content
                .id(("reply-preview", message.id))
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    entity
                        .update(cx, |app, cx| app.jump_to_message(target, window, cx))
                        .ok();
                })
                .child(
                    div()
                        .flex_none()
                        .text_size(rpx(11.))
                        .text_color(rgb(theme::FAINT))
                        .child(reply.timestamp.time()),
                )
                .child(theme::member_avatar(
                    &app.store,
                    app.selected_guild_id,
                    reply.author_id,
                    16.,
                    None,
                    false,
                    cx,
                ))
                .child(
                    div()
                        .flex_none()
                        .max_w(relative(0.5))
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(author_color(app, reply.author_id))
                        .child(label(
                            ("reply-author", message.id),
                            &author_name(app, reply.author_id),
                        )),
                )
                .child(div().flex_1().min_w_0().overflow_hidden().child(preview))
                .into_any_element()
        }
        ReplyState::Deleted => content
            .child(div().italic().child("Original message was deleted"))
            .into_any_element(),
        ReplyState::Unavailable => content
            .child(div().italic().child("Message could not be loaded"))
            .into_any_element(),
    };
    div()
        .relative()
        .flex()
        .items_center()
        .h(rpx(22.))
        .child(spine)
        .child(div().w(rpx(GUTTER)).flex_none())
        .child(content)
        .into_any_element()
}

fn render_forward(
    app: &DiscordApp,
    message_id: u64,
    forward: &Forward,
    cx: &App,
) -> AnyElement {
    let key = forward_key(message_id);
    let parsed = app.parsed(key, &forward.content);
    let context = body_context(app, key);
    let show_body =
        !parsed.blocks.is_empty() && !embeds::hides_content(&forward.content, &forward.embeds);
    let channel = forward
        .channel_id
        .and_then(|channel_id| app.store.channel_name(channel_id));
    let footer = (channel.is_some() || forward.timestamp.is_some()).then(|| {
        let text = match (channel, forward.timestamp) {
            (Some(channel), Some(timestamp)) => format!("#{channel} • {}", timestamp.full()),
            (Some(channel), None) => format!("#{channel}"),
            (None, timestamp) => timestamp.map(|timestamp| timestamp.full()).unwrap_or_default(),
        };
        div()
            .mt(rpx(2.))
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .text_color(rgb(theme::FAINT))
            .child(text)
    });
    div()
        .flex()
        .flex_col()
        .w_full()
        .mt(rpx(2.))
        .py(rpx(2.))
        .pl(rpx(12.))
        .rounded_l(rpx(4.))
        .border_l_4()
        .border_color(rgb(theme::BORDER))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .child(theme::icon("forward", 16., theme::MUTED))
                .child(
                    div()
                        .italic()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::MUTED))
                        .child("Forwarded"),
                ),
        )
        .when(show_body, |this| {
            this.child(
                div()
                    .w_full()
                    .text_size(rpx(16.))
                    .line_height(rpx(22.))
                    .child(render_body(
                        &parsed.blocks,
                        forward.edited,
                        parsed.jumbo,
                        &context,
                        cx,
                    )),
            )
        })
        .children(
            forward
                .attachments
                .iter()
                .map(|attachment| render_attachment(app, attachment, cx)),
        )
        .children(
            forward
                .embeds
                .iter()
                .enumerate()
                .map(|(index, embed)| embeds::render_embed(app, key, index, embed, cx)),
        )
        .children(embeds::render_components(app, key, &forward.components, cx))
        .children(footer)
        .into_any_element()
}

fn render_message(
    app: &DiscordApp,
    message: &Message,
    grouped: bool,
    preview: bool,
    cx: &App,
) -> AnyElement {
    if is_system(message.kind)
        && let Some(element) = render_system(app, message)
    {
        return element;
    }
    let user = app.store.user(message.author_id);
    let parsed = app.parsed(message.id, &message.content);
    let context = body_context(app, message.id);
    let rich = !message.embeds.is_empty() || !message.components.is_empty();
    let trailing_edited = message.edited && parsed.blocks.is_empty() && rich;
    let show_body = (!parsed.blocks.is_empty() || message.edited)
        && !trailing_edited
        && !embeds::hides_content(&message.content, &message.embeds);
    let editing = app
        .editing
        .as_ref()
        .filter(|editing| !preview && editing.message_id == message.id);
    let replying = !preview
        && app
            .reply
            .is_some_and(|reply| reply.message_id == message.id);
    let entity = app.entity.clone();
    let id = message.id;
    div()
        .id(("message", message.id))
        .group("message")
        .relative()
        .flex()
        .flex_col()
        .w_full()
        .pr_4()
        .py(rpx(2.))
        .when(!grouped && !preview, |this| this.mt_4())
        .when(!preview, |this| {
            this.on_hover(move |hovered, _, cx| {
                entity
                    .update(cx, |app, cx| app.set_message_hover(id, *hovered, false, cx))
                    .ok();
            })
        })
        .map(|this| {
            if replying {
                this.bg(rgba(0x5865F21A)).child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(rpx(2.))
                        .bg(rgb(theme::BLURPLE)),
                )
            } else if message.mentions_me {
                this.bg(rgba(0xF0B2321A))
                    .hover(|style| style.bg(rgba(0xF0B23224)))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .bottom_0()
                            .w(rpx(2.))
                            .bg(rgb(theme::YELLOW)),
                    )
            } else {
                this.when(!preview, |this| {
                    this.hover(|style| style.bg(rgb(theme::HOVER)))
                })
            }
        })
        .children(
            message
                .reply
                .as_ref()
                .map(|reply| reply_bar(app, message, reply, cx)),
        )
        .child(
            div()
                .flex()
                .w_full()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .justify_center()
                        .w(rpx(GUTTER))
                        .map(|gutter| {
                            if grouped {
                                gutter.pt(rpx(4.)).child(
                                    div()
                                        .text_size(rpx(11.))
                                        .text_color(transparent_black())
                                        .group_hover("message", |style| {
                                            style.text_color(rgb(theme::FAINT))
                                        })
                                        .child(message.timestamp.time()),
                                )
                            } else {
                                gutter.pt(rpx(2.)).child(
                                    div()
                                        .id("author-avatar")
                                        .when(!preview, |this| {
                                            this.cursor_pointer().on_click(profile::opener(
                                                app,
                                                message.author_id,
                                                Anchor::TopLeft,
                                            ))
                                        })
                                        .child(theme::member_avatar(
                                            &app.store,
                                            app.selected_guild_id,
                                            message.author_id,
                                            40.,
                                            None,
                                            !preview && app.hovered_message == Some(message.id),
                                            cx,
                                        )),
                                )
                            }
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .when(!grouped, |this| {
                            this.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rpx(6.))
                                    .child(
                                        div()
                                            .id("author-name")
                                            .text_size(rpx(16.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(author_color(app, message.author_id))
                                            .when(!preview, |this| {
                                                this.cursor_pointer()
                                                    .hover(|style| style.underline())
                                                    .on_click(profile::opener(
                                                        app,
                                                        message.author_id,
                                                        Anchor::TopLeft,
                                                    ))
                                            })
                                            .child(author_label(app, message)),
                                    )
                                    .when(user.is_some_and(|user| user.bot), |this| {
                                        this.child(theme::app_tag())
                                    })
                                    .children(
                                        user.and_then(|user| user.tag.as_ref())
                                            .map(|tag| theme::guild_tag(tag, cx)),
                                    )
                                    .child(
                                        div()
                                            .text_size(rpx(12.))
                                            .text_color(rgb(theme::FAINT))
                                            .child(message.timestamp.full()),
                                    ),
                            )
                        })
                        .map(|this| match editing {
                            Some(editing) => this.child(message_editor(app, editing)),
                            None if show_body => this.child(
                                div()
                                    .w_full()
                                    .text_size(rpx(16.))
                                    .line_height(rpx(22.))
                                    .when(message.delivery == Delivery::Pending, |this| {
                                        this.opacity(0.5)
                                    })
                                    .when(message.delivery == Delivery::Failed, |this| {
                                        this.text_color(rgb(theme::RED))
                                    })
                                    .child(render_body(
                                        &parsed.blocks,
                                        message.edited,
                                        parsed.jumbo,
                                        &context,
                                        cx,
                                    )),
                            ),
                            None => this,
                        })
                        .children(
                            message
                                .forward
                                .as_ref()
                                .map(|forward| render_forward(app, message.id, forward, cx)),
                        )
                        .when(message.delivery == Delivery::Failed && !preview, |this| {
                            this.child(send_failure(app, message.id))
                        })
                        .children(message.embeds.iter().enumerate().map(|(index, embed)| {
                            embeds::render_embed(app, message.id, index, embed, cx)
                        }))
                        .children(embeds::render_components(
                            app,
                            message.id,
                            &message.components,
                            cx,
                        ))
                        .children(
                            message
                                .attachments
                                .iter()
                                .map(|attachment| render_attachment(app, attachment, cx)),
                        )
                        .when(trailing_edited, |this| {
                            this.child(
                                div()
                                    .mt(rpx(2.))
                                    .text_size(rpx(10.))
                                    .text_color(rgb(theme::FAINT))
                                    .child("(edited)"),
                            )
                        })
                        .children(embeds::render_reactions(app, message.id, &message.reactions, cx)),
                ),
        )
        .when(!preview && editing.is_none(), |this| {
            this.children(message_actions(app, message))
        })
        .into_any_element()
}

fn message_editor(app: &DiscordApp, editing: &Editing) -> impl IntoElement {
    let cancel = app.entity.clone();
    let save = app.entity.clone();
    let link = |id: &'static str, text: &'static str| {
        div()
            .id(id)
            .cursor_pointer()
            .text_color(rgb(LINK))
            .hover(|style| style.underline())
            .child(text)
    };
    div()
        .flex()
        .flex_col()
        .w_full()
        .child(
            div()
                .mt_1()
                .px(rpx(16.))
                .rounded(rpx(8.))
                .bg(rgb(theme::ELEVATED))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .child(
                    flush_textarea(Textarea::new(&editing.input))
                        .py(rpx(11.))
                        .text_size(rpx(16.))
                        .line_height(rpx(22.))
                        .text_color(rgb(theme::TEXT)),
                ),
        )
        .child(
            div()
                .flex()
                .py(rpx(4.))
                .text_size(rpx(12.))
                .text_color(rgb(theme::TEXT))
                .child("escape to\u{a0}")
                .child(
                    link("edit-cancel", "cancel").on_click(move |_, window, cx| {
                        cancel
                            .update(cx, |app, cx| app.cancel_edit(window, cx))
                            .ok();
                    }),
                )
                .child("\u{a0}• enter to\u{a0}")
                .child(link("edit-save", "save").on_click(move |_, window, cx| {
                    save.update(cx, |app, cx| app.save_edit(window, cx)).ok();
                })),
        )
}

fn send_failure(app: &DiscordApp, message_id: u64) -> impl IntoElement {
    let retry = app.entity.clone();
    let dismiss = app.entity.clone();
    let error = app
        .failed_sends
        .get(&message_id)
        .map_or("Message failed to send.", |failed| failed.error.as_str())
        .to_owned();
    let action = |id: (&'static str, u64), text: &'static str| {
        div()
            .id(id)
            .cursor_pointer()
            .text_color(rgb(LINK))
            .hover(|style| style.underline())
            .child(text)
    };
    div()
        .flex()
        .flex_wrap()
        .gap(rpx(6.))
        .text_size(rpx(12.))
        .text_color(rgb(theme::RED))
        .child(error)
        .child(
            action(("retry-send", message_id), "Retry").on_click(move |_, window, cx| {
                retry
                    .update(cx, |app, cx| app.retry_message(message_id, window, cx))
                    .ok();
            }),
        )
        .child(
            action(("dismiss-send", message_id), "Dismiss").on_click(move |_, window, cx| {
                dismiss
                    .update(cx, |app, cx| app.discard_message(message_id, window, cx))
                    .ok();
            }),
        )
}

fn message_actions(app: &DiscordApp, message: &Message) -> Option<AnyElement> {
    let id = message.id;
    if !(app.hovered_message == Some(id) || app.hovered_actions == Some(id))
        || message.delivery != Delivery::Sent
        || is_system(message.kind) && !matches!(message.kind, 19)
    {
        return None;
    }
    let channel_id = app.selected_channel_id;
    let own = message.author_id == app.store.current_user_id;
    let can_reply = app.store.can_send(channel_id);
    let can_delete = own || app.store.can_manage_messages(channel_id);
    let button = |key: &'static str, icon: &str, color: u32, tip: &'static str| {
        theme::icon_button((key, id), icon, 20., color)
            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
    };
    let hover = app.entity.clone();
    let edit = app.entity.clone();
    let reply = app.entity.clone();
    let delete = app.entity.clone();
    Some(
        div()
            .id(("message-actions", id))
            .absolute()
            .top(rpx(-16.))
            .right(rpx(14.))
            .flex()
            .p(rpx(2.))
            .rounded(rpx(8.))
            .bg(rgb(theme::ELEVATED))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .block_mouse_except_scroll()
            .on_hover(move |hovered, _, cx| {
                hover
                    .update(cx, |app, cx| app.set_message_hover(id, *hovered, true, cx))
                    .ok();
            })
            .when(own && matches!(message.kind, 0 | 19), |this| {
                this.child(button("edit", "pencil", theme::MUTED, "Edit").on_click(
                    move |_, window, cx| {
                        edit.update(cx, |app, cx| app.start_edit(id, window, cx))
                            .ok();
                    },
                ))
            })
            .when(can_reply, |this| {
                this.child(button("reply", "reply", theme::MUTED, "Reply").on_click(
                    move |_, window, cx| {
                        reply
                            .update(cx, |app, cx| app.start_reply(id, window, cx))
                            .ok();
                    },
                ))
            })
            .when(can_delete, |this| {
                this.child(button("delete", "trash-2", theme::RED, "Delete").on_click(
                    move |event, window, cx| {
                        let confirmed = event.modifiers().shift;
                        delete
                            .update(cx, |app, cx| app.request_delete(id, confirmed, window, cx))
                            .ok();
                    },
                ))
            })
            .into_any_element(),
    )
}

fn protip() -> StyledText {
    const TIP: &str = "You can hold down shift when clicking delete message to bypass this confirmation entirely.";
    const BOLD: &str = "delete message";
    let start = TIP.find(BOLD).unwrap_or_default();
    StyledText::new(TIP).with_highlights([(
        start..start + BOLD.len(),
        HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        },
    )])
}

fn dialog_button(id: &'static str, text: &'static str, bg: u32, hover: u32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .h(rpx(44.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(rgb(bg))
        .hover(move |style| style.bg(rgb(hover)))
        .text_size(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::STRONG))
        .child(text)
}

pub(super) fn delete_dialog(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let message_id = app.confirm_delete?;
    let message = app
        .channel()?
        .messages
        .iter()
        .find(|message| message.id == message_id)?;
    let closing = app.delete_dialog_closing();
    let (key, duration) = if closing {
        ("delete-dialog-out", DIALOG_EXIT)
    } else {
        ("delete-dialog-in", DIALOG_ENTER)
    };
    let animation = Animation::new(duration).with_easing(ease_out_quint());
    let progress = move |delta: f32| if closing { 1. - delta } else { delta };
    let backdrop = app.entity.clone();
    let close = app.entity.clone();
    let cancel = app.entity.clone();
    let confirm = app.entity.clone();
    Some(
        div()
            .id("delete-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .on_click(move |_, _, cx| {
                backdrop.update(cx, |app, cx| app.cancel_delete(cx)).ok();
            })
            .with_animation((key, message_id), animation.clone(), move |this, delta| {
                this.bg(hsla(0., 0., 0., 0.7 * progress(delta)))
            })
            .child(
                div()
                    .id("delete-dialog")
                    .relative()
                    .flex()
                    .flex_col()
                    .w(rpx(440.))
                    .max_w_full()
                    .p(rpx(24.))
                    .rounded(rpx(12.))
                    .bg(rgb(theme::ELEVATED))
                    .border_1()
                    .border_color(rgb(theme::BORDER))
                    .occlude()
                    .child(
                        theme::icon_button("delete-close", "x", 24., theme::MUTED)
                            .absolute()
                            .top(rpx(16.))
                            .right(rpx(16.))
                            .on_click(move |_, _, cx| {
                                close.update(cx, |app, cx| app.cancel_delete(cx)).ok();
                            }),
                    )
                    .child(
                        div()
                            .text_size(rpx(20.))
                            .line_height(rpx(24.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::STRONG))
                            .child("Delete Message"),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_size(rpx(16.))
                            .text_color(rgb(theme::MUTED))
                            .child("Are you sure you want to delete this message?"),
                    )
                    .child(
                        div()
                            .mt_4()
                            .max_h(rpx(300.))
                            .overflow_hidden()
                            .py_3()
                            .rounded(rpx(8.))
                            .bg(rgb(theme::BG))
                            .border_1()
                            .border_color(rgb(theme::BORDER))
                            .child(render_message(app, message, false, true, cx)),
                    )
                    .child(
                        div()
                            .mt_4()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .text_color(rgb(theme::TEXT))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::GREEN))
                                    .child("Protip:"),
                            )
                            .child(protip()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .mt_6()
                            .child(
                                dialog_button("delete-cancel", "Cancel", 0x232327, 0x2C2C31)
                                    .border_1()
                                    .border_color(rgb(theme::BORDER))
                                    .on_click(move |_, _, cx| {
                                        cancel.update(cx, |app, cx| app.cancel_delete(cx)).ok();
                                    }),
                            )
                            .child(
                                dialog_button("delete-confirm", "Delete", 0xD22D39, 0xB82A33)
                                    .on_click(move |_, window, cx| {
                                        confirm
                                            .update(cx, |app, cx| {
                                                app.request_delete(message_id, true, window, cx)
                                            })
                                            .ok();
                                    }),
                            ),
                    )
                    .with_animation((key, message_id), animation, move |this, delta| {
                        let shown = progress(delta);
                        this.opacity(shown).top(rpx(16. * (1. - shown)))
                    }),
            )
            .into_any_element(),
    )
}

const MEDIA_MAX: (f32, f32) = (550., 350.);

fn render_attachment(app: &DiscordApp, attachment: &Attachment, cx: &App) -> AnyElement {
    if attachment.is_video() {
        let Some(url) = attachment.video_url() else {
            return file_card(attachment).into_any_element();
        };
        let (width, height) = (
            attachment.width.unwrap_or(1).max(1) as f32,
            attachment.height.unwrap_or(1).max(1) as f32,
        );
        let scale = (MEDIA_MAX.0 / width).min(MEDIA_MAX.1 / height).min(1.);
        let (width, height) = ((width * scale).round(), (height * scale).round());
        let poster = attachment.video_poster_spec();
        if attachment.spoiler() && !app.revealed_spoilers.contains(&(attachment.id, 0)) {
            let media = poster.as_ref().map_or_else(
                || {
                    div()
                        .relative()
                        .flex_none()
                        .w(rpx(width))
                        .h(rpx(height))
                        .rounded(rpx(8.))
                        .overflow_hidden()
                        .bg(rgb(theme::BORDER))
                },
                |spec| embeds::blurred_box(spec, width, height, 8., cx),
            );
            return media
                .mt_1()
                .child(embeds::spoiler_overlay(app, attachment.id, 8.))
                .into_any_element();
        }
        return div()
            .mt_1()
            .child(crate::player::video_box(
                app,
                crate::player::Video {
                    key: attachment.id,
                    url: &url,
                    poster,
                    width,
                    height,
                    radius: 8.,
                },
                cx,
            ))
            .into_any_element();
    }
    let Some(spec) = attachment.image_spec() else {
        return file_card(attachment).into_any_element();
    };
    let (original_width, original_height) = (
        attachment.width.unwrap_or(1).max(1),
        attachment.height.unwrap_or(1).max(1),
    );
    let (width, height) = (original_width as f32, original_height as f32);
    let scale = (MEDIA_MAX.0 / width).min(MEDIA_MAX.1 / height).min(1.);
    let (width, height) = ((width * scale).round(), (height * scale).round());
    if attachment.spoiler() && !app.revealed_spoilers.contains(&(attachment.id, 0)) {
        return embeds::blurred_box(&spec, width, height, 8., cx)
            .mt_1()
            .child(embeds::spoiler_overlay(app, attachment.id, 8.))
            .into_any_element();
    }
    let preview = theme::image_px(width.max(height));
    let image = images::resolve(&spec, preview, cx);
    let entity = app.entity.clone();
    let url = attachment.url.clone();
    div()
        .id(("lightbox-open", attachment.id))
        .relative()
        .mt_1()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(rgb(theme::ELEVATED))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| {
                    app.open_lightbox(
                        spec.clone(),
                        (original_width, original_height),
                        preview,
                        url.clone(),
                        cx,
                    )
                })
                .ok();
        })
        .children(image.map(|path| {
            embeds::animated(path, &attachment.url)
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(rpx(8.))
        }))
        .into_any_element()
}

fn file_card(attachment: &Attachment) -> impl IntoElement + use<> {
    div()
        .flex()
        .items_center()
        .gap_3()
        .mt_1()
        .max_w(rpx(432.))
        .p_3()
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .child(theme::icon("file", 32., theme::MUTED))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_color(rgb(LINK))
                        .child(attachment.filename.clone()),
                )
                .child(
                    div()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::FAINT))
                        .child(file_size(attachment.size)),
                ),
        )
}

pub(super) fn file_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{:.2} KB", bytes as f64 / 1024.),
        _ => format!("{:.2} MB", bytes as f64 / 1_048_576.),
    }
}
