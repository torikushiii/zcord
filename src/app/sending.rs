use super::attachments::{StagedFile, remove_pasted};
use super::*;
use crate::discord::UploadFile;
use crate::store::{PendingUpload, Sticker};
use std::sync::Arc;

#[cfg(test)]
mod tests;

type PendingEntry = (
    u64,
    String,
    Option<ReplyTarget>,
    Vec<StagedFile>,
    Option<Sticker>,
    Option<Arc<serde_json::Value>>,
);

#[derive(Default)]
pub(super) struct PendingSends {
    entries: Vec<PendingEntry>,
}

impl PendingSends {
    fn should_stage(&self, channel_id: u64, at_present: bool, refreshing: bool) -> bool {
        !at_present || refreshing || self.has_channel(channel_id)
    }

    pub(super) fn has_channel(&self, channel_id: u64) -> bool {
        self.entries.iter().any(|entry| entry.0 == channel_id)
    }

    fn push(
        &mut self,
        channel_id: u64,
        content: String,
        reply: Option<ReplyTarget>,
        files: Vec<StagedFile>,
        entry: Option<Arc<serde_json::Value>>,
    ) {
        self.entries
            .push((channel_id, content, reply, files, None, entry));
    }

    fn push_sticker(&mut self, channel_id: u64, reply: Option<ReplyTarget>, sticker: Sticker) {
        self.entries.push((
            channel_id,
            String::new(),
            reply,
            Vec::new(),
            Some(sticker),
            None,
        ));
    }

    fn take_channel(&mut self, channel_id: u64) -> Vec<PendingEntry> {
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
                            return;
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
                // Its users are fetched again only once Discord has the change, so they include
                // or drop you; a queued follow-up change does this when it lands instead.
                app.reaction_changed(message_id, &emoji, cx);
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
        for (channel, content, reply, files, sticker, entry) in
            self.pending_sends.take_channel(channel_id)
        {
            match sticker {
                Some(sticker) => self.submit_sticker(channel, sticker, reply, window, cx),
                None => self.dispatch_send(channel, content, reply, files, entry, window, cx),
            }
        }
    }

    pub(super) fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Convert first: EM SPACE (the start of a token) is whitespace to `trim`.
        let content = composer_tokens::to_wire(&self.composer.read(cx).value());
        let content = content.trim().to_owned();
        let channel_id = self.selected_channel_id;
        let has_files = !self.staged_files().is_empty();
        if (content.is_empty() && !has_files) || !self.store.can_send(channel_id) {
            return;
        }
        let content = mentions::encode_mentions(&content, &self.mention_names);
        let (content, used_emoji) = self.encode_emoji_text(&content);
        if content.chars().count() > MESSAGE_LIMIT {
            return;
        }
        for choice in &used_emoji {
            self.record_emoji_use(choice, cx);
        }
        self.reset_mention_draft();
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        let files = self.staged.remove(&channel_id).unwrap_or_default();
        self.send_content(content, files, window, cx);
    }

    pub(crate) fn send_content(
        &mut self,
        content: String,
        files: Vec<StagedFile>,
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
        self.manual_unread = None;
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
        self.submit_send(channel_id, content, reply, files, None, window, cx);
    }

    pub(crate) fn send_sticker(
        &mut self,
        channel_id: u64,
        sticker_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.store.can_send(channel_id) {
            return;
        }
        let Some(sticker) = self.store.sticker(sticker_id) else {
            return;
        };
        let reply = self.reply.take();
        self.typing_sent = None;
        self.unread_since = None;
        self.unread_bar = false;
        self.manual_unread = None;
        self.messages.set_follow_mode(FollowMode::Tail);
        if !self.live {
            let guild_id = self
                .store
                .guilds
                .iter()
                .find(|guild| guild.channel(channel_id).is_some())
                .map(|guild| guild.id);
            if let Some(index) = guild_id
                .and_then(|guild_id| self.store.append_sticker(guild_id, channel_id, sticker))
            {
                let previous = index.saturating_sub(1);
                self.messages.splice(previous..index, index - previous + 1);
                self.messages.scroll_to_end();
            }
            cx.notify();
            return;
        }
        self.submit_sticker(channel_id, sticker, reply, window, cx);
    }

    pub(super) fn submit_send(
        &mut self,
        channel_id: u64,
        content: String,
        reply: Option<ReplyTarget>,
        files: Vec<StagedFile>,
        entry: Option<Arc<serde_json::Value>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.store.can_send(channel_id) {
            return;
        }
        if self.pending_sends.should_stage(
            channel_id,
            self.store.at_present(channel_id),
            self.loading.contains(&channel_id),
        ) {
            self.pending_sends
                .push(channel_id, content, reply, files, entry);
            self.pending_present.insert(channel_id);
            smooth_scroll::cancel(cx);
            // Refresh even if another response already marked the channel present. A later
            // submission retries a failed refresh.
            self.fetch_history(channel_id, None, window, cx);
        } else {
            self.dispatch_send(channel_id, content, reply, files, entry, window, cx);
        }
    }

    fn dispatch_send(
        &mut self,
        channel_id: u64,
        content: String,
        reply: Option<ReplyTarget>,
        files: Vec<StagedFile>,
        entry: Option<Arc<serde_json::Value>>,
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
        let uploads = files
            .iter()
            .map(|file| PendingUpload {
                name: file.name.clone(),
                size: file.size,
            })
            .collect();
        let change = self.store.queue_message(
            channel_id,
            nonce,
            content.clone(),
            reply.as_ref().map(|reply| reply.message_id),
            uploads,
        );
        self.apply_change(change, window, cx);
        let retry_entry = entry.clone();
        let outgoing = OutgoingMessage {
            content,
            nonce,
            reply: reply.as_ref().map(|reply| OutgoingReply {
                message_id: reply.message_id,
                mention: reply.mention,
            }),
            attachments: Vec::new(),
            sticker_ids: Vec::new(),
            content_inventory_entry: entry,
        };
        let request = if files.is_empty() {
            Tokio::spawn(cx, session.send_message(guild_id, channel_id, outgoing))
        } else {
            let total = files.iter().map(|file| file.size).sum();
            let upload_files = files
                .iter()
                .map(|file| UploadFile {
                    path: file.path.clone(),
                    filename: file.name.clone(),
                    size: file.size,
                    content_type: file.content_type.clone(),
                    spoiler: file.spoiler,
                })
                .collect();
            let progress = UploadProgress::new(total);
            self.upload_progress.insert(nonce, progress.clone());
            self.start_upload_ticker(cx);
            Tokio::spawn(
                cx,
                session.send_with_files(guild_id, channel_id, outgoing, upload_files, progress),
            )
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                app.upload_progress.remove(&nonce);
                let error = match result {
                    Ok(Ok(message)) => {
                        remove_pasted(&files, cx);
                        let change = app.store.apply(Event::MessageCreate(Box::new(message)));
                        app.apply_change(change, window, cx);
                        return;
                    }
                    Ok(Err(error)) => error,
                    Err(_) => discord::Error::Closed,
                };
                if matches!(error, discord::Error::Cancelled) {
                    let change = app.store.discard_message(channel_id, nonce);
                    app.apply_change(change, window, cx);
                    app.staged.entry(channel_id).or_default().extend(files);
                    cx.notify();
                    return;
                }
                log::warn!("Could not send message: {error}");
                app.failed_sends.insert(
                    nonce,
                    FailedSend {
                        reply,
                        files,
                        entry: retry_entry,
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

    fn submit_sticker(
        &mut self,
        channel_id: u64,
        sticker: Sticker,
        reply: Option<ReplyTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_sends.should_stage(
            channel_id,
            self.store.at_present(channel_id),
            self.loading.contains(&channel_id),
        ) {
            self.pending_sends.push_sticker(channel_id, reply, sticker);
            self.pending_present.insert(channel_id);
            smooth_scroll::cancel(cx);
            self.fetch_history(channel_id, None, window, cx);
        } else {
            self.dispatch_sticker(channel_id, sticker, reply, window, cx);
        }
    }

    fn dispatch_sticker(
        &mut self,
        channel_id: u64,
        sticker: Sticker,
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
        let sticker_id = sticker.id;
        let change = self.store.queue_sticker(
            channel_id,
            nonce,
            reply.as_ref().map(|reply| reply.message_id),
            sticker,
        );
        self.apply_change(change, window, cx);
        let outgoing = OutgoingMessage {
            content: String::new(),
            nonce,
            reply: reply.as_ref().map(|reply| OutgoingReply {
                message_id: reply.message_id,
                mention: reply.mention,
            }),
            attachments: Vec::new(),
            sticker_ids: vec![sticker_id],
            content_inventory_entry: None,
        };
        let request = Tokio::spawn(cx, session.send_message(guild_id, channel_id, outgoing));
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
                if matches!(error, discord::Error::Cancelled) {
                    let change = app.store.discard_message(channel_id, nonce);
                    app.apply_change(change, window, cx);
                    cx.notify();
                    return;
                }
                log::warn!("Could not send sticker: {error}");
                app.failed_sends.insert(
                    nonce,
                    FailedSend {
                        reply,
                        files: Vec::new(),
                        entry: None,
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

    fn start_upload_ticker(&mut self, cx: &mut Context<Self>) {
        if self.upload_ticking {
            return;
        }
        self.upload_ticking = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let running = this
                    .update(cx, |app, cx| {
                        if app.upload_progress.is_empty() {
                            app.upload_ticking = false;
                            false
                        } else {
                            cx.notify();
                            true
                        }
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
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
        let Some((content, sticker)) = self
            .channel()
            .and_then(|channel| channel.messages.iter().find(|message| message.id == nonce))
            .map(|message| (message.content.clone(), message.stickers.first().cloned()))
        else {
            return;
        };
        let failed = self.failed_sends.remove(&nonce);
        let reply = failed.as_ref().and_then(|failed| failed.reply.clone());
        let entry = failed.as_ref().and_then(|failed| failed.entry.clone());
        let files = failed.map_or_else(Vec::new, |failed| failed.files);
        let change = self.store.discard_message(channel_id, nonce);
        self.apply_change(change, window, cx);
        self.messages.set_follow_mode(FollowMode::Tail);
        if let Some(sticker) = sticker {
            self.submit_sticker(channel_id, sticker, reply, window, cx);
        } else {
            self.submit_send(channel_id, content, reply, files, entry, window, cx);
        }
    }

    pub(crate) fn discard_message(
        &mut self,
        nonce: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(failed) = self.failed_sends.remove(&nonce) {
            remove_pasted(&failed.files, cx);
        }
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
        let (content, mentions) = mentions::decode_mentions(&content, |id| {
            let user = self.store.user(id)?;
            Some(
                self.request_guild()
                    .and_then(|guild| self.store.member_name(guild, id))
                    .unwrap_or(&user.name)
                    .to_owned(),
            )
        });
        if let Some(previous) = self.editing.take() {
            self.remeasure_message(previous.message_id);
        }
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 20)
                .submit_on_enter(true)
                .context_menu(false)
        });
        input.update(cx, |input, cx| set_text(input, content, window, cx));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                move |app, _, event, window, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => {
                        if !(app.accept_mention_suggestion(None, window, cx)
                            || app.accept_emoji_suggestion(None, window, cx))
                        {
                            app.save_edit(window, cx);
                        }
                    }
                    InputEvent::Change => {
                        app.remeasure_message(message_id);
                        app.reset_emoji_suggest(cx);
                        app.reset_mention_suggest(cx);
                    }
                    _ => {}
                },
            );
        let cursor = cx.observe(&input, |app, _, cx| app.reset_mention_suggest(cx));
        self.editing = Some(Editing {
            message_id,
            input,
            mentions,
            _subscriptions: [subscription, cursor],
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
        let content = mentions::encode_mentions(&raw, &editing.mentions);
        let content = composer_tokens::to_wire(&content);
        let content = content.trim().to_owned();
        if content.chars().count() > MESSAGE_LIMIT {
            self.editing = Some(editing);
            return;
        }
        self.remeasure_message(message_id);
        self.focus_composer(window, cx);
        cx.notify();
        if content.is_empty() {
            self.request_delete(message_id, false, window, cx);
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
            // The composer usually has focus; move it to the dialog so Enter confirms
            // the delete instead of sending the composed text.
            self.delete_focus.focus(window, cx);
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
        self.message_reactions_removed(message_id);
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
