use crate::app::menu::PopupButton as _;
use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone, Utc};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_tokio::Tokio;

use super::super::inbox::{self, InboxTab, card_button};
use super::super::*;
use crate::store::SavedEntry;
use crate::theme::WithoutTextSelection as _;

const PRESET_WIDTH: f32 = 188.;

pub(crate) struct CustomReminder {
    channel_id: u64,
    message_id: u64,
    date: Entity<InputState>,
    time: Entity<InputState>,
    error: Option<SharedString>,
    _events: [Subscription; 2],
}

pub(crate) fn reminder_presets(now: DateTime<Local>) -> Vec<(String, DateTime<Local>)> {
    let at_nine = |date: NaiveDate| {
        let nine = NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default();
        Local
            .from_local_datetime(&date.and_time(nine))
            .earliest()
            .unwrap_or(now)
    };
    let today = now.date_naive();
    let until_monday = match 7 - i64::from(today.weekday().num_days_from_monday()) {
        1 => 8,
        days => days,
    };
    let monday = today + chrono::Duration::days(until_monday);
    vec![
        ("in 30m".to_owned(), now + chrono::Duration::minutes(30)),
        ("in 1h".to_owned(), now + chrono::Duration::hours(1)),
        ("in 4h".to_owned(), now + chrono::Duration::hours(4)),
        (
            "Tomorrow at 9:00am".to_owned(),
            at_nine(today + chrono::Duration::days(1)),
        ),
        (
            format!("{} at 9:00am", monday.format("%A")),
            at_nine(monday),
        ),
    ]
}

pub(crate) fn wire_time(due: DateTime<Local>) -> String {
    due.with_timezone(&Utc)
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

pub(crate) fn parse_custom(date: &str, time: &str) -> Option<DateTime<Local>> {
    let date = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").ok()?;
    let time = NaiveTime::parse_from_str(time.trim(), "%H:%M").ok()?;
    Local.from_local_datetime(&date.and_time(time)).earliest()
}

fn due_label(due_ms: i64, now_ms: i64) -> String {
    let minutes = (due_ms - now_ms).abs() / 60_000;
    let span = match minutes {
        0 => "less than a minute".to_owned(),
        1 => "1 minute".to_owned(),
        2..=59 => format!("{minutes} minutes"),
        60..=119 => "1 hour".to_owned(),
        120..=2879 => format!("{} hours", minutes / 60),
        _ => format!("{} days", minutes / 1440),
    };
    if due_ms <= now_ms {
        format!("Due {span} ago")
    } else {
        format!("Due in {span}")
    }
}

pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

impl DiscordApp {
    pub(crate) fn load_saved_messages(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let request = Tokio::spawn(cx, async move { session.saved_messages().await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| match result {
                Ok(Ok(saved)) => {
                    app.store.set_saved_messages(saved);
                    let now = now_ms();
                    app.notified_reminders.extend(
                        app.store
                            .reminders()
                            .iter()
                            .filter(|entry| entry.due_ms.is_some_and(|due| due <= now))
                            .map(|entry| entry.message_id),
                    );
                    app.arm_reminder_timer(cx);
                    cx.notify();
                }
                Ok(Err(error)) => log::warn!("Could not load saved messages: {error}"),
                Err(error) => log::warn!("Saved messages task failed: {error}"),
            })
            .ok();
        })
        .detach();
    }

    pub(in crate::app) fn toggle_bookmark(
        &mut self,
        channel_id: u64,
        message_id: u64,
        cx: &mut Context<Self>,
    ) {
        if self.store.is_bookmarked(message_id) {
            self.remove_saved(channel_id, message_id, cx);
        } else {
            self.save_message(channel_id, message_id, None, cx);
        }
    }

    pub(in crate::app) fn set_reminder(
        &mut self,
        channel_id: u64,
        message_id: u64,
        due: DateTime<Local>,
        cx: &mut Context<Self>,
    ) {
        self.notified_reminders.remove(&message_id);
        self.save_message(channel_id, message_id, Some(wire_time(due)), cx);
    }

    fn save_message(
        &mut self,
        channel_id: u64,
        message_id: u64,
        due_at: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let request = Tokio::spawn(cx, async move {
            session.save_message(channel_id, message_id, due_at).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| match result {
                Ok(Ok(saved)) => {
                    app.store.message_saved(saved);
                    app.arm_reminder_timer(cx);
                    cx.notify();
                }
                Ok(Err(error)) => log::warn!("Could not save message: {error}"),
                Err(error) => log::warn!("Save message task failed: {error}"),
            })
            .ok();
        })
        .detach();
    }

    pub(in crate::app) fn remove_saved(
        &mut self,
        channel_id: u64,
        message_id: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.store.message_unsaved(message_id);
        self.arm_reminder_timer(cx);
        let request = Tokio::spawn(cx, async move {
            session.unsave_message(channel_id, message_id).await
        });
        cx.spawn(async move |this, cx| {
            let failed = match request.await {
                Ok(Ok(_)) => return,
                Ok(Err(error)) => error.to_string(),
                Err(error) => error.to_string(),
            };
            log::warn!("Could not remove saved message: {failed}");
            this.update(cx, |app, cx| {
                app.load_saved_messages(cx);
                if app.store.bookmark_list().is_some() {
                    app.load_bookmarks(false, cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn saved_changed(&mut self, cx: &mut Context<Self>) {
        self.arm_reminder_timer(cx);
    }

    fn arm_reminder_timer(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        let Some(due) = self.store.next_reminder_due(now) else {
            self.reminder_timer = None;
            return;
        };
        let wait = Duration::from_millis((due - now).max(0) as u64 + 250);
        self.reminder_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |app, cx| app.reminders_due(cx)).ok();
        }));
    }

    fn reminders_due(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        let due: Vec<_> = self
            .store
            .reminders()
            .iter()
            .filter(|entry| {
                entry.due_ms.is_some_and(|due| due <= now)
                    && !self.notified_reminders.contains(&entry.message_id)
            })
            .map(|entry| {
                let body = entry.message.as_ref().map_or_else(
                    || "You asked to be reminded about a message.".to_owned(),
                    |message| {
                        let author = self
                            .store
                            .user(message.author_id)
                            .map_or("Someone", |user| user.name.as_str());
                        format!("{author}: {}", message.content)
                    },
                );
                (entry.channel_id, entry.message_id, body)
            })
            .collect();
        for (channel_id, message_id, body) in due {
            self.notified_reminders.insert(message_id);
            if crate::settings::notifications()
                && !crate::settings::streamer().mutes_notifications()
            {
                cx.background_executor()
                    .spawn(async move {
                        crate::notify::show("Reminder", &body, None, channel_id, message_id);
                    })
                    .detach();
            }
        }
        self.arm_reminder_timer(cx);
        cx.notify();
    }

    pub(crate) fn load_bookmarks(&mut self, more: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(inbox) = &mut self.inbox else {
            return;
        };
        if inbox.loading {
            return;
        }
        inbox.loading = true;
        inbox.failed = false;
        let before = if more {
            let Some(cursor) = self.store.bookmarks_cursor() else {
                if let Some(inbox) = &mut self.inbox {
                    inbox.loading = false;
                }
                return;
            };
            Some(cursor.to_owned())
        } else {
            None
        };
        let request = Tokio::spawn(cx, async move { session.bookmarks(before).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let page = match result {
                    Ok(Ok(page)) => Some(page),
                    Ok(Err(error)) => {
                        log::warn!("Could not load bookmarks: {error}");
                        None
                    }
                    Err(error) => {
                        log::warn!("Bookmarks task failed: {error}");
                        None
                    }
                };
                let failed = page.is_none();
                match page {
                    Some(page) if more => app.store.add_bookmarks(page),
                    Some(page) => app.store.set_bookmarks(page),
                    None => {}
                }
                if let Some(inbox) = &mut app.inbox {
                    inbox.loading = false;
                    inbox.failed = failed;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn toggle_inbox_presets(&mut self, message_id: u64, cx: &mut Context<Self>) {
        if let Some(inbox) = &mut self.inbox {
            inbox.presets_for = (inbox.presets_for != Some(message_id)).then_some(message_id);
            cx.notify();
        }
    }

    pub(crate) fn jump_to_saved(
        &mut self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_inbox(cx);
        self.open_message(guild_id, channel_id, message_id, window, cx);
    }

    pub(in crate::app) fn open_custom_reminder(
        &mut self,
        channel_id: u64,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = self
            .store
            .reminder(message_id)
            .and_then(|entry| entry.due_ms)
            .and_then(|due| Local.timestamp_millis_opt(due).single())
            .unwrap_or_else(|| reminder_presets(Local::now())[3].1);
        let date = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("YYYY-MM-DD")
                .default_value(start.format("%Y-%m-%d").to_string())
        });
        let time = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("HH:MM")
                .default_value(start.format("%H:%M").to_string())
        });
        let submit = |app: &mut Self,
                      _: &Entity<InputState>,
                      event: &InputEvent,
                      window: &mut Window,
                      cx: &mut Context<Self>| {
            if let InputEvent::PressEnter { .. } = event {
                app.submit_custom_reminder(window, cx);
            }
        };
        let events = [
            cx.subscribe_in(&date, window, submit),
            cx.subscribe_in(&time, window, submit),
        ];
        date.update(cx, |input, cx| input.focus(window, cx));
        self.custom_reminder = Some(CustomReminder {
            channel_id,
            message_id,
            date,
            time,
            error: None,
            _events: events,
        });
        cx.notify();
    }

    pub(in crate::app) fn close_custom_reminder(&mut self, cx: &mut Context<Self>) -> bool {
        if self.custom_reminder.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn submit_custom_reminder(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = &mut self.custom_reminder else {
            return;
        };
        let due = parse_custom(&dialog.date.read(cx).value(), &dialog.time.read(cx).value());
        let Some(due) = due.filter(|due| *due > Local::now()) else {
            dialog.error = Some("Pick a date and time in the future.".into());
            cx.notify();
            return;
        };
        let (channel_id, message_id) = (dialog.channel_id, dialog.message_id);
        self.custom_reminder = None;
        self.set_reminder(channel_id, message_id, due, cx);
        cx.notify();
    }
}

pub(crate) fn preset_rows(
    app: &DiscordApp,
    key: &'static str,
    channel_id: u64,
    message_id: u64,
) -> Vec<AnyElement> {
    let row = |id: ElementId| {
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .h(rpx(34.))
            .px(rpx(8.))
            .rounded(rpx(4.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::selected())))
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::strong()))
            .without_text_selection()
    };
    let mut rows = vec![
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(rpx(36.))
            .px(rpx(8.))
            .text_size(rpx(14.))
            .line_height(rpx(20.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::faint()))
            .child("Remind Me")
            .into_any_element(),
    ];
    for (index, (label, due)) in reminder_presets(Local::now()).into_iter().enumerate() {
        let entity = app.entity.clone();
        rows.push(
            row(ElementId::NamedInteger(key.into(), index as u64))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    entity
                        .update(cx, |app, cx| {
                            app.close_message_menu(cx);
                            if let Some(inbox) = &mut app.inbox {
                                inbox.presets_for = None;
                            }
                            app.set_reminder(channel_id, message_id, due, cx);
                        })
                        .ok();
                })
                .child(label)
                .into_any_element(),
        );
    }
    let entity = app.entity.clone();
    rows.push(
        row(ElementId::NamedInteger(key.into(), 99))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| {
                        app.close_message_menu(cx);
                        if let Some(inbox) = &mut app.inbox {
                            inbox.presets_for = None;
                        }
                        app.open_custom_reminder(channel_id, message_id, window, cx);
                    })
                    .ok();
            })
            .child("Custom...")
            .into_any_element(),
    );
    rows
}

pub(crate) fn preset_panel(rows: Vec<AnyElement>) -> Stateful<Div> {
    div()
        .id("reminder-presets")
        .flex()
        .flex_col()
        .w(rpx(PRESET_WIDTH))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::outline()))
        .shadow_lg()
        .occlude()
        .children(rows)
}

pub(crate) fn overdue_pill(app: &DiscordApp) -> Option<impl IntoElement + use<>> {
    let overdue = app.store.overdue_reminders(now_ms());
    if overdue == 0 {
        return None;
    }
    let entity = app.entity.clone();
    let label = match overdue {
        1 => "1 overdue reminder".to_owned(),
        count => format!("{count} overdue reminders"),
    };
    Some(
        div()
            .id("overdue-reminders")
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(24.))
            .px(rpx(8.))
            .rounded(rpx(8.))
            .cursor_pointer()
            .bg(rgba(theme::control_bg()))
            .border_1()
            .border_color(rgba(theme::border_soft()))
            .hover(|style| style.bg(rgba(theme::control_selected())))
            .text_size(rpx(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::strong()))
            .occlude()
            .on_popup_click(
                crate::app::menu::shown(app, |app| app.inbox.is_some()),
                move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| {
                            app.toggle_inbox(Some(InboxTab::Reminders), cx)
                        })
                        .ok();
                },
            )
            .child(theme::icon("clock", 14., theme::strong()))
            .child(label),
    )
}

pub(crate) fn saved_list(
    app: &DiscordApp,
    inbox: &super::super::inbox::Inbox,
    now: i64,
    cx: &App,
) -> Vec<AnyElement> {
    let entries: &[SavedEntry] = if inbox.tab == InboxTab::Reminders {
        app.store.reminders()
    } else {
        app.store.bookmark_list().unwrap_or_default()
    };
    let mut cards: Vec<_> = entries
        .iter()
        .map(|entry| {
            saved_card(
                app,
                entry,
                inbox.tab,
                inbox.presets_for == Some(entry.message_id),
                now,
                cx,
            )
        })
        .collect();
    let bookmarks = inbox.tab == InboxTab::Bookmarks;
    if cards.is_empty() {
        cards.push(super::super::inbox::note(if bookmarks && inbox.loading {
            "Loading\u{2026}"
        } else if bookmarks && inbox.failed {
            "Could not load bookmarks."
        } else if bookmarks {
            "You don't have any bookmarks yet."
        } else {
            "You don't have any reminders."
        }));
    }
    cards
}

pub(super) fn saved_card(
    app: &DiscordApp,
    entry: &SavedEntry,
    tab: InboxTab,
    presets_open: bool,
    now: i64,
    cx: &App,
) -> AnyElement {
    let (channel_id, message_id, guild_id) = (entry.channel_id, entry.message_id, entry.guild_id);
    let overdue = entry.due_ms.is_some_and(|due| due <= now);
    let status = (tab == InboxTab::Reminders).then(|| {
        let icon = if overdue {
            theme::icon("circle-alert-filled", 16., theme::RED)
        } else {
            theme::icon("clock", 16., theme::muted())
        };
        inbox::card_status(icon, due_label(entry.due_ms.unwrap_or(now), now))
    });
    let jump = app.entity.clone();
    let remove = app.entity.clone();
    let presets = app.entity.clone();
    let mut buttons = div().flex().flex_none().gap(rpx(8.));
    if tab == InboxTab::Reminders {
        let (icon, tip) = if overdue {
            ("bell-snooze", "Snooze Reminder")
        } else {
            ("pencil", "Edit Reminder")
        };
        buttons = buttons.child(
            card_button(
                ElementId::NamedInteger("saved-presets".into(), message_id),
                icon,
                tip,
            )
            .on_popup_click(
                crate::app::menu::shown(app, move |app| {
                    app.inbox
                        .as_ref()
                        .is_some_and(|inbox| inbox.presets_for == Some(message_id))
                }),
                move |_, _, cx| {
                    presets
                        .update(cx, |app, cx| app.toggle_inbox_presets(message_id, cx))
                        .ok();
                },
            ),
        );
    }
    let remove_tip = if tab == InboxTab::Reminders {
        "Remove Reminder"
    } else {
        "Remove Bookmark"
    };
    buttons = buttons
        .child(
            card_button(
                ElementId::NamedInteger("saved-jump".into(), message_id),
                "message-jump",
                "Jump to Message",
            )
            .on_click(move |_, window, cx| {
                jump.update(cx, |app, cx| {
                    app.jump_to_saved(guild_id, channel_id, message_id, window, cx)
                })
                .ok();
            }),
        )
        .child(
            card_button(
                ElementId::NamedInteger("saved-remove".into(), message_id),
                "x",
                remove_tip,
            )
            .on_click(move |_, _, cx| {
                remove
                    .update(cx, |app, cx| app.remove_saved(channel_id, message_id, cx))
                    .ok();
            }),
        );
    let header = inbox::card_header(app, guild_id, channel_id, None, buttons, cx).when(
        presets_open,
        |this| {
            let panel = preset_panel(preset_rows(app, "saved-preset", channel_id, message_id));
            this.child(
                div()
                    .absolute()
                    .top(rpx(52.))
                    .right(rpx(12.))
                    .size_0()
                    .child(
                        deferred(
                            anchored()
                                .anchor(Anchor::TopRight)
                                .snap_to_window_with_margin(px(8.))
                                .child(panel),
                        )
                        .with_priority(3),
                    ),
            )
        },
    );
    let body = match &entry.message {
        Some(message) => inbox::message_body(app, message, cx),
        None => div()
            .px(rpx(16.))
            .py(rpx(12.))
            .text_size(rpx(14.))
            .text_color(rgb(theme::muted()))
            .child("This message isn't available.")
            .into_any_element(),
    };
    inbox::card(status, header, body)
}

pub(in crate::app) fn custom_reminder_dialog(app: &DiscordApp) -> Option<AnyElement> {
    let dialog = app.custom_reminder.as_ref()?;
    let backdrop = app.entity.clone();
    let cancel = app.entity.clone();
    let submit = app.entity.clone();
    let field = |label: &'static str, input: &Entity<InputState>| {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .gap(rpx(8.))
            .child(
                div()
                    .text_size(rpx(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::muted()))
                    .child(label),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(rpx(40.))
                    .px(rpx(12.))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::bg()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(input)
                                .appearance(false)
                                .px_0()
                                .text_size(rpx(16.)),
                        ),
                    ),
            )
    };
    let button = |id: &'static str, text: &'static str, bg: u32, hover: u32, color: Hsla| {
        div()
            .id(id)
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .h(rpx(40.))
            .rounded(rpx(8.))
            .cursor_pointer()
            .bg(rgb(bg))
            .hover(move |style| style.bg(rgb(hover)))
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(color)
            .child(text)
    };
    Some(
        div()
            .id("custom-reminder-backdrop")
            .absolute()
            .inset_0()
            .top(rpx(titlebar::HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .bg(rgba(theme::SCRIM))
            .on_click(move |_, _, cx| {
                backdrop
                    .update(cx, |app, cx| app.close_custom_reminder(cx))
                    .ok();
            })
            .child(
                div()
                    .id("custom-reminder")
                    .flex()
                    .flex_col()
                    .gap(rpx(16.))
                    .w(rpx(440.))
                    .max_w_full()
                    .p(rpx(24.))
                    .rounded(rpx(12.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .occlude()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .text_size(rpx(20.))
                            .line_height(rpx(24.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::strong()))
                            .child("Create Reminder"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(rpx(12.))
                            .child(field("DATE", &dialog.date))
                            .child(field("TIME", &dialog.time)),
                    )
                    .children(dialog.error.clone().map(|error| {
                        div()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::RED))
                            .child(error)
                    }))
                    .child(
                        div()
                            .flex()
                            .gap(rpx(8.))
                            .child(
                                button(
                                    "custom-reminder-cancel",
                                    "Cancel",
                                    theme::button_bg(),
                                    theme::button_subtle_hover(),
                                    rgb(theme::strong()).into(),
                                )
                                .on_click(move |_, _, cx| {
                                    cancel
                                        .update(cx, |app, cx| app.close_custom_reminder(cx))
                                        .ok();
                                }),
                            )
                            .child(
                                button(
                                    "custom-reminder-save",
                                    "Save",
                                    theme::BLURPLE,
                                    theme::BLURPLE_ACTIVE,
                                    white(),
                                )
                                .on_click(move |_, window, cx| {
                                    submit
                                        .update(cx, |app, cx| {
                                            app.submit_custom_reminder(window, cx)
                                        })
                                        .ok();
                                }),
                            ),
                    ),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{due_label, parse_custom, reminder_presets, wire_time};
    use chrono::{Local, NaiveDate, TimeZone, Utc};

    #[test]
    fn presets_name_the_next_monday_and_never_tomorrow() {
        let saturday = Local.with_ymd_and_hms(2026, 11, 7, 15, 0, 0).unwrap();
        let presets = reminder_presets(saturday);
        let labels: Vec<_> = presets.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "in 30m",
                "in 1h",
                "in 4h",
                "Tomorrow at 9:00am",
                "Monday at 9:00am"
            ]
        );
        assert_eq!(
            presets[3].1.date_naive(),
            NaiveDate::from_ymd_opt(2026, 11, 8).unwrap()
        );
        assert_eq!(
            presets[4].1.date_naive(),
            NaiveDate::from_ymd_opt(2026, 11, 9).unwrap()
        );

        let sunday = Local.with_ymd_and_hms(2026, 11, 8, 15, 0, 0).unwrap();
        assert_eq!(
            reminder_presets(sunday)[4].1.date_naive(),
            NaiveDate::from_ymd_opt(2026, 11, 16).unwrap()
        );
    }

    #[test]
    fn due_labels_and_wire_times() {
        let now = 1_000_000_000_000;
        assert_eq!(due_label(now + 30 * 60_000, now), "Due in 30 minutes");
        assert_eq!(due_label(now - 14 * 3_600_000, now), "Due 14 hours ago");
        assert_eq!(due_label(now + 3 * 86_400_000, now), "Due in 3 days");
        let due = Utc.with_ymd_and_hms(2026, 11, 7, 11, 22, 20).unwrap();
        assert_eq!(
            wire_time(due.with_timezone(&Local)),
            "2026-11-07T11:22:20.000Z"
        );
        assert!(parse_custom("2026-11-07", "9:30").is_some());
        assert!(parse_custom("11/07/2026", "9:30").is_none());
    }
}
