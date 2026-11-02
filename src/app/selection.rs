use super::*;

impl DiscordApp {
    pub(super) fn select_initial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = crate::settings::get(cx).last_channel;
        if saved.is_some_and(|saved| saved.guild_id == 0) {
            self.open_home(window, cx);
            return;
        }
        let valid = saved.is_some_and(|saved| {
            self.store.guilds.iter().any(|guild| {
                guild.id == saved.guild_id
                    && guild
                        .channel(saved.channel_id)
                        .is_some_and(|channel| channel.kind.is_selectable())
            })
        });
        match saved.filter(|_| valid) {
            Some(saved) => {
                self.select_guild_channel(saved.guild_id, Some(saved.channel_id), window, cx)
            }
            None => self.select_first_guild(window, cx),
        }
    }

    pub(super) fn select_first_guild(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.store.guilds.first().map(|guild| guild.id) {
            Some(id) => self.select_guild(id, window, cx),
            None => {
                self.selected_guild_id = 0;
                self.selected_channel_id = 0;
                self.sync_channel_rows();
                self.messages.reset(0);
                cx.notify();
            }
        }
    }

    pub(crate) fn select_guild(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let saved = crate::settings::get(cx).guild_channels.get(&id).copied();
        self.select_guild_channel(id, saved, window, cx);
    }

    pub(super) fn select_guild_channel(
        &mut self,
        id: u64,
        preferred: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(guild) = self.store.guilds.iter().find(|guild| guild.id == id) else {
            return;
        };
        let channel_id = preferred
            .and_then(|channel_id| guild.channel(channel_id))
            .filter(|channel| channel.kind.is_selectable())
            .or_else(|| guild.first_text_channel())
            .or_else(|| guild.first_selectable_channel())
            .map(|channel| channel.id);
        let Some(channel_id) = channel_id else {
            return;
        };
        if self.selected_guild_id != id {
            if self.hovered_rail != Some(self.selected_guild_id) {
                self.rail_morphs.start(self.selected_guild_id, false);
            }
            self.member_rows.reset(0);
            self.member_layout = member_list::Layout::default();
            self.member_top = 0;
        }
        self.selected_guild_id = id;
        self.sync_channel_rows();
        self.sync_member_rows();
        if !self.live {
            self.store.mark_guild_read(id);
        }
        self.select_channel(channel_id, window, cx);
    }

    pub(crate) fn select_channel(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.profile = None;
        self.message_menu = None;
        self.channel_popout = None;
        self.thread_menu = None;
        self.thread_pane = None;
        let found = match self.guild() {
            Some(guild) => guild.channel(id).map(|channel| {
                (
                    channel.name.clone(),
                    channel.kind,
                    channel.messages.len(),
                    channel.messages_loaded,
                    channel.parent_id,
                    guild.threads.iter().any(|thread| thread.id == id),
                )
            }),
            None => self.store.private_channel(id).map(|dm| {
                (
                    self.store.private_channel_name(id),
                    dm.channel.kind,
                    dm.channel.messages.len(),
                    dm.channel.messages_loaded,
                    None,
                    false,
                )
            }),
        };
        let Some((name, kind, count, messages_loaded, parent_id, is_thread)) = found else {
            return;
        };
        let stream_chat = kind == crate::store::ChannelKind::Voice && self.stream_chat_open();
        if !kind.is_selectable() && !stream_chat {
            return;
        }
        if self.voice.joined_call().map(|(_, channel_id)| channel_id) != Some(id) {
            self.dismiss_call_view();
        }
        let forum = kind.is_forum();
        let needs_history = self.live && !forum && !messages_loaded;
        let placeholder = if forum {
            "Select a post".to_owned()
        } else if self.store.can_send(id) {
            if self.is_home() {
                home::placeholder(&self.store, id, &name)
            } else if is_thread {
                format!("Message {name}")
            } else {
                format!("Message #{name}")
            }
        } else {
            "You do not have permission to send messages in this channel.".to_owned()
        };
        let saved_id = if is_thread {
            parent_id.unwrap_or(id)
        } else {
            id
        };

        let previous = self.selected_channel_id;
        let draft = self.composer.read(cx).value().to_string();
        if draft.is_empty() {
            self.drafts.remove(&previous);
        } else {
            self.drafts.insert(previous, draft);
        }
        let restored = self.drafts.remove(&id).unwrap_or_default();

        if previous != id {
            self.expression_info.clear_open();
            self.reply = None;
            self.editing = None;
            self.confirm_delete = None;
            self.typing_sent = None;
            self.revealed_spoilers.clear();
            self.stop_player(Some(window), cx);
            self.close_lightbox(cx);
        }
        self.selected_channel_id = id;
        self.load_dm_profile(cx);
        self.pending_jump = None;
        self.jump_missing = false;
        self.jump_missing_nonce += 1;
        self.manual_unread = None;
        self.recent_channels.visit(id);
        self.sync_channel_rows();
        if forum {
            self.unread_since = None;
            self.unread_bar = false;
        } else {
            self.unread_since = self
                .store
                .channel_ack(id)
                .map(|_| self.store.last_read(id).unwrap_or(0));
            self.unread_bar = self.unread_since.is_some();
        }
        let unread_since = self.unread_since;
        if !self.live {
            self.store.mark_channel_read(self.selected_guild_id, id);
        } else {
            let guild_id = self.selected_guild_id;
            crate::settings::update(cx, |settings| {
                settings.last_channel = Some(crate::settings::SavedChannel {
                    guild_id,
                    channel_id: saved_id,
                });
                settings.guild_channels.insert(guild_id, saved_id);
            });
        }
        self.reset_messages(if forum { 0 } else { count });
        self.composer.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            set_text(input, restored, window, cx);
        });
        self.focus_composer(window, cx);
        if forum {
            self.load_forum(id, cx);
            cx.notify();
            return;
        }
        if self.viewing() {
            self.ack_channel(cx);
        }
        if needs_history {
            self.fetch_history(id, unread_since, window, cx);
        } else if self.live
            && let Some(since) = unread_since
        {
            let (oldest, inside) = self.channel().map_or((None, false), |channel| {
                (
                    channel.oldest_history_message().map(|message| message.id),
                    channel
                        .newest_history_message()
                        .is_some_and(|message| message.id > since),
                )
            });
            if !inside || oldest.is_some_and(|oldest| since < oldest) {
                self.fetch_history(id, Some(since), window, cx);
            } else {
                self.messages.set_follow_mode(FollowMode::Normal);
                self.scroll_to_unread(since);
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_category(&mut self, id: u64, cx: &mut Context<Self>) {
        if id == 0 {
            return;
        }
        let collapsed = self.channel_rows.toggle_collapsed(id);
        crate::settings::update(cx, |settings| {
            if collapsed {
                settings.collapsed_categories.insert(id);
            } else {
                settings.collapsed_categories.remove(&id);
            }
        });
        self.sync_channel_rows();
        cx.notify();
        if collapsed {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(channel_list::FOLD).await;
                this.update(cx, |app, cx| {
                    app.sync_channel_rows();
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    pub(crate) fn is_folder_open(&self, id: u64) -> bool {
        self.open_folders.contains(&id)
    }

    pub(crate) fn toggle_folder(&mut self, id: u64, cx: &mut Context<Self>) {
        let opening = !self.open_folders.remove(&id);
        if opening {
            self.open_folders.insert(id);
        }
        if !cx.reduce_motion() {
            self.folder_folds.start(id, opening);
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(guild_rail::FOLDER_FOLD)
                    .await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }
        cx.notify();
    }

    pub(crate) fn set_rail_hover(&mut self, key: u64, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(key)
        } else if self.hovered_rail == Some(key) {
            None
        } else {
            return;
        };
        if self.hovered_rail != next {
            let previous = std::mem::replace(&mut self.hovered_rail, next);
            for key in [previous, next].into_iter().flatten() {
                if key != self.selected_guild_id {
                    self.rail_morphs.start(key, next == Some(key));
                }
            }
            cx.notify();
        }
    }
}
