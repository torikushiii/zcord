use super::super::*;

impl DiscordApp {
    pub(crate) fn record_emoji_use(&mut self, choice: &EmojiChoice, cx: &mut Context<Self>) {
        let key = match choice {
            EmojiChoice::Unicode { emoji, .. } => emoji.clone(),
            EmojiChoice::Custom(emoji) => format!("custom:{}", emoji.id),
        };
        crate::settings::update(cx, |settings| {
            settings.recent_emojis.retain(|recent| *recent != key);
            settings.recent_emojis.insert(0, key);
            settings.recent_emojis.truncate(40);
        });
    }

    pub(crate) fn recent_emojis(&self, cx: &App) -> Vec<EmojiChoice> {
        crate::settings::get(cx)
            .recent_emojis
            .iter()
            .filter_map(|key| {
                if let Some(id) = key.strip_prefix("custom:") {
                    let id: u64 = id.parse().ok()?;
                    let emoji = self
                        .store
                        .guilds
                        .iter()
                        .flat_map(|guild| &guild.emojis)
                        .find(|emoji| emoji.id == id)?;
                    return Some(EmojiChoice::Custom(emoji.clone()));
                }
                let shortcode = emoji::shortcode_for_emoji(key)?;
                Some(EmojiChoice::Unicode {
                    emoji: key.clone(),
                    shortcode: shortcode.to_owned(),
                })
            })
            .take(40)
            .collect()
    }

    pub(crate) fn emoji_insert_text(&self, choice: &EmojiChoice) -> Option<String> {
        if !crate::settings::display().use_rich_chat_input {
            return Some(match choice {
                EmojiChoice::Unicode { shortcode, .. } => format!(":{shortcode}:"),
                EmojiChoice::Custom(custom) => format!(":{}:", custom.name),
            });
        }
        match choice {
            EmojiChoice::Unicode { emoji, .. } => Some(emoji.clone()),
            EmojiChoice::Custom(custom) => {
                (self
                    .store
                    .emoji_access(custom, self.selected_guild_id, self.selected_channel_id)
                    == EmojiAccess::Usable)
                    .then(|| composer_tokens::token_for(custom.id, &custom.name, custom.animated))
            }
        }
    }

    pub(crate) fn repair_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (text, cursor) = {
            let input = self.compose.input.read(cx);
            if !input.text().chars().any(composer_tokens::is_token_char) {
                return;
            }
            (input.value(), input.cursor())
        };
        let Some((fixed, removed)) = composer_tokens::repair_with_removed(&text) else {
            return;
        };
        let mut removed_before = 0;
        for range in &removed {
            if range.end <= cursor {
                removed_before += range.len();
            } else if range.start < cursor {
                removed_before += cursor - range.start;
            }
        }
        let cursor = cursor.saturating_sub(removed_before).min(fixed.len());
        self.compose.input.update(cx, |input, cx| {
            input.set_value(fixed, window, cx);
            let position = input.text().offset_to_position(cursor);
            input.set_cursor_position(position, window, cx);
        });
    }

    pub(crate) fn insert_emoji(
        &mut self,
        choice: &EmojiChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.emoji_insert_text(choice) else {
            return;
        };
        let text = format!("{text} ");
        self.compose.input.update(cx, |input, cx| {
            input.insert(text, window, cx);
            input.focus(window, cx);
        });
        self.record_emoji_use(choice, cx);
        cx.notify();
    }

    pub(crate) fn replace_emoji_query(
        &mut self,
        query_len: usize,
        choice: &EmojiChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.emoji_insert_text(choice) else {
            return;
        };
        self.replace_suggest_query(query_len, format!("{text} "), window, cx);
        self.record_emoji_use(choice, cx);
        cx.notify();
    }

    pub(crate) fn replace_suggest_query(
        &mut self,
        query_len: usize,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = {
            let input = self.suggest_input().read(cx);
            let value = input.text().to_string();
            let cursor = input.cursor().min(value.len());
            let start = if query_len == 0 {
                cursor
            } else {
                value[..cursor]
                    .char_indices()
                    .rev()
                    .nth(query_len - 1)
                    .map_or(0, |(offset, _)| offset)
            };
            start..cursor
        };
        self.suggest_input().clone().update(cx, |input, cx| {
            input.set_selected_range(range, cx);
            input.replace(text, window, cx);
            input.focus(window, cx);
        });
    }

    pub(crate) fn suggest_input(&self) -> &Entity<TextareaState> {
        self.compose
            .editing
            .as_ref()
            .map_or(&self.compose.input, |editing| &editing.input)
    }

    pub(crate) fn suggest_input_focused(&self, window: &Window, cx: &App) -> bool {
        self.suggest_input()
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }

    pub(crate) fn emoji_query(&self, cx: &App) -> Option<String> {
        let input = self.suggest_input().read(cx);
        let value = input.text().to_string();
        let cursor = input.cursor().min(value.len());
        let prefix = &value[..cursor];
        let mut start = cursor;
        let mut count = 0;
        for (offset, c) in prefix.char_indices().rev() {
            if !emoji_query_char(c) {
                break;
            }
            start = offset;
            count += 1;
        }
        if count < 2 {
            return None;
        }
        let (colon, _) = prefix[..start].char_indices().next_back()?;
        if prefix[colon..].chars().next()? != ':' {
            return None;
        }
        if colon > 0 {
            let before = prefix[..colon].chars().next_back()?;
            if !before.is_whitespace() {
                return None;
            }
        }
        Some(prefix[colon + 1..].to_owned())
    }

    pub(crate) fn encode_emoji_text(&self, content: &str) -> (String, Vec<EmojiChoice>) {
        let guild_id = self.selected_guild_id;
        let channel_id = self.selected_channel_id;
        let used = RefCell::new(Vec::new());
        let content = composer_tokens::to_wire(content);
        let encoded = emoji::encode_emoji(&content, |name| {
            let current = self
                .store
                .guild_by_id(guild_id)
                .and_then(|guild| guild.emojis.iter().find(|emoji| emoji.name == name))
                .filter(|emoji| {
                    self.store.emoji_access(emoji, guild_id, channel_id) == EmojiAccess::Usable
                });
            let custom = current.or_else(|| {
                self.store
                    .guilds
                    .iter()
                    .flat_map(|guild| &guild.emojis)
                    .find(|emoji| {
                        emoji.name == name
                            && self.store.emoji_access(emoji, guild_id, channel_id)
                                == EmojiAccess::Usable
                    })
            });
            if let Some(custom) = custom {
                used.borrow_mut().push(EmojiChoice::Custom(custom.clone()));
                return Some(format!(
                    "<{}:{}:{}>",
                    if custom.animated { "a" } else { "" },
                    custom.name,
                    custom.id
                ));
            }
            let value = emoji::shortcode(name)?;
            used.borrow_mut().push(EmojiChoice::Unicode {
                emoji: value.to_owned(),
                shortcode: name.to_owned(),
            });
            Some(value.to_owned())
        });
        (encoded, used.into_inner())
    }

    pub(crate) fn focus_composer(&self, window: &mut Window, cx: &mut Context<Self>) {
        let message_request = self.is_home()
            && self
                .store
                .private_channel(self.selected_channel_id)
                .is_some_and(|dm| dm.message_request);
        // Shortcuts need a mounted focus target even when there is no composer.
        if self.showing_home_page() || self.selected_forum().is_some() || message_request {
            self.focus_handle.focus(window, cx);
        } else {
            self.compose
                .input
                .update(cx, |input, cx| input.focus(window, cx));
        }
    }

    pub(crate) fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sharing_paused.take().is_some() {
            cx.notify();
            return;
        }
        if self.close_message_menu(cx)
            || self.close_dm_menu(cx)
            || self.close_dm_dialog(cx)
            || self.close_guild_menu(cx)
            || self.close_member_menu(cx)
            || self.close_thread_menu(cx)
            || self.close_create_thread(cx)
            || self.close_custom_reminder(cx)
            || self.close_schedule_edit(cx)
            || self.close_inbox_menu(cx)
            || self.close_inbox(cx)
            || self.close_channel_popout(cx)
            || self.close_search(window, cx)
            || self.close_expression_info(cx)
            || self.close_stream_menu(cx)
            || self.close_composer_menu(cx)
            || self.close_share_picker(cx)
        {
            return;
        }
        if self.close_quick_switcher(window, cx) {
            return;
        }
        if self.exit_video_fullscreen(window, cx) {
            return;
        }
        if self.dismiss_mention_suggestions(cx)
            || self.dismiss_emoji_suggestions(cx)
            || self.close_emoji_picker(cx)
            || self.close_gif_picker(cx)
        {
            return;
        }
        if self.close_lightbox_context_menu(cx)
            || self.close_lightbox(cx)
            || self.close_forward_dialog(cx)
            || self.close_reactions_dialog(cx)
        {
            return;
        }
        if self.close_account_menu(cx)
            || self.close_add_account(window, cx)
            || self.close_manage_accounts(cx)
        {
            return;
        }
        if self.dismiss_status_menu(window, cx) || self.dismiss_switch_menu(cx) {
            return;
        }
        if self
            .profile
            .as_ref()
            .is_some_and(|card| card.more_menu_open)
        {
            self.set_profile_menu(false, cx);
            return;
        }
        if self.cancel_remove_friend(cx)
            || self.finish_identity_edit(cx)
            || self.close_tag_menu(cx)
            || self.dismiss_styles_layer(cx)
            || self.close_profile(cx)
        {
            return;
        }
        if self.close_settings(cx) {
            return;
        }
        if self.compose.confirm_delete.is_some() {
            self.cancel_delete(cx);
        } else if self.compose.editing.is_some() {
            self.cancel_edit(window, cx);
        } else if self.clear_focused_staged(window, cx) {
        } else if self.compose.reply.is_some() {
            self.cancel_reply(cx);
        } else if self.thread_pane_focused(window, cx) {
            self.close_thread_pane(cx);
            self.focus_composer(window, cx);
        } else {
            self.jump_to_present(window, cx);
        }
    }

    pub(crate) fn composer_focused(&self, window: &Window, cx: &App) -> bool {
        self.compose
            .input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }

    pub(crate) fn composer_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.move_search(-1, cx) || self.move_quick_switcher(-1, cx) {
            return;
        }
        if self.suggest_input_focused(window, cx)
            && (self.move_mention_suggestion(-1, cx) || self.move_emoji_suggestion(-1, cx))
        {
            return;
        }
        let composer = self.compose.input.read(cx);
        let last_own = self.channel().and_then(|channel| {
            channel
                .messages
                .iter()
                .rev()
                .find(|message| {
                    message.author_id == self.store.current_user_id
                        && message.delivery == crate::store::Delivery::Sent
                        && matches!(message.kind, 0 | 19)
                })
                .map(|message| message.id)
        });
        match last_own {
            Some(message_id)
                if self.compose.editing.is_none()
                    && composer.value().is_empty()
                    && composer.focus_handle(cx).is_focused(window) =>
            {
                self.start_edit(message_id, window, cx);
                self.messages
                    .scroll_to_reveal_item(self.message_index(message_id).unwrap_or_default());
            }
            _ => cx.propagate(),
        }
    }
}
