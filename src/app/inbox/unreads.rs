use std::collections::{HashMap, HashSet};

use crate::app::menu::PopupButton as _;
use chrono::{Local, TimeZone};
use gpui_tokio::Tokio;

use super::*;
use crate::discord::model::{ChannelSetting, NotificationLevel};

const BATCH: usize = 5;
const MESSAGES: usize = 25;
/// Discord rejects bulk acks with more read states than this.
const BULK_ACK: usize = 100;

#[derive(Default)]
pub(in crate::app) struct Unreads {
    shown: usize,
    channels: HashMap<u64, Load>,
    collapsed: HashSet<u64>,
}

enum Load {
    Loading,
    Loaded(Vec<Message>),
    Failed,
}

impl DiscordApp {
    pub(super) fn load_unreads(&mut self, cx: &mut Context<Self>) {
        if let Some(inbox) = &mut self.inbox {
            inbox.unreads.shown = inbox.unreads.shown.max(BATCH);
        }
        self.fetch_unread_channels(cx);
    }

    pub(super) fn load_more_unreads(&mut self, cx: &mut Context<Self>) {
        let total = self.store.inbox_unreads(now()).len();
        let Some(inbox) = &mut self.inbox else {
            return;
        };
        if inbox.unreads.shown >= total
            || inbox
                .unreads
                .channels
                .values()
                .any(|load| matches!(load, Load::Loading))
        {
            return;
        }
        inbox.unreads.shown += BATCH;
        self.fetch_unread_channels(cx);
    }

    fn fetch_unread_channels(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let unreads = self.store.inbox_unreads(now());
        let Some(inbox) = &mut self.inbox else {
            return;
        };
        for unread in unreads.into_iter().take(inbox.unreads.shown) {
            if inbox.unreads.channels.contains_key(&unread.channel_id) {
                continue;
            }
            inbox
                .unreads
                .channels
                .insert(unread.channel_id, Load::Loading);
            let session = session.clone();
            let (guild_id, channel_id, after) =
                (unread.guild_id, unread.channel_id, unread.last_read);
            let request = Tokio::spawn(cx, async move {
                session
                    .fetch_messages_after(Some(guild_id), channel_id, after, MESSAGES)
                    .await
            });
            cx.spawn(async move |this, cx| {
                let result = request.await;
                this.update(cx, |app, cx| {
                    let load = match result {
                        Ok(Ok(messages)) => {
                            let mut messages = app.store.inbox_messages(&messages);
                            messages.sort_by_key(|message| message.id);
                            Load::Loaded(messages)
                        }
                        Ok(Err(error)) => {
                            log::warn!("Could not load unreads for {channel_id}: {error}");
                            Load::Failed
                        }
                        Err(error) => {
                            log::warn!("Unreads task failed for {channel_id}: {error}");
                            Load::Failed
                        }
                    };
                    if let Some(inbox) = &mut app.inbox {
                        inbox.unreads.channels.insert(channel_id, load);
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    fn mark_inbox_channel_read(&mut self, guild_id: u64, channel_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(message_id) = self.store.channel_ack(channel_id) else {
            return;
        };
        self.store.apply(Event::MessageAck {
            channel_id,
            message_id,
        });
        if let Some(inbox) = &mut self.inbox {
            inbox.unreads.channels.remove(&channel_id);
        }
        cx.notify();
        let days = days_since_discord_epoch();
        let request = Tokio::spawn(cx, async move {
            session
                .ack_message(Some(guild_id), channel_id, message_id, days)
                .await
        });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not mark channel read: {error}"),
            Err(error) => log::warn!("Mark read task failed: {error}"),
        })
        .detach();
    }

    fn mark_all_inbox_read(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let acks: Vec<_> = self
            .store
            .inbox_unreads(now())
            .into_iter()
            .filter_map(|unread| {
                Some((
                    unread.channel_id,
                    self.store.channel_ack(unread.channel_id)?,
                ))
            })
            .collect();
        for &(channel_id, message_id) in &acks {
            self.store.apply(Event::MessageAck {
                channel_id,
                message_id,
            });
        }
        if let Some(inbox) = &mut self.inbox {
            inbox.unreads = Unreads::default();
        }
        cx.notify();
        for chunk in acks.chunks(BULK_ACK) {
            let chunk = chunk.to_vec();
            let session = session.clone();
            let request = Tokio::spawn(cx, async move {
                session.ack_bulk(None, chunk[0].0, chunk).await
            });
            cx.spawn(async move |_, _| match request.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::warn!("Could not mark all read: {error}"),
                Err(error) => log::warn!("Mark all read task failed: {error}"),
            })
            .detach();
        }
    }

    fn toggle_unread_collapsed(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        if let Some(inbox) = &mut self.inbox
            && !inbox.unreads.collapsed.remove(&channel_id)
        {
            inbox.unreads.collapsed.insert(channel_id);
        }
        cx.notify();
    }

    fn set_inbox_channel(
        &mut self,
        guild_id: u64,
        channel_id: u64,
        setting: ChannelSetting,
        cx: &mut Context<Self>,
    ) {
        self.set_inbox_menu(None, cx);
        self.run_guild_action(
            "change channel notifications",
            move |session| async move {
                session
                    .set_channel_setting(guild_id, channel_id, setting)
                    .await
            },
            cx,
        );
    }

    fn dismiss_unreads_tip(&mut self, cx: &mut Context<Self>) {
        crate::settings::update(cx, |settings| settings.inbox_unreads_tip_dismissed = true);
        cx.notify();
    }
}

fn now() -> u64 {
    messages::saved::now_ms().max(0) as u64
}

pub(super) fn mark_all_button(app: &DiscordApp) -> AnyElement {
    let entity = app.entity.clone();
    header_button("inbox-mark-all", "check-check", "Mark All as Read", false)
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.mark_all_inbox_read(cx))
                .ok();
        })
        .into_any_element()
}

fn tip(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    if crate::settings::get(cx).inbox_unreads_tip_dismissed {
        return None;
    }
    let entity = app.entity.clone();
    Some(
        div()
            .flex()
            .flex_none()
            .gap(rpx(16.))
            .mb(rpx(16.))
            .p(rpx(16.))
            .rounded(rpx(12.))
            .bg(rgb(theme::elevated()))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(rpx(56.))
                    .child(theme::icon("inbox", 28., theme::text())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(rpx(4.))
                    .child(
                        div()
                            .text_size(rpx(16.))
                            .line_height(rpx(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::strong()))
                            .child("Stay up to date with latest messages!"),
                    )
                    .child(
                        div()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .text_color(rgb(theme::text()))
                            .child(
                                "Unread messages from all your unmuted channels will show up \
                                 here. Time to get to that zero inbox!",
                            ),
                    )
                    .child(
                        div()
                            .id("inbox-unreads-tip")
                            .flex()
                            .items_center()
                            .self_start()
                            .mt(rpx(8.))
                            .h(rpx(32.))
                            .px(rpx(16.))
                            .rounded(rpx(8.))
                            .cursor_pointer()
                            .bg(rgb(theme::BLURPLE))
                            .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
                            .text_size(rpx(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .without_text_selection()
                            .on_click(move |_, _, cx| {
                                entity
                                    .update(cx, |app, cx| app.dismiss_unreads_tip(cx))
                                    .ok();
                            })
                            .child("Got it!"),
                    ),
            )
            .into_any_element(),
    )
}

fn date_divider(timestamp: Timestamp) -> Div {
    let label = Local
        .with_ymd_and_hms(
            i32::from(timestamp.year),
            u32::from(timestamp.month),
            u32::from(timestamp.day),
            0,
            0,
            0,
        )
        .single()
        .map(|date| date.format("%B %-d, %Y").to_string())
        .unwrap_or_default();
    let line = || div().flex_1().h(rpx(1.)).bg(rgba(theme::border()));
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .mx(rpx(16.))
        .mt(rpx(12.))
        .child(line())
        .child(
            div()
                .px(rpx(4.))
                .text_size(rpx(12.))
                .line_height(rpx(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::muted()))
                .child(label),
        )
        .child(line())
}

fn same_day(a: Timestamp, b: Timestamp) -> bool {
    (a.year, a.month, a.day) == (b.year, b.month, b.day)
}

fn unread_body(
    app: &DiscordApp,
    guild_id: u64,
    channel_id: u64,
    messages: &[Message],
    cx: &App,
) -> AnyElement {
    let mut body = div().flex().flex_col().pb(rpx(4.));
    let mut previous: Option<Timestamp> = None;
    for message in messages {
        if previous.is_none_or(|previous| !same_day(previous, message.timestamp)) {
            body = body.child(date_divider(message.timestamp));
        }
        previous = Some(message.timestamp);
        let message_id = message.id;
        let jump = app.entity.clone();
        let group = SharedString::from(format!("inbox-unread-{message_id}"));
        body = body.child(
            div()
                .relative()
                .group(group.clone())
                .child(chat::render_search_message(
                    app,
                    message,
                    CARD_MESSAGE_WIDTH,
                    cx,
                ))
                .child(
                    div()
                        .id(("inbox-unread-jump", message_id))
                        .absolute()
                        .top(rpx(6.))
                        .right(rpx(12.))
                        .invisible()
                        .group_hover(group, |style| style.visible())
                        .px(rpx(8.))
                        .py(rpx(4.))
                        .rounded(rpx(4.))
                        .cursor_pointer()
                        .bg(rgb(theme::surface_highest()))
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .without_text_selection()
                        .on_click(move |_, window, cx| {
                            jump.update(cx, |app, cx| {
                                app.jump_to_saved(
                                    Some(guild_id),
                                    channel_id,
                                    message_id,
                                    window,
                                    cx,
                                )
                            })
                            .ok();
                        })
                        .child("Jump"),
                ),
        );
    }
    body.into_any_element()
}

fn mention_badge(count: u32) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .min_w(rpx(16.))
        .h(rpx(16.))
        .px(rpx(4.))
        .rounded_full()
        .bg(rgb(theme::NOTIFICATION))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white())
        .child(count.to_string())
        .into_any_element()
}

pub(super) fn list(app: &DiscordApp, inbox: &Inbox, window: &Window, cx: &App) -> Vec<AnyElement> {
    let unreads = app.store.inbox_unreads(now());
    let mut items: Vec<AnyElement> = tip(app, cx).into_iter().collect();
    if unreads.is_empty() {
        items.push(note("You're all caught up!"));
        return items;
    }
    for unread in unreads.iter().take(inbox.unreads.shown) {
        let (guild_id, channel_id) = (unread.guild_id, unread.channel_id);
        let collapsed = inbox.unreads.collapsed.contains(&channel_id);
        let bell = app.entity.clone();
        let read = app.entity.clone();
        let collapse = app.entity.clone();
        let menu = InboxMenu::Notifications {
            guild_id,
            channel_id,
        };
        let buttons = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(8.))
            .child(
                card_button(
                    ElementId::NamedInteger("inbox-unread-bell".into(), channel_id),
                    "bell",
                    "Notification Settings",
                )
                .on_popup_click(
                    crate::app::menu::shown(app, move |app| {
                        app.inbox.as_ref().and_then(|inbox| inbox.menu) == Some(menu)
                    }),
                    move |_, _, cx| {
                        bell.update(cx, |app, cx| {
                            let open =
                                app.inbox.as_ref().and_then(|inbox| inbox.menu) == Some(menu);
                            app.set_inbox_menu((!open).then_some(menu), cx);
                        })
                        .ok();
                    },
                ),
            )
            .child(
                card_button(
                    ElementId::NamedInteger("inbox-unread-read".into(), channel_id),
                    "check",
                    "Mark As Read",
                )
                .on_click(move |_, _, cx| {
                    read.update(cx, |app, cx| {
                        app.mark_inbox_channel_read(guild_id, channel_id, cx)
                    })
                    .ok();
                }),
            )
            .child(
                div()
                    .id(("inbox-unread-collapse", channel_id))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(24.))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        collapse
                            .update(cx, |app, cx| app.toggle_unread_collapsed(channel_id, cx))
                            .ok();
                    })
                    .child(theme::icon(
                        if collapsed {
                            "chevron-right"
                        } else {
                            "chevron-down"
                        },
                        16.,
                        theme::muted(),
                    ))
                    .child(theme::hover_tooltip(
                        ("inbox-unread-collapse-tip", channel_id),
                        if collapsed { "Expand" } else { "Collapse" },
                    )),
            );
        let mentions = app.store.mention_count(channel_id);
        let header = card_header(
            app,
            Some(guild_id),
            channel_id,
            (mentions > 0).then(|| mention_badge(mentions)),
            buttons,
            cx,
        )
        .when(inbox.menu == Some(menu), |this| {
            this.child(
                div()
                    .absolute()
                    .top(rpx(48.))
                    .right(rpx(84.))
                    .size_0()
                    .child(notification_menu(app, guild_id, channel_id, window)),
            )
        });
        let body = if collapsed {
            div().into_any_element()
        } else {
            match inbox.unreads.channels.get(&channel_id) {
                Some(Load::Loaded(messages)) if !messages.is_empty() => {
                    unread_body(app, guild_id, channel_id, messages, cx)
                }
                Some(Load::Loaded(_)) => note("No new messages to show."),
                Some(Load::Failed) => note("Could not load these messages."),
                Some(Load::Loading) | None => note("Loading\u{2026}"),
            }
        };
        let header = div()
            .child(header)
            .border_b_1()
            .border_color(rgba(theme::outline()));
        items.push(card(None, header, body));
    }
    items
}

fn notification_menu(
    app: &DiscordApp,
    guild_id: u64,
    channel_id: u64,
    window: &Window,
) -> AnyElement {
    let level = app.store.channel_notification_level(guild_id, channel_id);
    let guild_level = app.store.guild_notification_level(guild_id);
    let level_name = |level: NotificationLevel| match level {
        NotificationLevel::All => "All Messages",
        NotificationLevel::Mentions => "Only @mentions",
        NotificationLevel::Nothing => "Nothing",
        NotificationLevel::Inherit => "Use Category Default",
    };
    let row = |id: &'static str, title: &'static str, subtitle: Option<&'static str>, setting| {
        let entity = app.entity.clone();
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(rpx(8.))
            .min_h(rpx(32.))
            .px(rpx(8.))
            .py(rpx(6.))
            .rounded(rpx(4.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::selected())))
            .without_text_selection()
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| match setting {
                        Some(setting) => app.set_inbox_channel(guild_id, channel_id, setting, cx),
                        None => {
                            app.set_inbox_menu(None, cx);
                            app.set_guild_mute(guild_id, Some(-1), cx);
                        }
                    })
                    .ok();
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::strong()))
                            .child(title),
                    )
                    .children(subtitle.map(|subtitle| {
                        div()
                            .text_size(rpx(12.))
                            .line_height(rpx(16.))
                            .text_color(rgb(theme::muted()))
                            .child(subtitle)
                    })),
            )
    };
    let radio = |id: &'static str, choice: NotificationLevel, subtitle| {
        let selected = level == choice;
        row(
            id,
            level_name(choice),
            subtitle,
            Some(ChannelSetting::Notifications(choice)),
        )
        .child(theme::icon(
            if selected { "radio-fill" } else { "radio-ring" },
            18.,
            if selected {
                theme::BLURPLE
            } else {
                theme::muted()
            },
        ))
    };
    let divider = || {
        div()
            .h(rpx(1.))
            .my(rpx(4.))
            .mx(rpx(4.))
            .bg(rgba(theme::border()))
    };
    let close = app.entity.clone();
    let menu = div()
        .id("inbox-notification-menu")
        .flex()
        .flex_col()
        .w(rpx(220.))
        .p(rpx(6.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border_bright()))
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(move |_, _, cx| {
            close
                .update(cx, |app, cx| app.set_inbox_menu(None, cx))
                .ok();
        })
        .child(row(
            "inbox-mute-channel",
            "Mute Channel",
            None,
            Some(ChannelSetting::Mute(Some(-1))),
        ))
        .child(row("inbox-mute-server", "Mute Server", None, None))
        .child(divider())
        .child(radio(
            "inbox-level-inherit",
            NotificationLevel::Inherit,
            Some(level_name(guild_level)),
        ))
        .child(radio("inbox-level-all", NotificationLevel::All, None))
        .child(radio(
            "inbox-level-mentions",
            NotificationLevel::Mentions,
            None,
        ))
        .child(radio(
            "inbox-level-nothing",
            NotificationLevel::Nothing,
            None,
        ));
    let margin = rpx(8.).to_pixels(window.rem_size());
    deferred(
        anchored()
            .anchor(Anchor::TopRight)
            .snap_to_window_with_margin(Edges::all(margin))
            .child(menu),
    )
    .with_priority(3)
    .into_any_element()
}
