use super::composer::{emoji_token_overlay, flush_textarea};
use super::rows::render_pane_row;
use super::*;
use crate::app::OLDER_MESSAGES_THRESHOLD;
use crate::store::{Change, Channel};
use gpui_component::input::InputEvent;
use gpui_tokio::Tokio;

pub(crate) const WIDTH: f32 = 420.;
const LIST_OVERDRAW: f32 = 800.;
const PREVIEW_PAGE: usize = 1;
const ACK_DELAY: Duration = Duration::from_secs(1);

pub(in crate::app) struct ThreadPane {
    id: u64,
    list: ListState,
    composer: Entity<TextareaState>,
    ack: Option<Task<()>>,
    _submit: Subscription,
}

impl DiscordApp {
    pub(crate) fn channel_by_id(&self, id: u64) -> Option<&Channel> {
        self.store.guilds.iter().find_map(|guild| guild.channel(id))
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
        !self.is_home()
            && self
                .thread_pane
                .as_ref()
                .is_some_and(|pane| self.channel_by_id(pane.id).is_some())
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
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
        });
        composer.update(cx, |input, cx| {
            input.set_placeholder(format!("Message {name}"), window, cx);
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
        self.thread_pane = Some(ThreadPane {
            id: thread_id,
            list,
            composer,
            ack: None,
            _submit: submit,
        });
        if self.live && (!loaded || !self.store.at_present(thread_id)) {
            self.fetch_history(thread_id, None, window, cx);
        } else {
            self.schedule_pane_ack(cx);
        }
        cx.notify();
    }

    pub(crate) fn close_thread_pane(&mut self, cx: &mut Context<Self>) {
        self.thread_pane = None;
        cx.notify();
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

    pub(crate) fn restore_thread_pane(
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
        self.schedule_pane_ack(cx);
    }

    pub(crate) fn prepend_thread_pane(&self, channel_id: u64, inserted: usize) {
        if let Some(pane) = self
            .thread_pane
            .as_ref()
            .filter(|pane| pane.id == channel_id)
        {
            pane.list.splice(0..0, inserted);
            pane.list.remeasure_items(inserted..inserted + 1);
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

    fn send_pane_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((channel_id, composer)) = self
            .thread_pane
            .as_ref()
            .map(|pane| (pane.id, pane.composer.clone()))
        else {
            return;
        };
        let content = composer_tokens::to_wire(&composer.read(cx).value());
        let content = content.trim().to_owned();
        if !self.live || content.is_empty() || !self.store.can_send(channel_id) {
            return;
        }
        let (content, used_emoji) = self.encode_emoji_text(&content);
        if content.chars().count() > MESSAGE_LIMIT {
            return;
        }
        for choice in &used_emoji {
            self.record_emoji_use(choice, cx);
        }
        composer.update(cx, |input, cx| input.set_value("", window, cx));
        if let Some(pane) = &self.thread_pane {
            pane.list.set_follow_mode(FollowMode::Tail);
        }
        self.submit_send(channel_id, content, None, Vec::new(), window, cx);
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
        .border_color(rgba(theme::BORDER))
        .child(theme::icon("thread", 20., theme::FAINT))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::STRONG))
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
                    .text_color(rgb(theme::STRONG))
                    .hover(|style| style.bg(rgba(theme::HOVER)))
                    .on_click(move |_, _, cx| {
                        join.update(cx, |app, cx| app.join_or_leave_thread(id, false, cx))
                            .ok();
                    })
                    .child("Join Thread"),
            )
        })
        .child(
            theme::icon_button("thread-pane-expand", "maximize-2", 20., theme::MUTED)
                .tooltip(|_, cx| theme::text_tooltip("Open in Full View", cx))
                .on_click(move |_, window, cx| {
                    expand
                        .update(cx, |app, cx| app.expand_thread_pane(window, cx))
                        .ok();
                }),
        )
        .child(
            theme::icon_button("thread-pane-close", "x", 20., theme::MUTED)
                .tooltip(|_, cx| theme::text_tooltip("Close", cx))
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
            .text_color(rgb(theme::MUTED))
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

fn composer_box(app: &DiscordApp, pane: &ThreadPane) -> impl IntoElement {
    div().flex_none().px_2().pb_2().child(
        div()
            .relative()
            .min_h(rpx(44.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgb(theme::ELEVATED))
            .border_1()
            .border_color(rgba(theme::BORDER))
            .child(
                flush_textarea(Textarea::new(&pane.composer))
                    .disabled(!app.store.can_send(pane.id))
                    .py(rpx(11.))
                    .text_size(rpx(16.))
                    .line_height(rpx(22.))
                    .text_color(rgb(theme::TEXT)),
            )
            .child(emoji_token_overlay(&pane.composer)),
    )
}

pub(crate) fn render_thread_pane(app: &DiscordApp) -> AnyElement {
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
        .border_color(rgba(theme::BORDER))
        .child(header(app, pane.id, &thread.name))
        .child(body(app, pane, thread))
        .child(composer_box(app, pane))
        .into_any_element()
}
