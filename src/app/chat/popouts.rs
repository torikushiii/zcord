use std::time::{Duration, Instant};

use gpui_component::input::{Input, InputEvent, InputState};
use gpui_tokio::Tokio;

use super::message::render_message;
use super::*;
use crate::app::titlebar;
use crate::discord::model;
use crate::store::ThreadRow;

const PIN_WIDTH: f32 = 420.;
const THREAD_MIN_WIDTH: f32 = 480.;
const THREAD_MAX_WIDTH: f32 = 600.;
const THREAD_WIDTH_SHARE: f32 = 0.35;
const MAX_PREVIEW_AVATARS: usize = 5;
/// The press that closes a popout also clicks its header button, which must not reopen it.
const REOPEN_GUARD: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum PopoutKind {
    Pins,
    Threads,
}

pub(in crate::app) struct Pins {
    channel_id: u64,
    messages: Vec<Message>,
    before: Option<String>,
    loading: bool,
    failed: bool,
}

pub(in crate::app) struct Threads {
    parent_id: u64,
    older: Vec<ThreadRow>,
    has_more: bool,
    loading: bool,
    results: Option<Vec<ThreadRow>>,
    searching: bool,
    search: Entity<InputState>,
    _search_events: Subscription,
}

pub(in crate::app) enum ChannelPopout {
    Pins(Pins),
    Threads(Threads),
}

impl ChannelPopout {
    fn kind(&self) -> PopoutKind {
        match self {
            Self::Pins(_) => PopoutKind::Pins,
            Self::Threads(_) => PopoutKind::Threads,
        }
    }
}

impl DiscordApp {
    pub(in crate::app) fn toggle_channel_popout(
        &mut self,
        kind: PopoutKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .popout_closed
            .is_some_and(|(closed, at)| closed == kind && at.elapsed() < REOPEN_GUARD)
        {
            return;
        }
        if self.channel_popout.as_ref().map(ChannelPopout::kind) == Some(kind) {
            self.close_channel_popout(cx);
            return;
        }
        match kind {
            PopoutKind::Pins => self.open_pins(cx),
            PopoutKind::Threads => self.open_threads(window, cx),
        }
    }

    pub(in crate::app) fn close_channel_popout(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(popout) = self.channel_popout.take() else {
            return false;
        };
        self.popout_closed = Some((popout.kind(), Instant::now()));
        cx.notify();
        true
    }

    fn open_pins(&mut self, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        self.channel_popout = Some(ChannelPopout::Pins(Pins {
            channel_id,
            messages: Vec::new(),
            before: None,
            loading: false,
            failed: false,
        }));
        self.load_pins(cx);
        if self.store.pins_unread(channel_id)
            && let Some(session) = self.session.clone().filter(|_| self.live)
        {
            self.store.mark_pins_read(channel_id);
            let request = Tokio::spawn(cx, async move { session.ack_pins(channel_id).await });
            cx.spawn(async move |_, _| {
                if let Ok(Err(error)) = request.await {
                    log::warn!("Could not mark pins read: {error}");
                }
            })
            .detach();
        }
        cx.notify();
    }

    fn load_pins(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(ChannelPopout::Pins(pins)) = &mut self.channel_popout else {
            return;
        };
        if pins.loading {
            return;
        }
        pins.loading = true;
        pins.failed = false;
        let channel_id = pins.channel_id;
        let before = pins.before.clone();
        let request = Tokio::spawn(cx, async move {
            session.pinned_messages(channel_id, before).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let guild_id = app.selected_guild_id;
                let Some(ChannelPopout::Pins(_)) = &app.channel_popout else {
                    return;
                };
                let page = match result {
                    Ok(Ok(page)) => Some(page),
                    Ok(Err(error)) => {
                        log::warn!("Could not load pins: {error}");
                        None
                    }
                    Err(error) => {
                        log::warn!("Pin task failed: {error}");
                        None
                    }
                };
                let before = page.as_ref().and_then(|page| {
                    page.has_more
                        .then(|| page.items.last().map(|pin| pin.pinned_at.clone()))
                        .flatten()
                });
                let failed = page.is_none();
                let messages: Vec<_> = page
                    .into_iter()
                    .flat_map(|page| page.items)
                    .map(|pin| app.store.map_listed_message(guild_id, pin.message))
                    .collect();
                if let Some(ChannelPopout::Pins(pins)) = &mut app.channel_popout
                    && pins.channel_id == channel_id
                {
                    pins.loading = false;
                    pins.failed = failed;
                    pins.before = before;
                    pins.messages.extend(messages);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(in crate::app) fn pin_message(&mut self, message_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let channel_id = self.selected_channel_id;
        let request = Tokio::spawn(cx, async move {
            session.pin_message(channel_id, message_id).await
        });
        cx.spawn(async move |_, _| {
            if let Ok(Err(error)) = request.await {
                log::warn!("Could not pin message: {error}");
            }
        })
        .detach();
    }

    fn unpin_message(&mut self, message_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(ChannelPopout::Pins(pins)) = &mut self.channel_popout else {
            return;
        };
        pins.messages.retain(|message| message.id != message_id);
        let channel_id = pins.channel_id;
        let request = Tokio::spawn(cx, async move {
            session.unpin_message(channel_id, message_id).await
        });
        cx.spawn(async move |_, _| {
            if let Ok(Err(error)) = request.await {
                log::warn!("Could not unpin message: {error}");
            }
        })
        .detach();
        cx.notify();
    }

    fn open_threads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent_id = self.thread_parent().unwrap_or(self.selected_channel_id);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search for Thread Name"));
        let events = cx.subscribe(&search, |app, input, event, cx| match event {
            InputEvent::PressEnter { .. } => app.search_threads(cx),
            InputEvent::Change if input.read(cx).value().trim().is_empty() => {
                if let Some(ChannelPopout::Threads(threads)) = &mut app.channel_popout {
                    threads.results = None;
                    cx.notify();
                }
            }
            _ => {}
        });
        self.channel_popout = Some(ChannelPopout::Threads(Threads {
            parent_id,
            older: Vec::new(),
            has_more: self.store.can_read_history(parent_id),
            loading: false,
            results: None,
            searching: false,
            search,
            _search_events: events,
        }));
        self.load_older_threads(cx);
        cx.notify();
    }

    fn load_older_threads(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(ChannelPopout::Threads(threads)) = &mut self.channel_popout else {
            return;
        };
        if threads.loading || !threads.has_more {
            return;
        }
        threads.loading = true;
        let parent_id = threads.parent_id;
        let offset = threads.older.len();
        let request = Tokio::spawn(cx, async move {
            session.archived_threads(parent_id, offset).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let (rows, has_more) = match result {
                    Ok(Ok(page)) => (app.remember_threads(page.threads), page.has_more),
                    Ok(Err(error)) => {
                        log::warn!("Could not load older threads: {error}");
                        (Vec::new(), false)
                    }
                    Err(error) => {
                        log::warn!("Thread task failed: {error}");
                        (Vec::new(), false)
                    }
                };
                if let Some(ChannelPopout::Threads(threads)) = &mut app.channel_popout
                    && threads.parent_id == parent_id
                {
                    threads.loading = false;
                    threads.has_more = has_more;
                    threads.older.extend(rows);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Listed threads must be known to the store, so a click can open them like any channel.
    fn remember_threads(&mut self, threads: Vec<model::Channel>) -> Vec<ThreadRow> {
        threads
            .into_iter()
            .map(|thread| {
                let row = self.store.thread_row(&thread);
                self.store.remember_thread(thread);
                row
            })
            .collect()
    }

    fn search_threads(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(ChannelPopout::Threads(threads)) = &mut self.channel_popout else {
            return;
        };
        let name = threads.search.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return;
        }
        threads.searching = true;
        let parent_id = threads.parent_id;
        let request = Tokio::spawn(
            cx,
            async move { session.search_threads(parent_id, &name).await },
        );
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let rows = match result {
                    Ok(Ok(page)) => app.remember_threads(page.threads),
                    Ok(Err(error)) => {
                        log::warn!("Could not search threads: {error}");
                        Vec::new()
                    }
                    Err(error) => {
                        log::warn!("Thread search task failed: {error}");
                        Vec::new()
                    }
                };
                if let Some(ChannelPopout::Threads(threads)) = &mut app.channel_popout
                    && threads.parent_id == parent_id
                {
                    threads.searching = false;
                    threads.results = Some(rows);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn open_listed_thread(
        &mut self,
        thread_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_channel_popout(cx);
        self.open_thread_pane(thread_id, window, cx);
    }
}

pub(in crate::app) fn pins_button(app: &DiscordApp) -> Stateful<Div> {
    let entity = app.entity.clone();
    let unread = app.store.pins_unread(app.selected_channel_id);
    theme::icon_button("pins", "pin", 20., theme::muted())
        .relative()
        .child(theme::hover_tooltip("pins-tip", "Pinned Messages"))
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| {
                    app.toggle_channel_popout(PopoutKind::Pins, window, cx)
                })
                .ok();
        })
        .when(unread, |this| {
            this.child(
                div()
                    .absolute()
                    .right(rpx(4.))
                    .bottom(rpx(4.))
                    .size(rpx(8.))
                    .rounded_full()
                    .bg(rgb(theme::BLURPLE)),
            )
        })
}

pub(in crate::app) fn threads_button(app: &DiscordApp) -> Stateful<Div> {
    let entity = app.entity.clone();
    theme::icon_button("threads", "messages-square", 20., theme::muted())
        .relative()
        .child(theme::hover_tooltip("threads-tip", "Threads"))
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| {
                    app.toggle_channel_popout(PopoutKind::Threads, window, cx)
                })
                .ok();
        })
}

fn frame(app: &DiscordApp, id: &'static str, width: f32) -> Stateful<Div> {
    let close = app.entity.clone();
    div()
        .id(id)
        .absolute()
        .top(rpx(titlebar::HEIGHT + HEADER_HEIGHT + 8.))
        .right(rpx(16.))
        .flex()
        .flex_col()
        .w(rpx(width))
        .h(relative(0.8))
        .rounded(rpx(8.))
        .bg(rgb(theme::surface_high()))
        .border_1()
        .border_color(rgba(theme::border_bright()))
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(move |_, _, cx| {
            close
                .update(cx, |app, cx| app.close_channel_popout(cx))
                .ok();
        })
}

fn load_more(
    id: &'static str,
    action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(rpx(36.))
        .mb(rpx(8.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(rgb(theme::elevated()))
        .hover(|style| style.bg(rgba(theme::selected())))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .on_click(action)
        .child("Load More")
}

fn note(text: &'static str) -> Div {
    div()
        .py(rpx(32.))
        .text_center()
        .text_size(rpx(14.))
        .text_color(rgb(theme::muted()))
        .child(text)
}

fn pin_card(app: &DiscordApp, message: &Message, can_unpin: bool, cx: &App) -> impl IntoElement {
    let id = message.id;
    let jump = app.entity.clone();
    let unpin = app.entity.clone();
    let action = |key: &'static str| {
        div()
            .id((key, id))
            .flex()
            .items_center()
            .justify_center()
            .h(rpx(24.))
            .px(rpx(8.))
            .rounded(rpx(6.))
            .cursor_pointer()
            .bg(rgb(theme::surface_high()))
            .hover(|style| style.bg(rgba(theme::selected())))
            .text_size(rpx(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::strong()))
    };
    div()
        .relative()
        .flex_none()
        .mb(rpx(8.))
        .py(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::outline()))
        .overflow_hidden()
        .child(render_message(app, message, false, true, cx))
        .child(
            div()
                .absolute()
                .top(rpx(8.))
                .right(rpx(8.))
                .flex()
                .gap_1()
                .child(
                    action("pin-jump")
                        .child("Jump")
                        .on_click(move |_, window, cx| {
                            jump.update(cx, |app, cx| {
                                app.close_channel_popout(cx);
                                app.jump_to_message(id, window, cx);
                            })
                            .ok();
                        }),
                )
                .when(can_unpin, |this| {
                    this.child(
                        action("pin-remove")
                            .child(theme::icon("x", 14., theme::strong()))
                            .on_click(move |_, _, cx| {
                                unpin.update(cx, |app, cx| app.unpin_message(id, cx)).ok();
                            }),
                    )
                }),
        )
}

fn pins_popout(app: &DiscordApp, pins: &Pins, cx: &App) -> AnyElement {
    let can_unpin = app.store.can_manage_messages(pins.channel_id);
    let more = app.entity.clone();
    let cards: Vec<_> = pins
        .messages
        .iter()
        .map(|message| pin_card(app, message, can_unpin, cx))
        .collect();
    let empty = pins.messages.is_empty();
    frame(app, "pins-popout", PIN_WIDTH)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(rpx(8.))
                .h(rpx(57.))
                .px(rpx(16.))
                .border_b_1()
                .border_color(rgba(theme::border()))
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(theme::icon("pin", 24., theme::muted()))
                .child("Pinned Messages"),
        )
        .child(
            div()
                .id("pins-list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p(rpx(8.))
                .children(cards)
                .when(empty && pins.loading, |this| {
                    this.child(note("Loading\u{2026}"))
                })
                .when(pins.failed, |this| {
                    this.child(note("Could not load pinned messages."))
                })
                .when(empty && !pins.loading && !pins.failed, |this| {
                    this.child(note("This channel doesn't have any pinned messages yet."))
                })
                .when(pins.before.is_some() && !pins.loading, |this| {
                    this.child(load_more("pins-more", move |_, _, cx| {
                        more.update(cx, |app, cx| app.load_pins(cx)).ok();
                    }))
                }),
        )
        .into_any_element()
}

fn thread_card(app: &DiscordApp, guild_id: u64, row: &ThreadRow, cx: &App) -> impl IntoElement {
    let id = row.id;
    let open = app.entity.clone();
    let avatar = |user_id: u64, size: f32| {
        theme::undecorated_member_avatar(&app.store, guild_id, user_id, size, cx)
    };
    let name_of = |user_id: u64| {
        app.store
            .member_name(guild_id, user_id)
            .map(str::to_owned)
            .or_else(|| app.store.user(user_id).map(|user| user.name.clone()))
            .unwrap_or_else(|| "Unknown".to_owned())
    };
    let line = div()
        .flex()
        .items_center()
        .gap(rpx(6.))
        .min_w_0()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(theme::muted()));
    let line = match (&row.recent, row.archived) {
        (Some((author, text, at)), false) => line
            .child(avatar(*author, 16.))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(name_of(*author)),
            )
            .child(div().min_w_0().truncate().child(text.clone()))
            .child(div().flex_none().child(format!("\u{2022} {}", at.date()))),
        (None, false) => line
            .child("No recent messages")
            .children(row.last_active.map(|at| format!("\u{2022} {}", at.date()))),
        (_, true) => line
            .children(row.owner_id.map(|owner| avatar(owner, 16.)))
            .child("Started by")
            .children(row.owner_id.map(|owner| {
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(name_of(owner))
            }))
            .children(
                row.last_active
                    .map(|at| format!("\u{2022} Last active {}", at.date())),
            ),
    };
    let extra = (row.member_count as usize).saturating_sub(MAX_PREVIEW_AVATARS);
    div()
        .id(("thread-card", id))
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .h(rpx(72.))
        .mb(rpx(8.))
        .px(rpx(16.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::outline()))
        .hover(|style| style.bg(rgba(theme::selected())))
        .on_click(move |_, window, cx| {
            open.update(cx, |app, cx| app.open_listed_thread(id, window, cx))
                .ok();
        })
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(2.))
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child(row.name.clone()),
                )
                .child(line),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .children(
                    row.member_ids
                        .iter()
                        .take(MAX_PREVIEW_AVATARS)
                        .enumerate()
                        .map(|(index, user_id)| {
                            div()
                                .when(index > 0, |this| this.ml(rpx(-6.)))
                                .child(avatar(*user_id, 24.))
                        }),
                )
                .when(extra > 0, |this| {
                    this.child(
                        div()
                            .ml(rpx(-6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .h(rpx(24.))
                            .px(rpx(6.))
                            .rounded_full()
                            .bg(rgb(theme::bg()))
                            .text_size(rpx(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::bright()))
                            .child(format!("+{extra}")),
                    )
                }),
        )
}

fn section_heading(text: String, first: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(24.))
        .mt(rpx(if first { 8. } else { 16. }))
        .mb(rpx(8.))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .child(text)
}

fn plural(count: usize, name: &str) -> String {
    let noun = if count == 1 { "thread" } else { "threads" };
    format!("{count} {name} {noun}")
}

fn threads_popout(app: &DiscordApp, threads: &Threads, window: &Window, cx: &App) -> AnyElement {
    let guild_id = app.selected_guild_id;
    let width = (f32::from(window.viewport_size().width) / f32::from(window.rem_size())
        * 16.
        * THREAD_WIDTH_SHARE)
        .clamp(THREAD_MIN_WIDTH, THREAD_MAX_WIDTH);
    let more = app.entity.clone();
    let create = app.entity.clone();
    let can_create = app.store.can_create_thread(threads.parent_id, false);
    let list = div()
        .id("threads-list")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .px(rpx(16.))
        .pb(rpx(8.));
    let list = if let Some(results) = &threads.results {
        list.child(section_heading(plural(results.len(), "matching"), true))
            .children(
                results
                    .iter()
                    .map(|row| thread_card(app, guild_id, row, cx)),
            )
            .when(results.is_empty(), |this| {
                this.child(note("No threads match that name."))
            })
    } else {
        let (joined, others): (Vec<_>, Vec<_>) = app
            .store
            .active_threads(threads.parent_id)
            .into_iter()
            .partition(|row| row.joined);
        let empty = joined.is_empty() && others.is_empty() && threads.older.is_empty();
        let mut first = true;
        let mut heading = |text: String| {
            let element = section_heading(text, first);
            first = false;
            element
        };
        let joined_heading = (!joined.is_empty()).then(|| heading(plural(joined.len(), "joined")));
        let others_heading =
            (!others.is_empty()).then(|| heading(plural(others.len(), "other active")));
        let older_heading =
            (!threads.older.is_empty()).then(|| heading("older threads".to_owned()));
        list.children(joined_heading)
            .children(joined.iter().map(|row| thread_card(app, guild_id, row, cx)))
            .children(others_heading)
            .children(others.iter().map(|row| thread_card(app, guild_id, row, cx)))
            .children(older_heading)
            .children(
                threads
                    .older
                    .iter()
                    .map(|row| thread_card(app, guild_id, row, cx)),
            )
            .when(threads.loading, |this| this.child(note("Loading\u{2026}")))
            .when(empty && !threads.loading, |this| {
                this.child(note("There are no threads here yet."))
            })
            .when(threads.has_more && !threads.loading, |this| {
                this.child(load_more("threads-more", move |_, _, cx| {
                    more.update(cx, |app, cx| app.load_older_threads(cx)).ok();
                }))
            })
    };
    frame(app, "threads-popout", width)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(rpx(12.))
                .h(rpx(49.))
                .px(rpx(16.))
                .border_b_1()
                .border_color(rgba(theme::border()))
                .child(theme::icon("messages-square", 24., theme::muted()))
                .child(
                    div()
                        .flex_none()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child("Threads"),
                )
                .child(
                    div()
                        .flex_none()
                        .w(rpx(1.))
                        .h(rpx(24.))
                        .bg(rgba(theme::border())),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .h(rpx(32.))
                        .px(rpx(10.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::elevated()))
                        .border_1()
                        .border_color(rgba(theme::border()))
                        .child(theme::icon("search", 16., theme::muted()))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&threads.search)
                                    .appearance(false)
                                    .px_0()
                                    .text_size(rpx(14.)),
                            ),
                        ),
                )
                .when(can_create, |this| {
                    this.child(
                        div()
                            .id("threads-create")
                            .flex()
                            .flex_none()
                            .items_center()
                            .h(rpx(32.))
                            .px(rpx(12.))
                            .rounded(rpx(8.))
                            .cursor_pointer()
                            .bg(rgb(theme::BLURPLE))
                            .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
                            .text_size(rpx(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .on_click(move |_, window, cx| {
                                create
                                    .update(cx, |app, cx| app.open_create_thread(None, window, cx))
                                    .ok();
                            })
                            .child("Create"),
                    )
                }),
        )
        .child(list)
        .into_any_element()
}

pub(in crate::app) fn channel_popout(
    app: &DiscordApp,
    window: &Window,
    cx: &App,
) -> Option<AnyElement> {
    match app.channel_popout.as_ref()? {
        ChannelPopout::Pins(pins) => Some(pins_popout(app, pins, cx)),
        ChannelPopout::Threads(threads) => Some(threads_popout(app, threads, window, cx)),
    }
}
