use super::*;

impl DiscordApp {
    pub(super) fn handle_event(
        &mut self,
        event: Event,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            Event::Fatal(error) => {
                let message: SharedString = match error {
                    FatalError::InvalidToken => {
                        self.forget_current_account(cx);
                        "Invalid token".into()
                    }
                    FatalError::Other(message) => message.into(),
                };
                self.restart(
                    Start::Login {
                        error: Some(message),
                    },
                    window,
                    cx,
                );
            }
            Event::Voice(event) => self.handle_voice_event(event, cx),
            Event::Reconnecting => {
                self.self_status.disconnect();
                self.reconnecting = true;
                cx.notify();
            }
            Event::Resumed => {
                self.reconnecting = false;
                if let Some(status) = self.selected_self_status() {
                    self.store.set_self_status(status);
                }
                self.arm_status_expiry_timer(cx);
                self.refresh_voice_start_times();
                self.member_subscription = None;
                self.typing.clear();
                cx.notify();
            }
            Event::Ready(ref ready) => {
                let user = ready.user.clone();
                let switched = self.account_ready(&user, cx);
                self.live = true;
                self.reconnecting = false;
                self.typing.clear();
                self.thread_previews.clear();
                self.forget_trimmed();
                let reconnect = matches!(self.screen, Screen::Chat);
                self.member_subscription = None;
                self.store.apply(event);
                if let Some(status) = self.selected_self_status() {
                    self.store.set_self_status(status);
                }
                self.update_game_activity();
                self.refresh_voice_start_times();
                self.arm_status_expiry_timer(cx);
                self.load_user_affinities(cx);
                self.refresh_activity_feed(true, cx);
                self.load_saved_messages(cx);
                self.sync_channel_rows();
                self.screen = Screen::Chat;
                if switched {
                    self.open_home(window, cx);
                } else if reconnect && self.channel().is_some() {
                    let channel = self.selected_channel_id;
                    let thread_pane = self.thread_pane.take();
                    self.select_channel(channel, window, cx);
                    self.restore_thread_pane(thread_pane, window, cx);
                } else {
                    self.select_initial(window, cx);
                }
            }
            event => {
                match &event {
                    Event::TypingStart(typing) => {
                        self.typing_started(typing.channel_id, typing.user_id, cx)
                    }
                    Event::MessageCreate(message) => {
                        self.trimmed_message_created(message.channel_id);
                        self.notify_message(message, cx);
                        self.typing_stopped(message.channel_id, message.author.id, cx)
                    }
                    Event::MessageUpdate(update) => {
                        self.trimmed_message_changed(update.channel_id, update.id)
                    }
                    Event::MessageReactionAdd(event)
                    | Event::MessageReactionRemove(event)
                    | Event::MessageReactionRemoveAll(event)
                    | Event::MessageReactionRemoveEmoji(event) => {
                        self.trimmed_message_changed(event.channel_id, event.message_id)
                    }
                    Event::MessageReactionAddMany(event) => {
                        self.trimmed_message_changed(event.channel_id, event.message_id)
                    }
                    Event::MessageDelete { channel_id, id } => {
                        self.trimmed_message_deleted(*channel_id, *id);
                        self.forget_message(*id, cx)
                    }
                    Event::CallDelete { channel_id }
                        if self.voice.joined_call() == Some((0, *channel_id)) =>
                    {
                        self.leave_voice(cx);
                    }
                    _ => {}
                }
                let supplemental = matches!(event, Event::ReadySupplemental(_));
                let status_update = match &event {
                    Event::SettingsUpdate(update) => update.settings.kind == 1,
                    Event::SessionsReplace(_) | Event::ReadySupplemental(_) => true,
                    Event::PresenceUpdate(presence) => {
                        presence
                            .user_id
                            .or_else(|| presence.user.as_ref().map(|user| user.id))
                            == Some(self.store.current_user_id)
                    }
                    _ => false,
                };
                let reacted = match &event {
                    Event::MessageReactionAdd(event) | Event::MessageReactionRemove(event) => {
                        Some(event.message_id)
                    }
                    Event::MessageReactionRemoveAll(event) => Some(event.message_id),
                    Event::MessageReactionRemoveEmoji(event) => Some(event.message_id),
                    Event::MessageReactionAddMany(event) => Some(event.message_id),
                    _ => None,
                };
                let announced_call = match &event {
                    Event::CallCreate(call) | Event::CallUpdate(call) => Some(call.channel_id),
                    _ => None,
                };
                let relationship = matches!(
                    event,
                    Event::RelationshipAdd(_)
                        | Event::RelationshipUpdate(_)
                        | Event::RelationshipRemove { .. }
                );
                let change = self.store.apply(event);
                if relationship {
                    self.remeasure_message_lists();
                }
                if let Some(channel_id) = announced_call {
                    self.ring_pending_dm_call(channel_id, cx);
                }
                if let Some(message_id) = reacted {
                    self.message_reactions_changed(message_id, cx);
                }
                if status_update && let Some(status) = self.selected_self_status() {
                    self.store.set_self_status(status);
                }
                if status_update {
                    self.update_game_activity();
                }
                self.apply_change(change, window, cx);
                if supplemental {
                    self.restore_voice(cx);
                }
            }
        }
    }
    pub(super) fn apply_change(
        &mut self,
        change: Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reconcile_dm_call(cx);
        self.arm_status_expiry_timer(cx);
        self.follow_account_theme(cx);
        crate::settings::set_display(cx, self.text_images());
        self.load_activity_reply_assets(cx);
        self.sync_thread_pane(&change, cx);
        if is_background_change(&change, self.selected_channel_id) {
            self.arm_members_timer(cx);
            return;
        }
        let len = self.channel().map_or(0, |channel| channel.messages.len());
        let placeholders = self.channel().is_some_and(|channel| {
            channel
                .messages
                .iter()
                .any(|message| message.delivery != crate::store::Delivery::Sent)
        });
        match change {
            Change::None => return,
            Change::VoiceTimers => {
                self.views.channel_list.update(cx, |_, cx| cx.notify());
                return;
            }
            Change::MessageInserted { channel_id, index }
                if channel_id == self.selected_channel_id =>
            {
                self.messages.splice(index..index, 1);
                self.messages.remeasure_items(if placeholders {
                    0..len
                } else {
                    index.saturating_sub(1)..(index + 2).min(len)
                });
                let own = self
                    .channel()
                    .and_then(|channel| channel.messages.get(index))
                    .is_some_and(|message| message.author_id == self.store.current_user_id);
                let dropped = self.window_after_insert();
                let index = index
                    .saturating_sub(dropped)
                    .min(self.channel().map_or(0, |channel| channel.messages.len()));
                if self.unread.since.is_none() && !own && !self.viewing() {
                    self.unread.since =
                        self.channel()
                            .and_then(|channel| {
                                channel.messages[..index].iter().rev().find(|message| {
                                    message.delivery == crate::store::Delivery::Sent
                                })
                            })
                            .map(|message| message.id);
                    self.unread.bar = self.unread.since.is_some();
                }
                self.schedule_ack(cx);
            }
            Change::MessageReplaced {
                channel_id,
                removed,
                inserted,
            } if channel_id == self.selected_channel_id => {
                self.messages.splice(removed..removed + 1, 0);
                self.messages.splice(inserted..inserted, 1);
                self.messages.remeasure_items(if placeholders {
                    0..len
                } else {
                    removed.min(inserted).saturating_sub(1)..(removed.max(inserted) + 2).min(len)
                });
            }
            Change::MessageChanged { channel_id, index }
                if channel_id == self.selected_channel_id =>
            {
                self.messages.remeasure_items(index..(index + 2).min(len));
            }
            Change::MessageRemoved { channel_id, index }
                if channel_id == self.selected_channel_id =>
            {
                self.stop_player(Some(window), cx);
                self.messages.splice(index..index + 1, 0);
                self.messages.remeasure_items(if placeholders {
                    0..len
                } else {
                    index.saturating_sub(1)..(index + 1).min(len)
                });
            }
            Change::Saved => self.saved_changed(cx),
            Change::Members { guild_id } => {
                if guild_id.is_none_or(|id| id == self.selected_guild_id) {
                    self.arm_members_timer(cx);
                    if self.mention_suggestions(cx).is_some() {
                        cx.notify();
                    }
                }
                return;
            }
            Change::Structure => {
                self.sync_channel_rows();
                self.sync_member_rows();
                if self.channel().is_none() && !self.showing_home_page() {
                    if self.is_home() {
                        self.open_friends(window, cx);
                    } else if self.store.guild_by_id(self.selected_guild_id).is_none() {
                        self.open_home(window, cx);
                    } else {
                        self.select_first_guild(window, cx);
                    }
                    return;
                }
                if self.messages.item_count() != len {
                    self.messages.reset(len);
                }
            }
            _ => {}
        }
        cx.notify();
    }
    pub(super) fn sync_channel_rows(&mut self) {
        self.request_activity_members();
        let guild = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.id == self.selected_guild_id);
        let banner = guild.and_then(|guild| self.store.guild_banner(guild.id));
        let selected = self.thread_parent().unwrap_or(self.selected_channel_id);
        let opened = self.selected_channel_id;
        let guild_id = self.selected_guild_id;
        let store = &self.store;
        let joined = self
            .voice
            .call
            .as_ref()
            .filter(|call| call.guild_id == guild_id)
            .map(|call| call.channel_id);
        self.channel_rows.sync_with_banner(
            guild,
            banner,
            selected,
            |channel_id| store.voice_members(guild_id, channel_id).len(),
            |channel_id| store.sidebar_threads(channel_id, opened).len(),
            joined,
        );
        self.request_voice_members();
    }
}
