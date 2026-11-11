use super::*;
use crate::store::Delivery;

mod paging;

impl DiscordApp {
    pub(super) fn reset_messages(&mut self, count: usize) {
        self.messages.reset(count);
        self.messages.set_follow_mode(FollowMode::Tail);
    }

    pub(crate) fn jump_to_present(
        &mut self,
        limit: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.screen, Screen::Chat) {
            return;
        }
        smooth_scroll::cancel(cx);
        self.unread.since = None;
        self.unread.bar = false;
        self.unread.manual = None;
        self.history.jump_return = None;
        let channel_id = self.selected_channel_id;
        let sending = self.compose.pending_sends.has_channel(channel_id);
        let needs_present = self.live
            && (sending
                || self.channel().is_some_and(|channel| {
                    !channel.messages_loaded || !self.store.at_present(channel.id)
                }));
        if needs_present {
            self.jumps.scroll = None;
            if !sending
                && self
                    .channel()
                    .is_some_and(|channel| channel.messages_loaded)
                && self.present_from_trimmed(limit, window, cx)
            {
                self.mark_channel_read(cx);
                return;
            }
            self.messages.set_follow_mode(FollowMode::Tail);
            self.history.pending_present.insert(channel_id);
            self.history.present_limit.insert(channel_id, limit);
            self.fetch_history(channel_id, None, window, cx);
        } else {
            self.glide_to_present(window, cx);
            self.mark_channel_read(cx);
        }
    }

    fn glide_to_present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.reduce_motion() || self.messages.is_following_tail() {
            self.jumps.scroll = None;
            self.messages.set_follow_mode(FollowMode::Tail);
            return;
        }
        self.start_jump(JumpTarget::Present, window, cx);
    }

    fn start_jump(&mut self, target: JumpTarget, window: &mut Window, cx: &mut Context<Self>) {
        let nonce = self.jumps.scroll.as_ref().map_or(0, |jump| jump.nonce) + 1;
        self.jumps.scroll = Some(JumpScroll { target, nonce });
        self.step_jump(nonce, window, cx);
    }

    pub(crate) fn retry_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        if self.history.loading.contains(&channel_id) {
            return;
        }
        let limit = self.page_size(cx);
        self.jump_to_present(limit, window, cx);
        if !self.history.loading.contains(&channel_id) {
            self.history.failed.remove(&channel_id);
        }
        cx.notify();
    }

    pub(crate) fn jump_to_quoted(
        &mut self,
        reply_id: u64,
        quoted_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_to_message(quoted_id, window, cx);
        self.history.jump_return = Some((self.selected_channel_id, reply_id));
    }

    pub(crate) fn jump_back_to_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((channel_id, reply_id)) = self.history.jump_return.take()
            && channel_id == self.selected_channel_id
        {
            self.jump_to_message(reply_id, window, cx);
        }
    }

    pub(super) fn scroll_to_unread(&self, since: u64) {
        let Some(index) = self
            .channel()
            .and_then(|channel| channel.first_unread_index(since, self.store.current_user_id))
        else {
            return;
        };
        self.messages.scroll_to(ListOffset {
            item_ix: index.saturating_sub(1),
            offset_in_item: px(0.),
        });
    }

    pub(crate) fn jump_to_unread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(since) = self.unread.since else {
            return;
        };
        smooth_scroll::cancel(cx);
        let loaded = self.channel().is_some_and(|channel| {
            channel
                .first_unread_index(since, self.store.current_user_id)
                .is_some_and(|index| index > 0 || !self.has_older_history(channel.id))
        });
        if loaded || !self.live {
            self.messages.set_follow_mode(FollowMode::Normal);
            self.scroll_to_unread(since);
        } else {
            self.fetch_history(self.selected_channel_id, Some(since), window, cx);
        }
        cx.notify();
    }

    pub(crate) fn jump_to_message(
        &mut self,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history.jump_return = None;
        self.go_to_message(message_id, window, cx);
    }

    pub(crate) fn open_message(
        &mut self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let guild_id = guild_id.or_else(|| {
            self.store
                .guilds
                .iter()
                .find(|guild| guild.channel(channel_id).is_some())
                .map(|guild| guild.id)
        });
        match guild_id {
            Some(guild_id) => self.select_guild_channel(guild_id, Some(channel_id), window, cx),
            None => self.open_home(window, cx),
        }
        if self.selected_channel_id != channel_id {
            self.select_channel(channel_id, window, cx);
        }
        if self.selected_channel_id == channel_id {
            self.jump_to_message(message_id, window, cx);
        }
    }

    pub(crate) fn open_channel_link(
        &mut self,
        channel_id: u64,
        message_id: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match message_id {
            Some(message_id) if self.selected_channel_id == channel_id => {
                self.jump_to_message(message_id, window, cx)
            }
            Some(message_id) => self.open_message(None, channel_id, message_id, window, cx),
            None if self.selected_channel_id == channel_id => {}
            None => {
                let guild_id = self
                    .store
                    .guilds
                    .iter()
                    .find(|guild| guild.channel(channel_id).is_some())
                    .map(|guild| guild.id);
                match guild_id {
                    Some(guild_id) => {
                        self.select_guild_channel(guild_id, Some(channel_id), window, cx)
                    }
                    None => self.select_channel(channel_id, window, cx),
                }
            }
        }
    }

    fn go_to_message(&mut self, message_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        smooth_scroll::cancel(cx);
        if let Some(index) = self.message_index(message_id) {
            self.messages.set_follow_mode(FollowMode::Normal);
            if cx.reduce_motion() {
                self.jumps.scroll = None;
                self.messages.scroll_to_reveal_item(index);
                self.highlight_jump(message_id, cx);
                cx.notify();
                return;
            }
            self.start_jump(JumpTarget::Message(message_id), window, cx);
        } else if self.live {
            self.history.pending_jump = Some(message_id);
            self.fetch_history(self.selected_channel_id, Some(message_id), window, cx);
        }
    }

    pub(super) fn list_offset(&self) -> Pixels {
        let top = self.messages.logical_scroll_top();
        self.messages.offset_for_item(top.item_ix) + top.offset_in_item
    }

    fn step_jump(&mut self, nonce: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(jump) = self
            .jumps
            .scroll
            .as_ref()
            .filter(|jump| jump.nonce == nonce)
        else {
            return;
        };
        let jump_target = jump.target;
        let viewport = self.messages.viewport_bounds().size.height;
        let content = self.messages.offset_for_item(self.messages.item_count());
        let bottom = (content - viewport).max(px(0.));
        // While following the tail the list reports its end, a viewport below what's on screen.
        let logical = self.list_offset();
        let current = logical.min(bottom);
        let target = match jump_target {
            JumpTarget::Message(message_id) => {
                let Some(index) = self.message_index(message_id) else {
                    self.jumps.scroll = None;
                    return;
                };
                let top = self.messages.offset_for_item(index);
                let height = self.messages.offset_for_item(index + 1) - top;
                let zoom = crate::theme::ZOOM_LEVELS[crate::theme::zoom_index(cx)];
                let clearance = if self.unread.bar { 32. } else { 16. };
                let clearance = px(clearance * zoom).min(viewport / 4.);
                let centered = (viewport - height) / 2. + px(8. * zoom);
                (top - centered.max(clearance)).max(px(0.)).min(bottom)
            }
            JumpTarget::Present => bottom,
            JumpTarget::Offset(offset) => offset.max(px(0.)).min(bottom),
        };
        let distance = target - current;
        let step = if distance.abs() > viewport * JUMP_GLIDE_VIEWPORTS {
            distance - viewport * distance.signum()
        } else if distance.abs() < px(1.) {
            distance
        } else {
            distance * JUMP_STEP_FRACTION
        };
        self.messages.scroll_by(current + step - logical);
        self.views.messages.update(cx, |_, cx| cx.notify());
        if distance.abs() < px(1.) {
            self.jumps.scroll = None;
            let present = self.store.at_present(self.selected_channel_id);
            match jump_target {
                JumpTarget::Message(message_id) => self.highlight_jump(message_id, cx),
                JumpTarget::Present => {
                    self.messages.set_follow_mode(FollowMode::Tail);
                    self.schedule_ack(cx);
                }
                JumpTarget::Offset(_) if present && target >= bottom => {
                    self.messages.set_follow_mode(FollowMode::Tail);
                    self.schedule_ack(cx);
                }
                JumpTarget::Offset(_) => {}
            }
            self.history.edge_check = true;
            cx.notify();
            return;
        }
        let entity = self.entity.clone();
        window.on_next_frame(move |window, cx| {
            entity
                .update(cx, |app, cx| app.step_jump(nonce, window, cx))
                .ok();
        });
    }

    fn highlight_jump(&mut self, message_id: u64, cx: &mut Context<Self>) {
        let nonce = self.jumps.highlight.map_or(0, |(_, nonce)| nonce) + 1;
        self.jumps.highlight = Some((message_id, nonce));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JUMP_HIGHLIGHT).await;
            this.update(cx, |app, _| {
                if app
                    .jumps
                    .highlight
                    .is_some_and(|(_, current)| current == nonce)
                {
                    app.jumps.highlight = None;
                }
            })
            .ok();
        })
        .detach();
    }

    fn flash_jump_missing(&mut self, cx: &mut Context<Self>) {
        self.jumps.missing = true;
        self.views.messages.update(cx, |_, cx| cx.notify());
        self.jumps.missing_nonce += 1;
        let nonce = self.jumps.missing_nonce;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JUMP_MISSING).await;
            this.update(cx, |app, cx| {
                if app.jumps.missing && app.jumps.missing_nonce == nonce {
                    app.jumps.missing = false;
                    app.views.messages.update(cx, |_, cx| cx.notify());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn fetch_history(
        &mut self,
        channel_id: u64,
        around: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if !self.history.loading.insert(channel_id) {
            return;
        }
        let limit = match around {
            Some(message_id)
                if message_id != 0 && self.history.pending_jump == Some(message_id) =>
            {
                JUMP_PAGE
            }
            Some(_) => MESSAGE_PAGE,
            None => self
                .history
                .present_limit
                .remove(&channel_id)
                .unwrap_or(MESSAGE_PAGE),
        };
        let guild_id = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.channel(channel_id).is_some())
            .map(|guild| guild.id);
        let request = Tokio::spawn(cx, async move {
            match around {
                Some(0) => {
                    session
                        .fetch_messages_after(guild_id, channel_id, 0, limit)
                        .await
                }
                Some(message_id) => {
                    session
                        .fetch_messages_around(guild_id, channel_id, message_id, limit)
                        .await
                }
                None => {
                    session
                        .fetch_messages(guild_id, channel_id, None, limit)
                        .await
                }
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                app.history.loading.remove(&channel_id);
                let presenting =
                    around.is_none() && app.history.pending_present.contains(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        app.history.failed.remove(&channel_id);
                        app.history.complete.remove(&channel_id);
                        app.history.trimmed.remove(&channel_id);
                        app.history.paused.remove(&channel_id);
                        let previous_first = (app.selected_channel_id == channel_id)
                            .then(|| app.channel())
                            .flatten()
                            .and_then(|channel| channel.oldest_history_message())
                            .map(|message| message.id);
                        match around {
                            Some(_) => app.store.set_around_messages(channel_id, messages),
                            None => {
                                if messages.len() < limit {
                                    app.history.complete.insert(channel_id);
                                }
                                app.store.set_messages(channel_id, messages);
                            }
                        }
                        if around == Some(0) {
                            app.history.complete.insert(channel_id);
                        }
                        app.request_missing_members(channel_id);
                        app.reset_thread_pane(channel_id, cx);
                        if app.selected_channel_id == channel_id {
                            let count = app.channel().map_or(0, |channel| channel.messages.len());
                            app.reset_messages(count);
                            if presenting {
                                app.glide_down_from_top(window, cx);
                            }
                            if let Some(since) = around {
                                app.messages.set_follow_mode(FollowMode::Normal);
                                if app.history.pending_jump.take() == Some(since) {
                                    if let Some(index) = app.message_index(since) {
                                        let start =
                                            match previous_first {
                                                Some(first) if !cx.reduce_motion() => {
                                                    if since > first { 0 } else { count }
                                                }
                                                _ => index,
                                            };
                                        app.messages.scroll_to(ListOffset {
                                            item_ix: start,
                                            offset_in_item: px(0.),
                                        });
                                        let entity = app.entity.clone();
                                        window.on_next_frame(move |window, cx| {
                                            entity
                                                .update(cx, |app, cx| {
                                                    app.go_to_message(since, window, cx)
                                                })
                                                .ok();
                                        });
                                    } else {
                                        app.history.jump_return = None;
                                        app.scroll_to_unread(since);
                                        app.flash_jump_missing(cx);
                                    }
                                } else if let Some(index) = app.message_index(since) {
                                    app.messages.scroll_to(ListOffset {
                                        item_ix: index,
                                        offset_in_item: px(0.),
                                    });
                                } else {
                                    app.scroll_to_unread(since);
                                }
                            }
                            if app.viewing() {
                                app.ack_channel(cx);
                            }
                        }
                        if around.is_none() {
                            app.flush_pending_sends(channel_id, window, cx);
                        }
                    }
                    Ok(Err(error)) => {
                        app.history.pending_jump = None;
                        app.history.failed.insert(channel_id);
                        if around.is_some()
                            && app
                                .history
                                .jump_return
                                .is_some_and(|(channel, _)| channel == channel_id)
                        {
                            app.history.jump_return = None;
                        }
                        log::warn!("Could not load messages: {error}");
                    }
                    Err(error) => {
                        app.history.pending_jump = None;
                        app.history.failed.insert(channel_id);
                        if around.is_some()
                            && app
                                .history
                                .jump_return
                                .is_some_and(|(channel, _)| channel == channel_id)
                        {
                            app.history.jump_return = None;
                        }
                        log::warn!("History task failed: {error}");
                    }
                }
                if app.history.pending_present.remove(&channel_id) && around.is_some() {
                    app.fetch_history(channel_id, None, window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn fetch_newer(&mut self, limit: usize, cx: &mut Context<Self>) {
        if !self.live {
            return;
        }
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(newest) = self
            .channel()
            .filter(|channel| channel.messages_loaded)
            .and_then(|channel| channel.newest_history_message())
            .map(|message| message.id)
        else {
            return;
        };
        if self.store.at_present(channel_id) || !self.history.newer.insert(channel_id) {
            return;
        }
        let guild_id = self.request_guild();
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_messages_after(guild_id, channel_id, newest, limit)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.history.newer.remove(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        app.history.failed.remove(&channel_id);
                        let current_newest = app
                            .history_channel(channel_id)
                            .and_then(|channel| channel.newest_history_message())
                            .map(|message| message.id);
                        if current_newest != Some(newest) {
                            cx.notify();
                            return;
                        }
                        let optimistic = app.channel().is_some_and(|channel| {
                            channel
                                .messages
                                .iter()
                                .any(|message| message.delivery != Delivery::Sent)
                        });
                        let inserted = app.store.append_messages(channel_id, messages, limit);
                        if inserted > 0 {
                            app.request_missing_members(channel_id);
                        }
                        if app.selected_channel_id == channel_id {
                            let len = app.channel().map_or(0, |channel| channel.messages.len());
                            let index = len - inserted;
                            if inserted > 0 {
                                app.messages.splice(index..index, inserted);
                            }
                            if optimistic {
                                app.messages.remeasure_items(0..len);
                            }
                            app.newer_loaded(index, inserted, cx);
                        }
                    }
                    Ok(Err(error)) => {
                        app.history.failed.insert(channel_id);
                        log::warn!("Could not load newer messages: {error}");
                    }
                    Err(error) => {
                        app.history.failed.insert(channel_id);
                        log::warn!("History task failed: {error}");
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn fetch_older_in(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        self.fetch_older_page(channel_id, OLDER_PAGE, cx);
    }

    fn fetch_older_page(&mut self, channel_id: u64, limit: usize, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(oldest) = self
            .history_channel(channel_id)
            .filter(|channel| channel.messages_loaded)
            .and_then(|channel| channel.oldest_history_message())
            .map(|message| message.id)
        else {
            return;
        };
        if self
            .history
            .older_retry_at
            .get(&channel_id)
            .is_some_and(|at| Instant::now() < *at)
        {
            return;
        }
        if self.history.complete.contains(&channel_id) || !self.history.older.insert(channel_id) {
            return;
        }
        let guild_id = self.request_guild();
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_messages(guild_id, channel_id, Some(oldest), limit)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.history.older.remove(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        app.history.older_retry_at.remove(&channel_id);
                        let current_oldest = app
                            .history_channel(channel_id)
                            .and_then(|channel| channel.oldest_history_message())
                            .map(|message| message.id);
                        if current_oldest != Some(oldest) {
                            cx.notify();
                            return;
                        }
                        if messages.len() < limit {
                            app.history.complete.insert(channel_id);
                        }
                        let optimistic = app.history_channel(channel_id).is_some_and(|channel| {
                            channel
                                .messages
                                .iter()
                                .any(|message| message.delivery != Delivery::Sent)
                        });
                        let inserted = app.store.prepend_messages(channel_id, messages);
                        if inserted > 0 {
                            app.prepend_thread_pane(channel_id, inserted, cx);
                            app.request_missing_members(channel_id);
                        }
                        if app.selected_channel_id == channel_id {
                            app.older_loaded(inserted, optimistic, cx);
                        }
                    }
                    Ok(Err(error)) => {
                        app.history
                            .older_retry_at
                            .insert(channel_id, Instant::now() + OLDER_RETRY_DELAY);
                        log::warn!("Could not load older messages: {error}");
                    }
                    Err(error) => {
                        app.history
                            .older_retry_at
                            .insert(channel_id, Instant::now() + OLDER_RETRY_DELAY);
                        log::warn!("History task failed: {error}");
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
