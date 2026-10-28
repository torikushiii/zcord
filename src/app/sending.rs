use super::*;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct PendingSends {
    entries: Vec<(u64, String, Option<ReplyTarget>)>,
}

impl PendingSends {
    fn should_stage(&self, channel_id: u64, at_present: bool, refreshing: bool) -> bool {
        !at_present || refreshing || self.has_channel(channel_id)
    }

    pub(super) fn has_channel(&self, channel_id: u64) -> bool {
        self.entries.iter().any(|entry| entry.0 == channel_id)
    }

    fn push(&mut self, channel_id: u64, content: String, reply: Option<ReplyTarget>) {
        self.entries.push((channel_id, content, reply));
    }

    fn take_channel(&mut self, channel_id: u64) -> Vec<(u64, String, Option<ReplyTarget>)> {
        let (ready, waiting) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|entry| entry.0 == channel_id);
        self.entries = waiting;
        ready
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
}

impl DiscordApp {
    pub(crate) fn toggle_reaction(
        &mut self,
        message_id: u64,
        emoji: ReactionEmoji,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.selected_channel_id;
        let me = self
            .store
            .reaction_state(channel_id, message_id, &emoji)
            .unwrap_or(false);
        self.set_reaction(message_id, emoji, !me, cx);
    }

    pub(crate) fn react(&mut self, message_id: u64, choice: &EmojiChoice, cx: &mut Context<Self>) {
        let emoji = match choice {
            EmojiChoice::Custom(custom) => {
                if self
                    .store
                    .emoji_access(custom, self.selected_guild_id, self.selected_channel_id)
                    != EmojiAccess::Usable
                {
                    return;
                }
                ReactionEmoji {
                    id: Some(custom.id),
                    name: custom.name.clone(),
                    animated: custom.animated,
                }
            }
            EmojiChoice::Unicode { emoji, .. } => ReactionEmoji {
                id: None,
                name: emoji.clone(),
                animated: false,
            },
        };
        let channel_id = self.selected_channel_id;
        let already = self
            .store
            .reaction_state(channel_id, message_id, &emoji)
            .unwrap_or(false);
        if !already {
            self.set_reaction(message_id, emoji, true, cx);
        }
        self.record_emoji_use(choice, cx);
    }

    fn set_reaction(
        &mut self,
        message_id: u64,
        emoji: ReactionEmoji,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.selected_channel_id;
        if self.store.set_reaction(channel_id, message_id, &emoji, add) == Change::None {
            return;
        }
        self.remeasure_message(message_id);
        cx.notify();
        if self.session.is_none() || !self.live {
            return;
        }
        if self
            .reaction_requests
            .contains(&(channel_id, message_id, reaction_emoji_text(&emoji)))
        {
            return;
        }
        let guild_id = self.request_guild();
        self.send_reaction(guild_id, channel_id, message_id, emoji, add, cx);
    }

    fn send_reaction(
        &mut self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: ReactionEmoji,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let text = reaction_emoji_text(&emoji);
        self.reaction_requests
            .insert((channel_id, message_id, text.clone()));
        let request = Tokio::spawn(cx, async move {
            if add {
                session
                    .add_reaction(guild_id, channel_id, message_id, &text)
                    .await
            } else {
                session
                    .remove_reaction(guild_id, channel_id, message_id, &text)
                    .await
            }
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.reaction_requests.remove(&(
                    channel_id,
                    message_id,
                    reaction_emoji_text(&emoji),
                ));
                match result {
                    Ok(Ok(())) => {
                        let desired = app
                            .store
                            .reaction_state(channel_id, message_id, &emoji)
                            .unwrap_or(false);
                        if desired != add && app.live && app.session.is_some() {
                            app.send_reaction(guild_id, channel_id, message_id, emoji, desired, cx);
                        }
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not update reaction: {error}");
                        if app.store.set_reaction(channel_id, message_id, &emoji, !add)
                            != Change::None
                        {
                            app.remeasure_message(message_id);
                            cx.notify();
                        }
                    }
                    Err(error) => log::warn!("Reaction task failed: {error}"),
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn flush_pending_sends(
        &mut self,
        channel_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (channel, content, reply) in self.pending_sends.take_channel(channel_id) {
            self.dispatch_send(channel, content, reply, window, cx);
        }
    }

    pub(super) fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Convert first: EM SPACE (the start of a token) is whitespace to `trim`.
        let content = composer_tokens::to_wire(&self.composer.read(cx).value());
        let content = content.trim().to_owned();
        let channel_id = self.selected_channel_id;
        if content.is_empty() || !self.store.can_send(channel_id) {
            return;
        }
        let (content, used_emoji) = self.encode_emoji_text(&content);
        if content.chars().count() > MESSAGE_LIMIT {
            return;
        }
        for choice in &used_emoji {
            self.record_emoji_use(choice, cx);
        }
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.send_content(content, window, cx);
    }

    pub(crate) fn send_content(
        &mut self,
        content: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.selected_channel_id;
        if !self.store.can_send(channel_id) {
            return;
        }
        let reply = self.reply.take();
        self.typing_sent = None;
        self.unread_since = None;
        self.unread_bar = false;
        if !self.live {
            self.messages.set_follow_mode(FollowMode::Tail);
            if let Some(index) =
                self.store
                    .append_message(self.selected_guild_id, channel_id, content)
            {
                let previous = index.saturating_sub(1);
                self.messages.splice(previous..index, index - previous + 1);
                self.messages.scroll_to_end();
            }
            cx.notify();
            return;
        }
        self.messages.set_follow_mode(FollowMode::Tail);
        self.submit_send(channel_id, content, reply, window, cx);
    }

    fn submit_send(
        &mut self,
        channel_id: u64,
        content: String,
        reply: Option<ReplyTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_sends.should_stage(
            channel_id,
            self.store.at_present(channel_id),
            self.loading.contains(&channel_id),
        ) {
            self.pending_sends.push(channel_id, content, reply);
            self.pending_present.insert(channel_id);
            smooth_scroll::cancel(cx);
            // Refresh even if another response already marked the channel present. A later
            // submission retries a failed refresh.
            self.fetch_history(channel_id, None, window, cx);
        } else {
            self.dispatch_send(channel_id, content, reply, window, cx);
        }
    }

    fn dispatch_send(
        &mut self,
        channel_id: u64,
        content: String,
        reply: Option<ReplyTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let guild_id = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.channel(channel_id).is_some())
            .map(|guild| guild.id);
        self.last_nonce = now_snowflake().max(self.last_nonce + 1);
        let nonce = self.last_nonce;
        let change = self.store.queue_message(
            channel_id,
            nonce,
            content.clone(),
            reply.as_ref().map(|reply| reply.message_id),
        );
        self.apply_change(change, window, cx);
        let outgoing = OutgoingMessage {
            content,
            nonce,
            reply: reply.as_ref().map(|reply| OutgoingReply {
                message_id: reply.message_id,
                mention: reply.mention,
            }),
        };
        let send = session.send_message(guild_id, channel_id, outgoing);
        let request = Tokio::spawn(cx, send);
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                let error = match result {
                    Ok(Ok(message)) => {
                        let change = app.store.apply(Event::MessageCreate(Box::new(message)));
                        app.apply_change(change, window, cx);
                        return;
                    }
                    Ok(Err(error)) => error,
                    Err(_) => discord::Error::Closed,
                };
                log::warn!("Could not send message: {error}");
                app.failed_sends.insert(
                    nonce,
                    FailedSend {
                        reply,
                        error: send_error_text(&error),
                    },
                );
                let change = app.store.fail_message(channel_id, nonce);
                app.apply_change(change, window, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn retry_message(
        &mut self,
        nonce: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.selected_channel_id;
        let Some(content) = self
            .channel()
            .and_then(|channel| channel.messages.iter().find(|message| message.id == nonce))
            .map(|message| message.content.clone())
        else {
            return;
        };
        let reply = self
            .failed_sends
            .remove(&nonce)
            .and_then(|failed| failed.reply);
        let change = self.store.discard_message(channel_id, nonce);
        self.apply_change(change, window, cx);
        self.messages.set_follow_mode(FollowMode::Tail);
        self.submit_send(channel_id, content, reply, window, cx);
    }

    pub(crate) fn discard_message(
        &mut self,
        nonce: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.failed_sends.remove(&nonce);
        let change = self.store.discard_message(self.selected_channel_id, nonce);
        self.apply_change(change, window, cx);
    }

    pub(crate) fn start_reply(
        &mut self,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(message) = self
            .message_index(message_id)
            .and_then(|index| self.channel()?.messages.get(index))
        else {
            return;
        };
        self.reply = Some(ReplyTarget {
            message_id,
            author_id: message.author_id,
            webhook_name: message
                .webhook
                .as_ref()
                .map(|author| author.name.clone().into()),
            mention: true,
        });
        self.focus_composer(window, cx);
        cx.notify();
    }

    pub(crate) fn toggle_reply_mention(&mut self, cx: &mut Context<Self>) {
        if let Some(reply) = &mut self.reply {
            reply.mention = !reply.mention;
            cx.notify();
        }
    }

    pub(crate) fn cancel_reply(&mut self, cx: &mut Context<Self>) {
        if self.reply.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn start_edit(
        &mut self,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(content) = self
            .message_index(message_id)
            .and_then(|index| self.channel()?.messages.get(index))
            .filter(|message| message.author_id == self.store.current_user_id)
            .map(|message| composer_tokens::from_wire(&message.content))
        else {
            return;
        };
        if let Some(previous) = self.editing.take() {
            self.remeasure_message(previous.message_id);
        }
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 20)
                .submit_on_enter(true)
        });
        input.update(cx, |input, cx| set_text(input, content, window, cx));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                move |app, _, event, window, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => app.save_edit(window, cx),
                    InputEvent::Change => app.remeasure_message(message_id),
                    _ => {}
                },
            );
        self.editing = Some(Editing {
            message_id,
            input,
            _subscription: subscription,
        });
        self.remeasure_message(message_id);
        cx.notify();
    }

    pub(crate) fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editing) = self.editing.take() {
            self.remeasure_message(editing.message_id);
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        let message_id = editing.message_id;
        // Convert first: EM SPACE (the start of a token) is whitespace to `trim`.
        let raw = editing.input.read(cx).value().to_string();
        let raw = composer_tokens::repair(&raw).unwrap_or(raw);
        let content = composer_tokens::to_wire(&raw);
        let content = content.trim().to_owned();
        if content.chars().count() > MESSAGE_LIMIT {
            self.editing = Some(editing);
            return;
        }
        self.remeasure_message(message_id);
        self.focus_composer(window, cx);
        cx.notify();
        if content.is_empty() {
            self.confirm_delete = Some(message_id);
            return;
        }
        let channel_id = self.selected_channel_id;
        let Some(original) = self
            .message_index(message_id)
            .and_then(|index| self.channel()?.messages.get(index))
            .map(|message| message.content.clone())
            .filter(|original| *original != content)
        else {
            return;
        };
        let change = self
            .store
            .edit_message_locally(channel_id, message_id, content.clone());
        self.apply_change(change, window, cx);
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let guild_id = self.request_guild();
        let request = Tokio::spawn(cx, async move {
            session
                .edit_message(guild_id, channel_id, message_id, content)
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                let change = match result {
                    Ok(Ok(update)) => app.store.apply(Event::MessageUpdate(Box::new(update))),
                    Ok(Err(error)) => {
                        log::warn!("Could not edit message: {error}");
                        app.store
                            .edit_message_locally(channel_id, message_id, original)
                    }
                    Err(_) => return,
                };
                app.apply_change(change, window, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn request_delete(
        &mut self,
        message_id: u64,
        confirmed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if confirmed {
            self.delete_message(message_id, window, cx);
        } else {
            self.confirm_delete = Some(message_id);
            self.delete_closing = None;
            cx.notify();
        }
    }

    pub(crate) fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        if self.confirm_delete.is_none() || self.delete_closing.is_some() {
            return;
        }
        self.delete_closing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DIALOG_EXIT).await;
            this.update(cx, |app, cx| {
                app.confirm_delete = None;
                app.delete_closing = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub(crate) fn delete_dialog_closing(&self) -> bool {
        self.delete_closing.is_some()
    }

    fn delete_message(&mut self, message_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_delete(cx);
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            let change = self.store.apply(Event::MessageDelete {
                channel_id,
                id: message_id,
            });
            self.forget_message(message_id, cx);
            self.apply_change(change, window, cx);
            return;
        };
        let guild_id = self.request_guild();
        let request = Tokio::spawn(cx, async move {
            session
                .delete_message(guild_id, channel_id, message_id)
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| match result {
                Ok(Ok(())) => {
                    app.forget_message(message_id, cx);
                    let change = app.store.apply(Event::MessageDelete {
                        channel_id,
                        id: message_id,
                    });
                    app.apply_change(change, window, cx);
                }
                Ok(Err(error)) => log::warn!("Could not delete message: {error}"),
                Err(_) => {}
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn forget_message(&mut self, message_id: u64, cx: &mut Context<Self>) {
        if self
            .reply
            .as_ref()
            .is_some_and(|reply| reply.message_id == message_id)
        {
            self.reply = None;
            cx.notify();
        }
        if self
            .editing
            .as_ref()
            .is_some_and(|editing| editing.message_id == message_id)
        {
            self.editing = None;
            cx.notify();
        }
        if self.confirm_delete == Some(message_id) {
            self.confirm_delete = None;
            cx.notify();
        }
    }
}
