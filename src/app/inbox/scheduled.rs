use chrono::{DateTime, Local};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_tokio::Tokio;

use super::*;
use crate::discord::model;

const LATEST: chrono::Duration = chrono::Duration::days(8);

#[derive(Default)]
pub(in crate::app) struct Scheduled {
    loading: bool,
    failed: bool,
    loaded: bool,
    items: Vec<Item>,
}

struct Item {
    id: u64,
    channel_id: u64,
    send_at: Option<DateTime<Local>>,
    state: u8,
    content: String,
    flags: u64,
    preview: Option<Message>,
}

pub(crate) struct ScheduleEdit {
    id: u64,
    kind: EditKind,
    error: Option<SharedString>,
    _events: Vec<Subscription>,
}

enum EditKind {
    Time {
        date: Entity<InputState>,
        time: Entity<InputState>,
    },
    Content {
        input: Entity<InputState>,
        flags: u64,
    },
}

#[derive(Clone, Copy)]
enum Action {
    SendNow,
    Cancel,
}

impl DiscordApp {
    pub(super) fn load_scheduled(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(inbox) = &mut self.inbox else {
            return;
        };
        if inbox.scheduled.loading {
            return;
        }
        inbox.scheduled.loading = true;
        inbox.scheduled.failed = false;
        let request = Tokio::spawn(cx, async move { session.scheduled_messages().await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let items = match result {
                    Ok(Ok(scheduled)) => Some(
                        scheduled
                            .into_iter()
                            .map(|scheduled| app.scheduled_item(scheduled))
                            .collect::<Vec<_>>(),
                    ),
                    Ok(Err(error)) => {
                        log::warn!("Could not load scheduled messages: {error}");
                        None
                    }
                    Err(error) => {
                        log::warn!("Scheduled messages task failed: {error}");
                        None
                    }
                };
                let Some(inbox) = &mut app.inbox else {
                    return;
                };
                let scheduled = &mut inbox.scheduled;
                scheduled.loading = false;
                scheduled.loaded = true;
                match items {
                    Some(mut items) => {
                        items.sort_by_key(|item| item.send_at);
                        scheduled.items = items;
                    }
                    None => scheduled.failed = true,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn scheduled_item(&mut self, scheduled: model::ScheduledMessage) -> Item {
        let send_at = scheduled
            .send_at_timestamp
            .as_deref()
            .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
            .map(|time| time.with_timezone(&Local));
        let preview = scheduled.message_preview.map(|mut preview| {
            if let Some(time) = &scheduled.send_at_timestamp {
                preview.timestamp.clone_from(time);
            }
            preview
        });
        let preview = preview.and_then(|preview| {
            self.store
                .inbox_messages(std::slice::from_ref(&preview))
                .pop()
        });
        Item {
            id: scheduled.scheduled_message_id,
            channel_id: scheduled.create_args.channel_id,
            send_at,
            state: scheduled.state,
            content: scheduled.create_args.content,
            flags: scheduled.create_args.flags,
            preview,
        }
    }

    fn scheduled_action(&mut self, id: u64, action: Action, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if let Some(inbox) = &mut self.inbox {
            inbox.scheduled.items.retain(|item| item.id != id);
        }
        cx.notify();
        let request = Tokio::spawn(cx, async move {
            match action {
                Action::SendNow => session.send_scheduled_now(id).await,
                Action::Cancel => session.cancel_scheduled(id).await,
            }
        });
        cx.spawn(async move |this, cx| {
            let failed = match request.await {
                Ok(Ok(())) => false,
                Ok(Err(error)) => {
                    log::warn!("Could not update scheduled message {id}: {error}");
                    true
                }
                Err(error) => {
                    log::warn!("Scheduled message task failed: {error}");
                    true
                }
            };
            if failed {
                this.update(cx, |app, cx| app.load_scheduled(cx)).ok();
            }
        })
        .detach();
    }

    fn open_schedule_edit(
        &mut self,
        id: u64,
        content: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self
            .inbox
            .as_ref()
            .and_then(|inbox| inbox.scheduled.items.iter().find(|item| item.id == id))
        else {
            return;
        };
        let submit = |app: &mut Self,
                      _: &Entity<InputState>,
                      event: &InputEvent,
                      window: &mut Window,
                      cx: &mut Context<Self>| {
            if let InputEvent::PressEnter { .. } = event {
                app.submit_schedule_edit(window, cx);
            }
        };
        let (kind, events) = if content {
            let text = item.content.clone();
            let flags = item.flags;
            let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
            input.update(cx, |input, cx| input.focus(window, cx));
            let events = vec![cx.subscribe_in(&input, window, submit)];
            (EditKind::Content { input, flags }, events)
        } else {
            let start = item.send_at.unwrap_or_else(Local::now);
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
            date.update(cx, |input, cx| input.focus(window, cx));
            let events = vec![
                cx.subscribe_in(&date, window, submit),
                cx.subscribe_in(&time, window, submit),
            ];
            (EditKind::Time { date, time }, events)
        };
        self.schedule_edit = Some(ScheduleEdit {
            id,
            kind,
            error: None,
            _events: events,
        });
        cx.notify();
    }

    pub(in crate::app) fn close_schedule_edit(&mut self, cx: &mut Context<Self>) -> bool {
        if self.schedule_edit.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn submit_schedule_edit(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(edit) = &mut self.schedule_edit else {
            return;
        };
        let (send_at, content) = match &edit.kind {
            EditKind::Time { date, time } => {
                let due =
                    messages::saved::parse_custom(&date.read(cx).value(), &time.read(cx).value());
                let now = Local::now();
                match due {
                    Some(due) if due > now && due <= now + LATEST => {
                        (Some(messages::saved::wire_time(due)), None)
                    }
                    Some(due) if due > now => {
                        edit.error = Some("Pick a time within the next 8 days.".into());
                        cx.notify();
                        return;
                    }
                    _ => {
                        edit.error = Some("Pick a date and time in the future.".into());
                        cx.notify();
                        return;
                    }
                }
            }
            EditKind::Content { input, flags } => {
                let text = input.read(cx).value().trim().to_owned();
                if text.is_empty() {
                    edit.error = Some("The message can't be empty.".into());
                    cx.notify();
                    return;
                }
                (None, Some((text, *flags)))
            }
        };
        let id = edit.id;
        self.schedule_edit = None;
        cx.notify();
        let request = Tokio::spawn(cx, async move {
            session.update_scheduled(id, send_at, content).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let updated = match result {
                    Ok(Ok(updated)) => app.scheduled_item(updated),
                    Ok(Err(error)) => {
                        log::warn!("Could not update scheduled message {id}: {error}");
                        return;
                    }
                    Err(error) => {
                        log::warn!("Scheduled message task failed: {error}");
                        return;
                    }
                };
                if let Some(inbox) = &mut app.inbox
                    && let Some(item) = inbox.scheduled.items.iter_mut().find(|item| item.id == id)
                {
                    *item = updated;
                    inbox.scheduled.items.sort_by_key(|item| item.send_at);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn status(item: &Item, cx: &App) -> AnyElement {
    if item.state != model::ScheduledMessage::SCHEDULED {
        return card_status(
            theme::icon("circle-alert-filled", 16., theme::RED),
            "This message couldn't be sent.".to_owned(),
        );
    }
    let when = item.send_at.map_or_else(String::new, |time| {
        let clock = if crate::settings::get(cx).clock_24h {
            "%H:%M"
        } else {
            "%-I:%M %p"
        };
        time.format(&format!("%B %-d, %Y at {clock}")).to_string()
    });
    card_status(
        theme::icon("clock", 16., theme::muted()),
        format!("Message scheduled for {when}"),
    )
}

fn scheduled_card(app: &DiscordApp, item: &Item, cx: &App) -> AnyElement {
    let id = item.id;
    let guild_id = app.store.channel_guild_id(item.channel_id);
    let send = app.entity.clone();
    let edit = app.entity.clone();
    let time = app.entity.clone();
    let cancel = app.entity.clone();
    let button = |name: &'static str, icon: &'static str, tip: &'static str| {
        card_button(ElementId::NamedInteger(name.into(), id), icon, tip)
    };
    let buttons = div()
        .flex()
        .flex_none()
        .gap(rpx(8.))
        .child(
            button("inbox-scheduled-send", "send", "Send now").on_click(move |_, _, cx| {
                send.update(cx, |app, cx| app.scheduled_action(id, Action::SendNow, cx))
                    .ok();
            }),
        )
        .child(
            button("inbox-scheduled-edit", "pencil", "Edit message").on_click(
                move |_, window, cx| {
                    edit.update(cx, |app, cx| app.open_schedule_edit(id, true, window, cx))
                        .ok();
                },
            ),
        )
        .child(
            button("inbox-scheduled-time", "clock", "Edit schedule time").on_click(
                move |_, window, cx| {
                    time.update(cx, |app, cx| app.open_schedule_edit(id, false, window, cx))
                        .ok();
                },
            ),
        )
        .child(
            button("inbox-scheduled-cancel", "x", "Cancel scheduled message").on_click(
                move |_, _, cx| {
                    cancel
                        .update(cx, |app, cx| app.scheduled_action(id, Action::Cancel, cx))
                        .ok();
                },
            ),
        );
    let header = card_header(app, guild_id, item.channel_id, None, buttons, cx)
        .border_b_1()
        .border_color(rgba(theme::outline()));
    let body = match &item.preview {
        Some(message) => message_body(app, message, cx),
        None => div()
            .px(rpx(16.))
            .py(rpx(12.))
            .text_size(rpx(14.))
            .text_color(rgb(theme::text()))
            .child(item.content.clone())
            .into_any_element(),
    };
    card(Some(status(item, cx)), header, body)
}

fn empty() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(rpx(8.))
        .py(rpx(48.))
        .px(rpx(16.))
        .text_center()
        .child(theme::icon("send", 48., theme::muted()))
        .child(
            div()
                .mt(rpx(16.))
                .text_size(rpx(24.))
                .line_height(rpx(30.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Send it at the perfect moment"),
        )
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .text_color(rgb(theme::text()))
                .child(
                    "Messages you schedule in Discord wait here until they're sent. You can \
                     send them early, edit them, or cancel them.",
                ),
        )
        .into_any_element()
}

pub(super) fn list(app: &DiscordApp, inbox: &Inbox, cx: &App) -> Vec<AnyElement> {
    let scheduled = &inbox.scheduled;
    let mut items: Vec<_> = scheduled
        .items
        .iter()
        .map(|item| scheduled_card(app, item, cx))
        .collect();
    if items.is_empty() {
        items.push(if scheduled.loading || !scheduled.loaded {
            note("Loading\u{2026}")
        } else if scheduled.failed {
            note("Could not load scheduled messages.")
        } else {
            empty()
        });
    }
    items
}

pub(crate) fn schedule_edit_dialog(app: &DiscordApp) -> Option<AnyElement> {
    let edit = app.schedule_edit.as_ref()?;
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
    let (title, fields) = match &edit.kind {
        EditKind::Time { date, time } => (
            "Edit Schedule Time",
            div()
                .flex()
                .gap(rpx(12.))
                .child(field("DATE", date))
                .child(field("TIME", time)),
        ),
        EditKind::Content { input, .. } => {
            ("Edit Message", div().flex().child(field("MESSAGE", input)))
        }
    };
    Some(
        div()
            .id("schedule-edit-backdrop")
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
                    .update(cx, |app, cx| app.close_schedule_edit(cx))
                    .ok();
            })
            .child(theme::selection_guard())
            .child(
                div()
                    .id("schedule-edit")
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
                            .child(title),
                    )
                    .child(fields)
                    .children(edit.error.clone().map(|error| {
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
                                    "schedule-edit-cancel",
                                    "Cancel",
                                    theme::button_bg(),
                                    theme::button_subtle_hover(),
                                    rgb(theme::strong()).into(),
                                )
                                .on_click(move |_, _, cx| {
                                    cancel
                                        .update(cx, |app, cx| app.close_schedule_edit(cx))
                                        .ok();
                                }),
                            )
                            .child(
                                button(
                                    "schedule-edit-save",
                                    "Save",
                                    theme::BLURPLE,
                                    theme::BLURPLE_ACTIVE,
                                    white(),
                                )
                                .on_click(move |_, window, cx| {
                                    submit
                                        .update(cx, |app, cx| app.submit_schedule_edit(window, cx))
                                        .ok();
                                }),
                            ),
                    ),
            )
            .into_any_element(),
    )
}
