use super::composer::{emoji_token_overlay, flush_textarea, send_icon};
use super::popouts::PopoutKind;
use super::rows::render_pane_row;
use super::*;
use crate::app::menu::PopupButton as _;
use crate::app::{
    OLDER_MESSAGES_THRESHOLD,
    home::{HomePage, blocked_bar, request_bar},
};
use crate::discord::model;
use crate::store::{Change, Channel, EmojiChoice};
use gpui_component::input::InputEvent;
use gpui_tokio::Tokio;
use std::time::Instant;

pub(crate) const WIDTH: f32 = 420.;
const LIST_OVERDRAW: f32 = 800.;
const PREVIEW_PAGE: usize = 1;
const ACK_DELAY: Duration = Duration::from_secs(1);
const MENU_WIDTH: f32 = 220.;
const MENU_REOPEN_GUARD: Duration = Duration::from_millis(200);

pub(in crate::app) struct ThreadPane {
    id: u64,
    list: ListState,
    composer: Entity<TextareaState>,
    ack: Option<Task<()>>,
    /// A forum post opened for the first time reads from its starter message, so the list
    /// keeps returning to the top while older pages load, until the history is complete.
    seek_top: bool,
    /// Read position when the pane opened, so the NEW divider stays put after the ack.
    unread_since: Option<u64>,
    menu: bool,
    menu_closed: Option<Instant>,
    /// The menu shows the notification levels under "Notification Settings".
    notify_open: bool,
    _submit: Subscription,
}

impl ThreadPane {
    pub(super) fn remeasure(&self) {
        self.list.remeasure();
    }
}

impl DiscordApp {
    /// A guild channel or thread, or a DM; the side pane shows any of them.
    pub(crate) fn channel_by_id(&self, id: u64) -> Option<&Channel> {
        self.store
            .guilds
            .iter()
            .find_map(|guild| guild.channel(id))
            .or_else(|| self.store.private_channel(id).map(|dm| &dm.channel))
    }

    pub(crate) fn history_channel(&self, id: u64) -> Option<&Channel> {
        if id == self.selected_channel_id {
            self.channel()
        } else {
            self.channel_by_id(id)
        }
    }

    pub(crate) fn thread_pane_channel(&self) -> Option<&Channel> {
        self.channel_by_id(self.thread_pane.as_ref()?.id)
    }

    pub(crate) fn thread_pane_open(&self) -> bool {
        let Some(pane) = &self.thread_pane else {
            return false;
        };
        // A message request opens beside the requests list, like Discord's sidebar, and only
        // there; a thread pane belongs to its guild.
        if self.store.private_channel(pane.id).is_some() {
            return self.showing_home_page() && self.home.page == HomePage::Requests;
        }
        !self.is_home() && self.channel_by_id(pane.id).is_some()
    }

    pub(crate) fn thread_pane_is_dm(&self) -> bool {
        self.thread_pane
            .as_ref()
            .is_some_and(|pane| self.store.private_channel(pane.id).is_some())
    }

    pub(crate) fn open_thread_pane(
        &mut self,
        thread_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .thread_pane
            .as_ref()
            .is_some_and(|pane| pane.id == thread_id)
        {
            return;
        }
        let Some((name, count, loaded)) = self.channel_by_id(thread_id).map(|thread| {
            (
                thread.name.clone(),
                thread.messages.len(),
                thread.messages_loaded,
            )
        }) else {
            return;
        };
        let placeholder = if self.store.forum_post(thread_id).is_some() {
            format!("Send a message in \"{name}\"")
        } else if self.store.private_channel(thread_id).is_some() {
            format!("Message @{}", self.store.private_channel_name(thread_id))
        } else {
            format!("Message {name}")
        };
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .context_menu(false)
        });
        composer.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            input.focus(window, cx);
        });
        let submit = cx.subscribe_in(
            &composer,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { shift: false, .. } = event {
                    this.send_pane_message(window, cx);
                }
            },
        );
        let list = ListState::new(count, ListAlignment::Bottom, px(LIST_OVERDRAW));
        list.set_follow_mode(FollowMode::Tail);
        let entity = self.entity.clone();
        list.set_scroll_handler(move |event, _, cx| {
            if event.visible_range.start < OLDER_MESSAGES_THRESHOLD {
                entity
                    .update(cx, |app, cx| app.fetch_older_in(thread_id, cx))
                    .ok();
            }
            if event.is_following_tail {
                let entity = entity.clone();
                cx.defer(move |cx| {
                    entity.update(cx, |app, cx| app.schedule_pane_ack(cx)).ok();
                });
            }
        });
        let post = self.store.forum_post(thread_id).is_some();
        let last_read = self.store.last_read(thread_id);
        let seek_top = post && last_read.is_none();
        // A post never opened is new from its starter message on.
        let unread_since = last_read.or(post.then_some(0));
        self.thread_pane = Some(ThreadPane {
            id: thread_id,
            list,
            composer,
            ack: None,
            seek_top,
            unread_since,
            menu: false,
            menu_closed: None,
            notify_open: false,
            _submit: submit,
        });
        if self.live && (!loaded || !self.store.at_present(thread_id)) {
            self.fetch_history(thread_id, None, window, cx);
        } else {
            self.seek_pane_top(thread_id, cx);
            self.schedule_pane_ack(cx);
        }
        cx.notify();
    }

    pub(crate) fn close_thread_pane(&mut self, cx: &mut Context<Self>) {
        self.thread_pane = None;
        cx.notify();
    }

    pub(crate) fn thread_pane_unread_since(&self, channel_id: u64) -> Option<u64> {
        self.thread_pane
            .as_ref()
            .filter(|pane| pane.id == channel_id)
            .and_then(|pane| pane.unread_since)
    }

    pub(crate) fn thread_pane_shows(&self, channel_id: u64) -> bool {
        self.thread_pane_open()
            && self
                .thread_pane
                .as_ref()
                .is_some_and(|pane| pane.id == channel_id)
    }

    pub(crate) fn thread_pane_focused(&self, window: &Window, cx: &App) -> bool {
        self.thread_pane
            .as_ref()
            .is_some_and(|pane| pane.composer.read(cx).focus_handle(cx).is_focused(window))
    }

    pub(in crate::app) fn focus_thread_pane_composer(&self, window: &mut Window, cx: &mut App) {
        if let Some(pane) = &self.thread_pane {
            let focus = pane.composer.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        }
    }

    fn schedule_pane_ack(&mut self, cx: &mut Context<Self>) {
        if !self.thread_pane_open() {
            return;
        }
        let Some(pane) = self
            .thread_pane
            .as_mut()
            .filter(|pane| pane.ack.is_none() && pane.list.is_following_tail())
        else {
            return;
        };
        let id = pane.id;
        pane.ack = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ACK_DELAY).await;
            this.update(cx, |app, cx| {
                let Some(pane) = app.thread_pane.as_mut().filter(|pane| pane.id == id) else {
                    return;
                };
                pane.ack = None;
                if pane.list.is_following_tail() && app.thread_pane_open() {
                    app.ack_channel_in(id, cx);
                }
            })
            .ok();
        }));
    }

    pub(crate) fn jump_in_thread_pane(&self, source: u64, target: u64) -> bool {
        let Some((pane, thread)) = self
            .thread_pane
            .as_ref()
            .and_then(|pane| Some((pane, self.channel_by_id(pane.id)?)))
        else {
            return false;
        };
        if !thread.messages.iter().any(|message| message.id == source) {
            return false;
        }
        if let Some(index) = thread
            .messages
            .iter()
            .position(|message| message.id == target)
        {
            pane.list.scroll_to_reveal_item(index);
        }
        true
    }

    fn expand_thread_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.thread_pane.take() {
            self.select_channel(pane.id, window, cx);
        }
    }

    pub(in crate::app) fn restore_thread_pane(
        &mut self,
        pane: Option<ThreadPane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = pane
            .as_ref()
            .map(|pane| pane.id)
            .filter(|id| self.channel_by_id(*id).is_some())
        else {
            return;
        };
        self.thread_pane = pane;
        self.reset_thread_pane(id, cx);
        self.fetch_history(id, None, window, cx);
    }

    pub(crate) fn reset_thread_pane(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        let count = self
            .channel_by_id(channel_id)
            .map_or(0, |channel| channel.messages.len());
        let Some(pane) = self
            .thread_pane
            .as_ref()
            .filter(|pane| pane.id == channel_id)
        else {
            return;
        };
        pane.list.reset(count);
        pane.list.set_follow_mode(FollowMode::Tail);
        self.seek_pane_top(channel_id, cx);
        self.schedule_pane_ack(cx);
    }

    pub(crate) fn prepend_thread_pane(
        &mut self,
        channel_id: u64,
        inserted: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane) = self
            .thread_pane
            .as_ref()
            .filter(|pane| pane.id == channel_id)
        {
            pane.list.splice(0..0, inserted);
            pane.list.remeasure_items(inserted..inserted + 1);
        }
        self.seek_pane_top(channel_id, cx);
    }

    /// Scrolls to the first loaded message; at the top the scroll handler fetches older pages.
    fn seek_pane_top(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        let complete = self.history_complete.contains(&channel_id);
        let Some(pane) = self
            .thread_pane
            .as_mut()
            .filter(|pane| pane.id == channel_id && pane.seek_top)
        else {
            return;
        };
        pane.list.set_follow_mode(FollowMode::Normal);
        pane.list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.),
        });
        if complete {
            pane.seek_top = false;
        } else {
            self.fetch_older_in(channel_id, cx);
        }
    }

    fn toggle_pane_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(pane) = &mut self.thread_pane {
            let guarded = pane
                .menu_closed
                .is_some_and(|at| at.elapsed() < MENU_REOPEN_GUARD);
            pane.menu = !guarded && !pane.menu;
            cx.notify();
        }
    }

    fn toggle_pane_notify(&mut self, cx: &mut Context<Self>) {
        if let Some(pane) = &mut self.thread_pane {
            pane.notify_open = !pane.notify_open;
            cx.notify();
        }
    }

    /// Applies the change locally first; Discord echoes it back over the gateway.
    fn update_thread_settings(
        &mut self,
        thread_id: u64,
        settings: model::ThreadSettings,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.store.set_thread_settings(thread_id, settings);
        let request = Tokio::spawn(cx, async move {
            session.update_thread_settings(thread_id, settings).await
        });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not update thread settings: {error}"),
            Err(error) => log::warn!("Thread settings task failed: {error}"),
        })
        .detach();
        cx.notify();
    }

    fn close_pane_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(pane) = self.thread_pane.as_mut().filter(|pane| pane.menu) {
            pane.menu = false;
            pane.notify_open = false;
            pane.menu_closed = Some(Instant::now());
            cx.notify();
        }
    }

    pub(crate) fn sync_thread_pane(&mut self, change: &Change, cx: &mut Context<Self>) {
        let Some(id) = self.thread_pane.as_ref().map(|pane| pane.id) else {
            return;
        };
        let len = self
            .channel_by_id(id)
            .map_or(0, |channel| channel.messages.len());
        let Some(pane) = &self.thread_pane else {
            return;
        };
        match *change {
            Change::MessageInserted { channel_id, index } if channel_id == id => {
                pane.list.splice(index..index, 1);
                pane.list
                    .remeasure_items(index.saturating_sub(1)..(index + 2).min(len));
                self.schedule_pane_ack(cx);
            }
            Change::MessageReplaced {
                channel_id,
                removed,
                inserted,
            } if channel_id == id => {
                pane.list.splice(removed..removed + 1, 0);
                pane.list.splice(inserted..inserted, 1);
                pane.list.remeasure_items(0..len);
            }
            Change::MessageChanged { channel_id, index } if channel_id == id => {
                pane.list.remeasure_items(index..(index + 2).min(len));
            }
            Change::MessageRemoved { channel_id, index } if channel_id == id => {
                pane.list.splice(index..index + 1, 0);
                pane.list
                    .remeasure_items(index.saturating_sub(1)..(index + 1).min(len));
            }
            _ => return,
        }
        cx.notify();
    }

    pub(super) fn pane_composer(&self) -> Option<Entity<TextareaState>> {
        self.thread_pane.as_ref().map(|pane| pane.composer.clone())
    }

    pub(super) fn send_pane_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(composer) = self.thread_pane.as_ref().map(|pane| pane.composer.clone()) else {
            return;
        };
        let content = composer_tokens::to_wire(&composer.read(cx).value());
        if self.send_pane_content(content.trim().to_owned(), window, cx) {
            composer.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    /// Sends text plus the pane's staged files to its thread; false when nothing could be sent.
    pub(crate) fn send_pane_content(
        &mut self,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(channel_id) = self.thread_pane.as_ref().map(|pane| pane.id) else {
            return false;
        };
        let has_files = !self.staged_files_in(channel_id).is_empty();
        if !self.live || (content.is_empty() && !has_files) || !self.store.can_send(channel_id) {
            return false;
        }
        let (content, used_emoji) = self.encode_emoji_text(&content);
        if content.chars().count() > MESSAGE_LIMIT {
            return false;
        }
        for choice in &used_emoji {
            self.record_emoji_use(choice, cx);
        }
        if let Some(pane) = &self.thread_pane {
            pane.list.set_follow_mode(FollowMode::Tail);
        }
        let files = self.staged.remove(&channel_id).unwrap_or_default();
        self.submit_send(channel_id, content, None, files, None, window, cx);
        true
    }

    pub(crate) fn insert_pane_emoji(
        &mut self,
        choice: &EmojiChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.emoji_insert_text(choice) else {
            return;
        };
        let Some(composer) = self.thread_pane.as_ref().map(|pane| pane.composer.clone()) else {
            return;
        };
        composer.update(cx, |input, cx| {
            input.insert(format!("{text} "), window, cx);
            input.focus(window, cx);
        });
        self.record_emoji_use(choice, cx);
        cx.notify();
    }

    pub(crate) fn load_thread_preview(&mut self, thread_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if !self.thread_previews.insert(thread_id) {
            return;
        }
        let guild_id = self.request_guild();
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_messages(guild_id, thread_id, None, PREVIEW_PAGE)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| match result {
                Ok(Ok(messages)) => {
                    if let Some(message) = messages.into_iter().next() {
                        app.store.set_thread_preview(thread_id, message);
                        app.messages.remeasure();
                        cx.notify();
                    }
                }
                Ok(Err(error)) => log::warn!("Could not load thread preview: {error}"),
                Err(error) => log::warn!("Thread preview task failed: {error}"),
            })
            .ok();
        })
        .detach();
    }
}

fn post_header(app: &DiscordApp, pane: &ThreadPane, name: &str) -> impl IntoElement + use<> {
    let id = pane.id;
    let toggle = app.entity.clone();
    let close = app.entity.clone();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .h(rpx(HEADER_HEIGHT))
        .pl_4()
        .pr_2()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(label(("thread-pane-name", id), name)),
        )
        .child(
            div()
                .relative()
                .flex_none()
                .children(pane.menu.then(|| post_menu(app, id, pane.notify_open)))
                .child(
                    theme::icon_button("thread-pane-more", "ellipsis", 20., theme::muted())
                        .relative()
                        .child(theme::hover_tooltip("thread-pane-more-tip", "More"))
                        .on_popup_click(
                            crate::app::menu::shown(app, |app| {
                                app.thread_pane.as_ref().is_some_and(|pane| pane.menu)
                            }),
                            move |_, _, cx| {
                                toggle.update(cx, |app, cx| app.toggle_pane_menu(cx)).ok();
                            },
                        ),
                ),
        )
        .child(
            theme::icon_button("thread-pane-close", "x", 20., theme::muted())
                .relative()
                .child(theme::hover_tooltip("post-pane-close-tip", "Close"))
                .on_click(move |_, _, cx| {
                    close.update(cx, |app, cx| app.close_thread_pane(cx)).ok();
                }),
        )
}

type MenuAction = Box<dyn Fn(&mut DiscordApp, &mut Window, &mut Context<DiscordApp>)>;

fn post_menu(app: &DiscordApp, id: u64, notify_open: bool) -> impl IntoElement + use<> {
    let row = app.store.thread_row_by_id(id);
    let joined = row.as_ref().is_some_and(|row| row.joined);
    let muted = row.as_ref().is_some_and(|row| row.muted);
    // An unset level behaves as "Only @mentions".
    let level = row.as_ref().map_or(0, |row| row.notify);
    let level = if level == 0 { 4 } else { level };
    let link = format!(
        "https://discord.com/channels/{}/{id}",
        app.selected_guild_id
    );
    let item = |key: &'static str,
                text: &'static str,
                trailing: Option<&'static str>,
                run: Option<MenuAction>| {
        let entity = app.entity.clone();
        let enabled = run.is_some();
        let color = if enabled {
            theme::strong()
        } else {
            theme::faint()
        };
        div()
            .id(key)
            .flex()
            .items_center()
            .justify_between()
            .gap(rpx(8.))
            .h(rpx(36.))
            .px(rpx(8.))
            .rounded(rpx(4.))
            .text_color(rgb(color))
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(rgba(theme::hover())))
            })
            .when_some(run, |this, run| {
                this.on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    entity
                        .update(cx, |app, cx| {
                            app.close_pane_menu(cx);
                            run(app, window, cx);
                        })
                        .ok();
                })
            })
            .child(text)
            .children(trailing.map(|icon| theme::icon(icon, 20., color)))
    };
    let separator = || {
        div()
            .h(rpx(1.))
            .my(rpx(8.))
            .mx(rpx(8.))
            .bg(rgba(theme::border_subtle()))
    };
    let close = app.entity.clone();
    div().absolute().top(rpx(32.)).right_0().size_0().child(
        deferred(
            anchored().anchor(Anchor::TopRight).child(
                div()
                    .id("thread-pane-menu")
                    .flex()
                    .flex_col()
                    .w(rpx(MENU_WIDTH))
                    .p(rpx(8.))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border_subtle()))
                    .shadow(vec![BoxShadow {
                        color: rgb(theme::shadow()).opacity(0.24).into(),
                        offset: point(px(0.), px(12.)),
                        blur_radius: px(24.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .occlude()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .without_text_selection()
                    .on_mouse_down_out(move |_, _, cx| {
                        close.update(cx, |app, cx| app.close_pane_menu(cx)).ok();
                    })
                    .child(item(
                        "post-follow",
                        if joined {
                            "Unfollow Post"
                        } else {
                            "Follow Post"
                        },
                        None,
                        Some(Box::new(move |app, _, cx| {
                            app.join_or_leave_thread(id, joined, cx)
                        })),
                    ))
                    .child(item(
                        "post-full-view",
                        "Open in Full View",
                        None,
                        Some(Box::new(|app, window, cx| {
                            app.expand_thread_pane(window, cx)
                        })),
                    ))
                    .child(separator())
                    .child(item(
                        "post-mute",
                        if muted { "Unmute Post" } else { "Mute Post" },
                        None,
                        Some(Box::new(move |app, _, cx| {
                            let settings = model::ThreadSettings {
                                muted: Some(!muted),
                                flags: None,
                            };
                            app.update_thread_settings(id, settings, cx);
                        })),
                    ))
                    .child({
                        let entity = app.entity.clone();
                        item(
                            "post-notifications",
                            "Notification Settings",
                            Some(if notify_open {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            }),
                            None,
                        )
                        .text_color(rgb(theme::strong()))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgba(theme::hover())))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            entity.update(cx, |app, cx| app.toggle_pane_notify(cx)).ok();
                        })
                    })
                    .when(notify_open, |this| {
                        this.children(
                            [(2, "All Messages"), (4, "Only @mentions"), (8, "Nothing")].map(
                                |(flag, text)| {
                                    let entity = app.entity.clone();
                                    div()
                                        .id(("post-notify", flag))
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .h(rpx(36.))
                                        .pl(rpx(20.))
                                        .pr(rpx(8.))
                                        .rounded(rpx(4.))
                                        .text_color(rgb(theme::strong()))
                                        .cursor_pointer()
                                        .hover(|style| style.bg(rgba(theme::hover())))
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            entity
                                                .update(cx, |app, cx| {
                                                    app.close_pane_menu(cx);
                                                    let settings = model::ThreadSettings {
                                                        muted: None,
                                                        flags: Some(flag),
                                                    };
                                                    app.update_thread_settings(id, settings, cx);
                                                })
                                                .ok();
                                        })
                                        .child(text)
                                        .child(radio_dot(level == flag))
                                },
                            ),
                        )
                    })
                    .child(separator())
                    .child(item(
                        "post-search",
                        "Search",
                        Some("external-link"),
                        Some(Box::new(|app, window, cx| {
                            app.expand_thread_pane(window, cx);
                            app.open_search(window, cx);
                        })),
                    ))
                    .child(item(
                        "post-pins",
                        "Pins",
                        Some("external-link"),
                        Some(Box::new(|app, window, cx| {
                            app.expand_thread_pane(window, cx);
                            app.toggle_channel_popout(PopoutKind::Pins, window, cx);
                        })),
                    ))
                    .child(separator())
                    .child(item(
                        "post-copy-link",
                        "Copy Link",
                        None,
                        Some(Box::new(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                        })),
                    ))
                    .child(item(
                        "post-copy-id",
                        "Copy Thread ID",
                        Some("id-badge"),
                        Some(Box::new(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()));
                        })),
                    )),
            ),
        )
        .with_priority(1),
    )
}

fn radio_dot(checked: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(20.))
        .rounded_full()
        .map(|this| {
            if checked {
                this.bg(rgb(theme::BLURPLE))
                    .child(div().size(rpx(8.)).rounded_full().bg(white()))
            } else {
                this.border_2().border_color(rgb(theme::muted()))
            }
        })
}

fn header(app: &DiscordApp, id: u64, name: &str) -> impl IntoElement + use<> {
    let joined = app.store.thread_row_by_id(id).is_none_or(|row| row.joined);
    let join = app.entity.clone();
    let expand = app.entity.clone();
    let close = app.entity.clone();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .h(rpx(HEADER_HEIGHT))
        .pl_4()
        .pr_2()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(theme::icon("thread", 20., theme::faint()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(label(("thread-pane-name", id), name)),
        )
        .when(!joined, |this| {
            this.child(
                div()
                    .id("thread-pane-join")
                    .flex_none()
                    .px_2()
                    .py_1()
                    .rounded(rpx(6.))
                    .cursor_pointer()
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .hover(|style| style.bg(rgba(theme::hover())))
                    .on_click(move |_, _, cx| {
                        join.update(cx, |app, cx| app.join_or_leave_thread(id, false, cx))
                            .ok();
                    })
                    .child("Join Thread"),
            )
        })
        .child(
            theme::icon_button("thread-pane-expand", "maximize-2", 20., theme::muted())
                .relative()
                .child(theme::hover_tooltip(
                    "thread-pane-expand-tip",
                    "Open in Full View",
                ))
                .on_click(move |_, window, cx| {
                    expand
                        .update(cx, |app, cx| app.expand_thread_pane(window, cx))
                        .ok();
                }),
        )
        .child(
            theme::icon_button("thread-pane-close", "x", 20., theme::muted())
                .relative()
                .child(theme::hover_tooltip("thread-pane-close-tip", "Close"))
                .on_click(move |_, _, cx| {
                    close.update(cx, |app, cx| app.close_thread_pane(cx)).ok();
                }),
        )
}

/// A message request's sidebar header, like Discord's: the sender's avatar and name, and close.
fn dm_header(app: &DiscordApp, id: u64, cx: &App) -> impl IntoElement + use<> {
    let close = app.entity.clone();
    let name = app.store.private_channel_name(id);
    let user_id = app
        .store
        .private_channel(id)
        .and_then(|dm| dm.recipient_ids.first().copied())
        .unwrap_or(0);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .h(rpx(HEADER_HEIGHT))
        .pl_4()
        .pr_2()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(theme::user_avatar(
            app.store.user(user_id),
            user_id,
            24.,
            None,
            false,
            cx,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(label(("dm-pane-name", id), &name)),
        )
        .child(
            theme::icon_button("dm-pane-close", "x", 20., theme::muted())
                .relative()
                .child(theme::hover_tooltip("dm-pane-close-tip", "Close"))
                .on_click(move |_, _, cx| {
                    close.update(cx, |app, cx| app.close_thread_pane(cx)).ok();
                }),
        )
}

fn body(app: &DiscordApp, pane: &ThreadPane, thread: &Channel) -> AnyElement {
    let id = pane.id;
    if app.live && !thread.messages_loaded {
        if app.is_loading(id) {
            return div()
                .flex_1()
                .min_h_0()
                .child(skeleton(id))
                .into_any_element();
        }
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .text_color(rgb(theme::muted()))
            .child("Couldn't load messages.")
            .into_any_element();
    }
    let entity = app.entity.clone();
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .w_full()
        .child(
            list(pane.list.clone(), move |index, _, cx| {
                let Some(app) = entity.upgrade() else {
                    return div().into_any_element();
                };
                render_pane_row(app.read(cx), id, index, cx)
            })
            .flex_1()
            .min_h_0()
            .w_full(),
        )
        .vertical_scrollbar(&pane.list)
        .into_any_element()
}

fn composer_box(app: &DiscordApp, pane: &ThreadPane, cx: &App) -> impl IntoElement {
    let id = pane.id;
    let can_send = app.store.can_send(id);
    let staged = app.staged_files_in(id);
    let send_ready = can_send
        && (!staged.is_empty()
            || app
                .pane_composer()
                .is_some_and(|input| !input.read(cx).text().to_string().trim().is_empty()));
    let picker_anchor = Rc::new(Cell::new(None));
    let attach = app.entity.clone();
    let gif = app.entity.clone();
    let sticker = app.entity.clone();
    let emoji = app.entity.clone();
    let gif_anchor = picker_anchor.clone();
    let sticker_anchor = picker_anchor.clone();
    let emoji_anchor = picker_anchor.clone();
    let send_button = crate::settings::get(cx).composer_send_button;
    div()
        .flex_none()
        .px_2()
        .pb_2()
        .on_prepaint({
            let anchor = picker_anchor.clone();
            move |bounds, window, _| {
                let gap = rpx(8.).to_pixels(window.rem_size());
                let right_inset = rpx(24.).to_pixels(window.rem_size());
                anchor.set(Some(point(
                    bounds.right() - right_inset,
                    bounds.top() - gap,
                )));
            }
        })
        .children((!staged.is_empty()).then(|| super::composer::staged_row(app, staged, cx)))
        .child(
            div()
                .relative()
                .flex()
                .items_start()
                .gap_1()
                .min_h(rpx(44.))
                .pl(rpx(12.))
                .pr(rpx(8.))
                .map(|this| {
                    if staged.is_empty() {
                        this.rounded(rpx(8.))
                    } else {
                        this.rounded_b(rpx(8.))
                    }
                })
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgba(theme::border()))
                .on_mouse_down(MouseButton::Right, {
                    let entity = app.entity.clone();
                    move |event, _, cx| {
                        cx.stop_propagation();
                        let position = event.position;
                        entity
                            .update(cx, |app, cx| {
                                app.open_composer_menu(ComposerTarget::Pane, position, cx)
                            })
                            .ok();
                    }
                })
                .child(
                    div().my(rpx(10.)).child(
                        theme::icon_button("pane-attach", "circle-plus", 24., theme::muted())
                            .when(!can_send, |this| this.opacity(0.5))
                            .when(can_send, |this| {
                                this.on_click(move |_, window, cx| {
                                    attach
                                        .update(cx, |app, cx| app.pick_files_for(id, window, cx))
                                        .ok();
                                })
                            }),
                    ),
                )
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .ml_2()
                        .child(
                            flush_textarea(Textarea::new(&pane.composer))
                                .disabled(!can_send)
                                .py(rpx(11.))
                                .text_size(rpx(16.))
                                .line_height(rpx(22.))
                                .text_color(rgb(theme::text())),
                        )
                        .child(emoji_token_overlay(&pane.composer)),
                )
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .my(rpx(10.))
                        .child(
                            theme::icon_button("pane-gif", "image-play", 24., theme::muted())
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    let Some(position) = gif_anchor.get() else {
                                        return;
                                    };
                                    gif.update(cx, |app, cx| {
                                        app.toggle_gif_picker_for(true, position, window, cx)
                                    })
                                    .ok();
                                }),
                        )
                        .child(
                            theme::icon_button("pane-stickers", "sticker", 24., theme::muted())
                                .relative()
                                .child(theme::hover_tooltip("pane-stickers-tip", "Stickers"))
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    let Some(position) = sticker_anchor.get() else {
                                        return;
                                    };
                                    sticker
                                        .update(cx, |app, cx| {
                                            app.toggle_sticker_picker(
                                                PickerTarget::Pane,
                                                position,
                                                window,
                                                cx,
                                            )
                                        })
                                        .ok();
                                }),
                        )
                        .child(
                            theme::icon_button("pane-emoji", "smile", 24., theme::muted())
                                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                    let Some(position) = emoji_anchor.get() else {
                                        return;
                                    };
                                    emoji
                                        .update(cx, |app, cx| {
                                            app.toggle_emoji_picker(
                                                PickerTarget::Pane,
                                                position,
                                                window,
                                                cx,
                                            )
                                        })
                                        .ok();
                                }),
                        )
                        .children(send_button.then(|| {
                            let entity = app.entity.clone();
                            send_icon(
                                "pane-send",
                                send_ready,
                                &app.pane_send_fill,
                                move |_, window, cx| {
                                    entity
                                        .update(cx, |app, cx| app.send_pane_message(window, cx))
                                        .ok();
                                },
                            )
                        })),
                ),
        )
}

pub(crate) fn render_thread_pane(app: &DiscordApp, cx: &App) -> AnyElement {
    let Some((pane, thread)) = app
        .thread_pane
        .as_ref()
        .and_then(|pane| Some((pane, app.channel_by_id(pane.id)?)))
    else {
        return div().into_any_element();
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .border_l_1()
        .border_color(rgba(theme::border()))
        .child(if app.store.forum_post(pane.id).is_some() {
            post_header(app, pane, &thread.name).into_any_element()
        } else if app.store.private_channel(pane.id).is_some() {
            dm_header(app, pane.id, cx).into_any_element()
        } else {
            header(app, pane.id, &thread.name).into_any_element()
        })
        .child(body(app, pane, thread))
        .child(
            match request_bar(app, pane.id).or_else(|| blocked_bar(app, pane.id)) {
                Some(bar) => bar,
                None => composer_box(app, pane, cx).into_any_element(),
            },
        )
        .into_any_element()
}
