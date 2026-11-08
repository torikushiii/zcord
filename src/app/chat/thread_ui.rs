use gpui_component::input::{Input, InputEvent, InputState};
use gpui_tokio::Tokio;
use std::{future::Future, pin::Pin};

use super::actions::{menu_divider, menu_entry};
use super::*;
use crate::{
    app::{guilds::forum::relative_time, titlebar},
    discord::Error,
};

const NAME_LIMIT: usize = 100;
const DEFAULT_NAME_CHARS: usize = 40;

pub(in crate::app) struct CreateThread {
    channel_id: u64,
    message_id: Option<u64>,
    name: Entity<InputState>,
    private: bool,
    busy: bool,
    error: Option<&'static str>,
    _events: Subscription,
}

#[derive(Clone, Copy)]
pub(in crate::app) struct ThreadMenu {
    thread_id: u64,
    position: Point<Pixels>,
    delete_armed: bool,
}

type ThreadCall = Pin<Box<dyn Future<Output = Result<(), Error>> + Send>>;

fn failure(error: &Error) -> &'static str {
    match error {
        Error::Captcha => "Discord asked for verification. Try this in the official client.",
        Error::RateLimited { .. } => "You're doing that too fast. Try again in a moment.",
        _ => "Could not create the thread. Try again.",
    }
}

impl DiscordApp {
    pub(in crate::app) fn open_create_thread(
        &mut self,
        message_id: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.channel_popout = None;
        self.message_menu = None;
        let channel_id = self.thread_parent().unwrap_or(self.selected_channel_id);
        let suggested: String = message_id
            .and_then(|id| {
                self.channel()?
                    .messages
                    .iter()
                    .find(|message| message.id == id)
            })
            .and_then(|message| message.content.lines().next())
            .map(|line| line.chars().take(DEFAULT_NAME_CHARS).collect())
            .unwrap_or_default();
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Thread name")
                .default_value(suggested)
        });
        let events = cx.subscribe_in(&name, window, |app, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                app.submit_create_thread(window, cx);
            }
        });
        name.update(cx, |input, cx| input.focus(window, cx));
        self.create_thread = Some(CreateThread {
            channel_id,
            message_id,
            name,
            private: false,
            busy: false,
            error: None,
            _events: events,
        });
        cx.notify();
    }

    pub(in crate::app) fn close_create_thread(&mut self, cx: &mut Context<Self>) -> bool {
        if self.create_thread.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn submit_create_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(dialog) = &mut self.create_thread else {
            return;
        };
        let name: String = dialog
            .name
            .read(cx)
            .value()
            .trim()
            .chars()
            .take(NAME_LIMIT)
            .collect();
        if name.is_empty() || dialog.busy {
            return;
        }
        dialog.busy = true;
        dialog.error = None;
        let (channel_id, message_id, private) =
            (dialog.channel_id, dialog.message_id, dialog.private);
        let request = Tokio::spawn(cx, async move {
            match message_id {
                Some(message_id) => session.create_thread(channel_id, message_id, &name).await,
                None => {
                    session
                        .create_standalone_thread(channel_id, &name, private)
                        .await
                }
            }
        });
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                match result {
                    Ok(Ok(thread)) => {
                        let id = thread.id;
                        app.store.remember_thread(thread);
                        app.create_thread = None;
                        app.select_channel(id, window, cx);
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not create thread: {error}");
                        if let Some(dialog) = &mut app.create_thread {
                            dialog.busy = false;
                            dialog.error = Some(failure(&error));
                        }
                    }
                    Err(error) => {
                        log::warn!("Create thread task failed: {error}");
                        if let Some(dialog) = &mut app.create_thread {
                            dialog.busy = false;
                            dialog.error = Some("Could not create the thread. Try again.");
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(in crate::app) fn close_thread_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.thread_menu.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn run_thread_call(&mut self, label: &'static str, call: ThreadCall, cx: &mut Context<Self>) {
        let request = Tokio::spawn(cx, call);
        cx.spawn(async move |_, _| {
            if let Ok(Err(error)) = request.await {
                log::warn!("Could not {label} thread: {error}");
            }
        })
        .detach();
    }

    pub(super) fn join_or_leave_thread(
        &mut self,
        thread_id: u64,
        joined: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let call: ThreadCall = Box::pin(async move {
            if joined {
                session.leave_thread(thread_id).await
            } else {
                session.join_thread(thread_id).await
            }
        });
        self.run_thread_call(if joined { "leave" } else { "join" }, call, cx);
    }

    fn set_thread_archived(&mut self, thread_id: u64, archived: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let call: ThreadCall =
            Box::pin(async move { session.archive_thread(thread_id, archived).await });
        self.run_thread_call(if archived { "close" } else { "reopen" }, call, cx);
    }

    fn delete_thread(&mut self, thread_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let parent = self.thread_parent();
        let call: ThreadCall = Box::pin(async move { session.delete_thread(thread_id).await });
        self.run_thread_call("delete", call, cx);
        if let Some(parent) = parent {
            self.select_channel(parent, window, cx);
        }
    }
}

pub(super) fn thread_menu_button(app: &DiscordApp) -> Option<Stateful<Div>> {
    app.thread_parent()?;
    let thread_id = app.selected_channel_id;
    let entity = app.entity.clone();
    Some(
        theme::icon_button("thread-more", "ellipsis", 20., theme::muted())
            .relative()
            .child(theme::hover_tooltip("thread-more-tip", "More"))
            .on_click(move |event, _, cx| {
                let position = event.position();
                entity
                    .update(cx, |app, cx| {
                        app.thread_menu = match app.thread_menu {
                            Some(_) => None,
                            None => Some(ThreadMenu {
                                thread_id,
                                position,
                                delete_armed: false,
                            }),
                        };
                        cx.notify();
                    })
                    .ok();
            }),
    )
}

pub(in crate::app) fn thread_menu(app: &DiscordApp) -> Option<AnyElement> {
    let menu = app.thread_menu?;
    let id = menu.thread_id;
    let row = app.store.thread_row_by_id(id)?;
    let joined = row.joined;
    let archived = row.archived;
    let manage = app.store.can_manage_thread(id);
    let close = app.entity.clone();
    let text = theme::text();
    let body = div()
        .id("thread-menu")
        .flex()
        .flex_col()
        .w(rpx(200.))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(move |_, _, cx| {
            close.update(cx, |app, cx| app.close_thread_menu(cx)).ok();
        })
        .child(menu_entry(
            app,
            "thread-join",
            "users",
            if joined {
                "Leave Thread"
            } else {
                "Join Thread"
            },
            text,
            move |app, _, _, cx| {
                app.thread_menu = None;
                app.join_or_leave_thread(id, joined, cx);
            },
        ))
        .when(manage, |this| {
            this.child(menu_entry(
                app,
                "thread-archive",
                "lock",
                if archived {
                    "Reopen Thread"
                } else {
                    "Close Thread"
                },
                text,
                move |app, _, _, cx| {
                    app.thread_menu = None;
                    app.set_thread_archived(id, !archived, cx);
                },
            ))
            .child(menu_divider())
            .child(menu_entry(
                app,
                "thread-delete",
                "trash-2",
                if menu.delete_armed {
                    "Click to Confirm"
                } else {
                    "Delete Thread"
                },
                theme::RED,
                move |app, _, window, cx| {
                    if let Some(menu) = app.thread_menu.as_mut().filter(|menu| !menu.delete_armed) {
                        menu.delete_armed = true;
                        cx.notify();
                        return;
                    }
                    app.thread_menu = None;
                    app.delete_thread(id, window, cx);
                },
            ))
        });
    Some(
        deferred(
            anchored()
                .position(menu.position)
                .anchor(Anchor::TopRight)
                .snap_to_window_with_margin(px(8.))
                .child(body),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

pub(in crate::app) fn create_thread_dialog(app: &DiscordApp) -> Option<AnyElement> {
    let dialog = app.create_thread.as_ref()?;
    let can_private =
        dialog.message_id.is_none() && app.store.can_create_thread(dialog.channel_id, true);
    let backdrop = app.entity.clone();
    let cancel = app.entity.clone();
    let submit = app.entity.clone();
    let private = app.entity.clone();
    let checked = dialog.private;
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
            .id("create-thread-backdrop")
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
                    .update(cx, |app, cx| app.close_create_thread(cx))
                    .ok();
            })
            .child(
                div()
                    .id("create-thread")
                    .flex()
                    .flex_col()
                    .gap(rpx(12.))
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
                            .child("Create Thread"),
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
                                    Input::new(&dialog.name)
                                        .appearance(false)
                                        .px_0()
                                        .text_size(rpx(16.)),
                                ),
                            ),
                    )
                    .when(can_private, |this| {
                        this.child(
                            div()
                                .id("create-thread-private")
                                .flex()
                                .items_center()
                                .gap_2()
                                .cursor_pointer()
                                .text_size(rpx(14.))
                                .text_color(rgb(theme::text()))
                                .on_click(move |_, _, cx| {
                                    private
                                        .update(cx, |app, cx| {
                                            if let Some(dialog) = &mut app.create_thread {
                                                dialog.private = !dialog.private;
                                                cx.notify();
                                            }
                                        })
                                        .ok();
                                })
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .size(rpx(20.))
                                        .rounded(rpx(4.))
                                        .border_1()
                                        .border_color(rgba(theme::border()))
                                        .when(checked, |this| {
                                            this.bg(rgb(theme::BLURPLE))
                                                .child(theme::icon("check", 14., 0xFFFFFF))
                                        }),
                                )
                                .child("Private Thread"),
                        )
                    })
                    .children(dialog.error.map(|text| {
                        div()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::RED))
                            .child(text)
                    }))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button(
                                    "create-thread-cancel",
                                    "Cancel",
                                    theme::button_subtle(),
                                    theme::button_subtle_hover(),
                                    rgb(theme::bright()).into(),
                                )
                                .on_click(move |_, _, cx| {
                                    cancel
                                        .update(cx, |app, cx| app.close_create_thread(cx))
                                        .ok();
                                }),
                            )
                            .child(
                                button(
                                    "create-thread-submit",
                                    "Create",
                                    theme::BLURPLE,
                                    theme::BLURPLE_ACTIVE,
                                    white(),
                                )
                                .when(dialog.busy, |this| this.opacity(0.5))
                                .on_click(move |_, window, cx| {
                                    submit
                                        .update(cx, |app, cx| app.submit_create_thread(window, cx))
                                        .ok();
                                }),
                            ),
                    ),
            )
            .into_any_element(),
    )
}

fn notice_link(
    id: (&'static str, u64),
    text: String,
    action: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .cursor_pointer()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()))
        .hover(|style| style.underline())
        .on_click(move |_, window, cx| action(window, cx))
        .child(text)
}

pub(super) fn system_tail(app: &DiscordApp, message: &Message) -> Option<Div> {
    let id = message.id;
    let open_list = |kind: PopoutKind| {
        let entity = app.entity.clone();
        move |window: &mut Window, cx: &mut App| {
            entity
                .update(cx, |app, cx| app.toggle_channel_popout(kind, window, cx))
                .ok();
        }
    };
    let row = div().flex().flex_wrap().items_baseline().gap(rpx(4.));
    match message.kind {
        3 => {
            let channel_id = message.call.as_ref()?.channel_id;
            let joined = app
                .voice
                .call
                .as_ref()
                .is_some_and(|call| call.channel_id == channel_id);
            if joined || !super::message::call_active(app, message) {
                return None;
            }
            let entity = app.entity.clone();
            Some(row.child(notice_link(
                ("notice-call", id),
                "Join the call".to_owned(),
                move |_, cx| {
                    entity
                        .update(cx, |app, cx| app.join_dm_call(channel_id, cx))
                        .ok();
                },
            )))
        }
        6 => {
            let jump = app.entity.clone();
            let target = message.reference_id;
            let word = notice_link(
                ("notice-pin", id),
                "a message".to_owned(),
                move |window, cx| {
                    if let Some(target) = target {
                        jump.update(cx, |app, cx| app.jump_to_message(target, window, cx))
                            .ok();
                    }
                },
            );
            Some(
                row.child("pinned")
                    .child(word)
                    .child("to this channel. See all")
                    .child(
                        div()
                            .flex()
                            .child(notice_link(
                                ("notice-pins", id),
                                "pinned messages".to_owned(),
                                open_list(PopoutKind::Pins),
                            ))
                            .child("."),
                    ),
            )
        }
        18 => {
            let open = app.entity.clone();
            let thread_id = message.thread.as_ref().map(|thread| thread.id);
            let name = notice_link(
                ("notice-thread", id),
                message.content.clone(),
                move |window, cx| {
                    if let Some(thread_id) = thread_id {
                        open.update(cx, |app, cx| app.open_listed_thread(thread_id, window, cx))
                            .ok();
                    }
                },
            );
            Some(
                row.child("started a thread:")
                    .child(div().flex().child(name).child("."))
                    .child("See all")
                    .child(
                        div()
                            .flex()
                            .child(notice_link(
                                ("notice-threads", id),
                                "threads".to_owned(),
                                open_list(PopoutKind::Threads),
                            ))
                            .child("."),
                    ),
            )
        }
        _ => None,
    }
}

pub(super) fn thread_notice_chip(app: &DiscordApp, message: &Message, cx: &App) -> Option<Div> {
    if message.kind != 18 {
        return None;
    }
    let chip = thread_chip(app, message, cx)?;
    Some(
        div()
            .relative()
            .flex()
            .pl(rpx(GUTTER))
            .child(
                div()
                    .absolute()
                    .left(rpx(GUTTER / 2.))
                    .top_0()
                    .w(rpx(GUTTER / 2. - 6.))
                    .h(rpx(34.))
                    .border_l_1()
                    .border_b_1()
                    .rounded_bl(rpx(8.))
                    .border_color(rgba(theme::connector())),
            )
            .child(chip),
    )
}

pub(super) fn thread_chip(app: &DiscordApp, message: &Message, cx: &App) -> Option<AnyElement> {
    let thread = message.thread.as_ref()?;
    if thread.message_count == 0 && thread.recent.is_none() {
        return None;
    }
    let id = thread.id;
    let entity = app.entity.clone();
    let guild_id = app.selected_guild_id;
    let count = thread.message_count;
    let noun = if count == 1 { "Message" } else { "Messages" };
    let recent = thread.recent.as_ref().map(|(author, text, at)| {
        let name = app
            .store
            .member_name(guild_id, *author)
            .map(str::to_owned)
            .or_else(|| app.store.user(*author).map(|user| user.name.clone()))
            .unwrap_or_default();
        div()
            .flex()
            .items_center()
            .gap(rpx(6.))
            .min_w_0()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .text_color(rgb(theme::muted()))
            .child(theme::undecorated_member_avatar(
                &app.store, guild_id, *author, 16., cx,
            ))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(name),
            )
            .child(div().min_w_0().truncate().child(text.clone()))
            .child(div().flex_none().child(relative_time(*at)))
    });
    Some(
        div()
            .id(("thread-chip", id))
            .flex()
            .flex_col()
            .gap(rpx(2.))
            .mt(rpx(6.))
            .self_start()
            .min_w_0()
            .max_w(rpx(480.))
            .px(rpx(12.))
            .py(rpx(8.))
            .rounded(rpx(8.))
            .cursor_pointer()
            .bg(rgb(theme::elevated()))
            .border_1()
            .border_color(rgba(theme::outline()))
            .hover(|style| style.bg(rgba(theme::selected())))
            .on_click(move |_, window, cx| {
                entity
                    .update(cx, |app, cx| app.open_listed_thread(id, window, cx))
                    .ok();
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::strong()))
                            .child(thread.name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::link()))
                            .child(format!("{count} {noun} \u{203a}")),
                    ),
            )
            .children(recent)
            .into_any_element(),
    )
}
