use super::*;
use crate::store::Delivery;

impl DiscordApp {
    pub(super) fn reset_messages(&mut self, count: usize) {
        self.messages.reset(count);
        self.messages.set_follow_mode(FollowMode::Tail);
    }

    pub(crate) fn jump_to_present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Chat) {
            return;
        }
        smooth_scroll::cancel(cx);
        self.unread_since = None;
        self.unread_bar = false;
        let channel_id = self.selected_channel_id;
        let needs_present = self.live
            && (self.pending_sends.has_channel(channel_id)
                || self.channel().is_some_and(|channel| {
                    !channel.messages_loaded || !self.store.at_present(channel.id)
                }));
        self.messages.set_follow_mode(FollowMode::Tail);
        if needs_present {
            self.pending_present.insert(channel_id);
            self.fetch_history(channel_id, None, window, cx);
        } else {
            self.mark_channel_read(cx);
        }
    }

    pub(super) fn scroll_to_unread(&self, since: u64) {
        let Some(index) = self
            .channel()
            .and_then(|channel| channel.first_unread_index(since))
        else {
            return;
        };
        self.messages.scroll_to(ListOffset {
            item_ix: index.saturating_sub(1),
            offset_in_item: px(0.),
        });
    }

    pub(crate) fn jump_to_unread(&mut self, cx: &mut Context<Self>) {
        let Some(since) = self.unread_since else {
            return;
        };
        smooth_scroll::cancel(cx);
        self.scroll_to_unread(since);
        cx.notify();
    }

    pub(crate) fn jump_to_message(
        &mut self,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        smooth_scroll::cancel(cx);
        if let Some(index) = self.message_index(message_id) {
            self.messages.set_follow_mode(FollowMode::Normal);
            if cx.reduce_motion() {
                self.messages.scroll_to_reveal_item(index);
                self.highlight_jump(message_id, cx);
                cx.notify();
                return;
            }
            let nonce = self.jump.as_ref().map_or(0, |jump| jump.nonce) + 1;
            self.jump = Some(JumpScroll { message_id, nonce });
            self.step_jump(nonce, window, cx);
        } else if self.live {
            self.fetch_history(self.selected_channel_id, Some(message_id), window, cx);
        }
    }

    fn list_offset(&self) -> Pixels {
        let top = self.messages.logical_scroll_top();
        self.messages.offset_for_item(top.item_ix) + top.offset_in_item
    }

    /// One frame of the eased scroll toward a jump target, centring it like Discord. The
    /// target is recomputed every frame because rows measured on the way change its offset;
    /// a far target is first approached by a jump to within a viewport.
    fn step_jump(&mut self, nonce: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(jump) = self.jump.as_ref().filter(|jump| jump.nonce == nonce) else {
            return;
        };
        let message_id = jump.message_id;
        let Some(index) = self.message_index(message_id) else {
            self.jump = None;
            return;
        };
        let current = self.list_offset();
        let viewport = self.messages.viewport_bounds().size.height;
        let top = self.messages.offset_for_item(index);
        let height = self.messages.offset_for_item(index + 1) - top;
        // Rows taller than the viewport start at its top rather than being centred off-screen.
        let target = (top - ((viewport - height) / 2.).max(px(16.).min(viewport / 4.))).max(px(0.));
        let distance = target - current;
        let step = if distance.abs() > viewport * JUMP_GLIDE_VIEWPORTS {
            distance - viewport * distance.signum()
        } else if distance.abs() < px(1.) {
            distance
        } else {
            distance * JUMP_STEP_FRACTION
        };
        self.messages.scroll_by(step);
        self.views.messages.update(cx, |_, cx| cx.notify());
        if distance.abs() < px(1.) {
            self.jump = None;
            self.highlight_jump(message_id, cx);
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
        let nonce = self.jump_highlight.map_or(0, |(_, nonce)| nonce) + 1;
        self.jump_highlight = Some((message_id, nonce));
        // Clear it once faded, so scrolling the row back into view doesn't replay the flash.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JUMP_HIGHLIGHT).await;
            this.update(cx, |app, _| {
                if app
                    .jump_highlight
                    .is_some_and(|(_, current)| current == nonce)
                {
                    app.jump_highlight = None;
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
        if !self.loading.insert(channel_id) {
            return;
        }
        let guild_id = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.channel(channel_id).is_some())
            .map(|guild| guild.id);
        let request = Tokio::spawn(cx, async move {
            match around {
                Some(message_id) => {
                    session
                        .fetch_messages_around(guild_id, channel_id, message_id, MESSAGE_PAGE)
                        .await
                }
                None => {
                    session
                        .fetch_messages(guild_id, channel_id, None, MESSAGE_PAGE)
                        .await
                }
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                app.loading.remove(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        app.history_complete.remove(&channel_id);
                        match around {
                            Some(_) => app.store.set_around_messages(channel_id, messages),
                            None => {
                                if messages.len() < MESSAGE_PAGE {
                                    app.history_complete.insert(channel_id);
                                }
                                app.store.set_messages(channel_id, messages);
                            }
                        }
                        app.request_missing_members(channel_id);
                        if app.selected_channel_id == channel_id {
                            let count = app.channel().map_or(0, |channel| channel.messages.len());
                            app.reset_messages(count);
                            if let Some(since) = around {
                                app.messages.set_follow_mode(FollowMode::Normal);
                                if let Some(index) = app.message_index(since) {
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
                    Ok(Err(error)) => log::warn!("Could not load messages: {error}"),
                    Err(error) => log::warn!("History task failed: {error}"),
                }
                if app.pending_present.remove(&channel_id) && around.is_some() {
                    app.fetch_history(channel_id, None, window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn fetch_newer(&mut self, cx: &mut Context<Self>) {
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
        if self.store.at_present(channel_id) || !self.loading_newer.insert(channel_id) {
            return;
        }
        let guild_id = self.request_guild();
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_messages_after(guild_id, channel_id, newest, MESSAGE_PAGE)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.loading_newer.remove(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        let current_newest = app
                            .store
                            .guilds
                            .iter()
                            .find(|guild| guild.channel(channel_id).is_some())
                            .and_then(|guild| guild.channel(channel_id))
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
                        let inserted =
                            app.store
                                .append_messages(channel_id, messages, MESSAGE_PAGE);
                        if inserted > 0 {
                            app.request_missing_members(channel_id);
                            if app.selected_channel_id == channel_id {
                                let len = app.channel().map_or(0, |channel| channel.messages.len());
                                let index = len - inserted;
                                app.messages.splice(index..index, inserted);
                                app.messages.remeasure_items(if optimistic {
                                    0..len
                                } else {
                                    index.saturating_sub(1)..(index + 1).min(len)
                                });
                            }
                        }
                        // A page may confirm an optimistic row without increasing the count.
                        if optimistic && inserted == 0 && app.selected_channel_id == channel_id {
                            let len = app.channel().map_or(0, |channel| channel.messages.len());
                            app.messages.remeasure_items(0..len);
                        }
                    }
                    Ok(Err(error)) => log::warn!("Could not load newer messages: {error}"),
                    Err(error) => log::warn!("History task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn fetch_older(&mut self, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(oldest) = self
            .channel()
            .filter(|channel| channel.messages_loaded)
            .and_then(|channel| channel.oldest_history_message())
            .map(|message| message.id)
        else {
            return;
        };
        if self
            .older_retry_at
            .get(&channel_id)
            .is_some_and(|at| Instant::now() < *at)
        {
            return;
        }
        if self.history_complete.contains(&channel_id) || !self.loading_older.insert(channel_id) {
            return;
        }
        let guild_id = self.request_guild();
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_messages(guild_id, channel_id, Some(oldest), OLDER_PAGE)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.loading_older.remove(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        app.older_retry_at.remove(&channel_id);
                        let current_oldest = app
                            .store
                            .guilds
                            .iter()
                            .find(|guild| guild.channel(channel_id).is_some())
                            .and_then(|guild| guild.channel(channel_id))
                            .and_then(|channel| channel.oldest_history_message())
                            .map(|message| message.id);
                        if current_oldest != Some(oldest) {
                            cx.notify();
                            return;
                        }
                        let complete = messages.len() < OLDER_PAGE;
                        if complete {
                            app.history_complete.insert(channel_id);
                        }
                        let optimistic = app.channel().is_some_and(|channel| {
                            channel
                                .messages
                                .iter()
                                .any(|message| message.delivery != Delivery::Sent)
                        });
                        let inserted = app.store.prepend_messages(channel_id, messages);
                        if complete && inserted == 0 && app.selected_channel_id == channel_id {
                            app.messages.remeasure_items(0..1);
                        }
                        if inserted > 0 {
                            app.request_missing_members(channel_id);
                            if app.selected_channel_id == channel_id {
                                app.messages.splice(0..0, inserted);
                                let len = app.channel().map_or(0, |channel| channel.messages.len());
                                app.messages.remeasure_items(if optimistic {
                                    0..len
                                } else {
                                    inserted..inserted + 1
                                });
                            }
                        }
                        if optimistic && inserted == 0 && app.selected_channel_id == channel_id {
                            let len = app.channel().map_or(0, |channel| channel.messages.len());
                            app.messages.remeasure_items(0..len);
                        }
                    }
                    Ok(Err(error)) => {
                        app.older_retry_at
                            .insert(channel_id, Instant::now() + OLDER_RETRY_DELAY);
                        log::warn!("Could not load older messages: {error}");
                    }
                    Err(error) => {
                        app.older_retry_at
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
