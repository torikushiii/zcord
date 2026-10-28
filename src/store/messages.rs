use super::*;

impl Store {
    pub fn append_message(
        &mut self,
        guild_id: u64,
        channel_id: u64,
        content: String,
    ) -> Option<usize> {
        let channel = self
            .guilds
            .iter_mut()
            .find(|guild| guild.id == guild_id)?
            .channel_mut(channel_id)
            .filter(|channel| channel.kind.is_text())?;
        let timestamp = channel
            .messages
            .last()
            .map_or(SEED_START, |message| message.timestamp.plus_minutes(1));
        let index = channel.messages.len();
        channel.messages.push(Message {
            id: self.next_message_id,
            send_order: None,
            author_id: self.current_user_id,
            timestamp,
            delivery: Delivery::Sent,
            content,
            embeds: Vec::new(),
            components: Vec::new(),
            reactions: Vec::new(),
            attachments: Vec::new(),
            stickers: Vec::new(),
            kind: 0,
            edited: false,
            reply: None,
            forward: None,
            mentions_me: false,
        });
        self.next_message_id += 1;
        Some(index)
    }

    pub(super) fn network_channel(&self, channel_id: u64) -> Option<&Channel> {
        if let Some(location) = self.channel_locations.get(&channel_id).copied() {
            return self
                .guilds
                .get(location.guild)?
                .categories
                .get(location.category)?
                .channels
                .get(location.channel);
        }
        if let Some(location) = self.thread_locations.get(&channel_id).copied() {
            return self
                .guilds
                .get(location.guild)?
                .threads
                .get(location.thread);
        }
        let location = self.private_locations.get(&channel_id).copied()?;
        self.private
            .get(location.index)
            .map(|private| &private.channel)
    }

    pub fn at_present(&self, channel_id: u64) -> bool {
        self.network_channel(channel_id)
            .is_some_and(|channel| channel.messages_loaded)
            && (self
                .channel_locations
                .get(&channel_id)
                .is_some_and(|location| location.at_present)
                || self
                    .thread_locations
                    .get(&channel_id)
                    .is_some_and(|location| location.at_present)
                || self
                    .private_locations
                    .get(&channel_id)
                    .is_some_and(|location| location.at_present))
    }

    pub fn queue_message(
        &mut self,
        channel_id: u64,
        nonce: u64,
        content: String,
        reply_to: Option<u64>,
    ) -> Change {
        let reply = reply_to.map(|message_id| {
            match self
                .network_channel(channel_id)
                .and_then(|channel| {
                    channel
                        .messages
                        .iter()
                        .find(|message| message.id == message_id)
                })
                .map(|message| {
                    let forward = message.forward.as_ref();
                    let (content, embeds) = match forward.filter(|_| message.content.is_empty()) {
                        Some(forward) => (&forward.content, &forward.embeds),
                        None => (&message.content, &message.embeds),
                    };
                    Reply {
                        message_id: message.id,
                        author_id: message.author_id,
                        timestamp: message.timestamp,
                        content: if hides_embed_content(content, embeds) {
                            String::new()
                        } else {
                            content.clone()
                        },
                        attachments: !message.attachments.is_empty()
                            || forward.is_some_and(|forward| !forward.attachments.is_empty()),
                        embeds: !message.embeds.is_empty()
                            || !message.components.is_empty()
                            || forward.is_some_and(|forward| {
                                !forward.embeds.is_empty() || !forward.components.is_empty()
                            }),
                        sticker: message
                            .stickers
                            .first()
                            .or_else(|| {
                                if message.content.is_empty() {
                                    forward.and_then(|forward| forward.stickers.first())
                                } else {
                                    None
                                }
                            })
                            .map(|sticker| sticker.name.clone()),
                    }
                }) {
                Some(reply) => ReplyState::Loaded(reply),
                None => ReplyState::Unavailable,
            }
        });
        let author_id = self.current_user_id;
        if !self.at_present(channel_id) {
            return Change::None;
        }
        let Some(channel) = self.network_channel_mut(channel_id) else {
            return Change::None;
        };
        channel.messages.push(Message {
            id: nonce,
            send_order: Some(SendOrder {
                nonce,
                history_seen: false,
            }),
            author_id,
            timestamp: timestamp_from_nonce(nonce),
            delivery: Delivery::Pending,
            content,
            embeds: Vec::new(),
            components: Vec::new(),
            reactions: Vec::new(),
            attachments: Vec::new(),
            stickers: Vec::new(),
            kind: 0,
            edited: false,
            reply,
            forward: None,
            mentions_me: false,
        });
        order_messages(&mut channel.messages);
        let index = channel.message_index(nonce).expect("queued message");
        Change::MessageInserted { channel_id, index }
    }

    pub fn edit_message_locally(
        &mut self,
        channel_id: u64,
        message_id: u64,
        content: String,
    ) -> Change {
        let Some(channel) = self.network_channel_mut(channel_id) else {
            return Change::None;
        };
        let Some(index) = channel.message_index(message_id) else {
            return Change::None;
        };
        let message = &mut channel.messages[index];
        message.content = content;
        message.edited = true;
        Change::MessageChanged { channel_id, index }
    }

    pub fn fail_message(&mut self, channel_id: u64, nonce: u64) -> Change {
        let Some(channel) = self.network_channel_mut(channel_id) else {
            return Change::None;
        };
        let Some(index) = channel.message_index(nonce) else {
            return Change::None;
        };
        if channel.messages[index].delivery != Delivery::Pending {
            return Change::None;
        }
        channel.messages[index].delivery = Delivery::Failed;
        Change::MessageChanged { channel_id, index }
    }

    pub fn discard_message(&mut self, channel_id: u64, nonce: u64) -> Change {
        let Some(channel) = self.network_channel_mut(channel_id) else {
            return Change::None;
        };
        let Some(index) = channel.message_index(nonce) else {
            return Change::None;
        };
        if channel.messages[index].delivery == Delivery::Sent {
            return Change::None;
        }
        channel.messages.remove(index);
        order_messages(&mut channel.messages);
        Change::MessageRemoved { channel_id, index }
    }

    pub(super) fn network_channel_mut(&mut self, channel_id: u64) -> Option<&mut Channel> {
        if let Some(location) = self.channel_locations.get(&channel_id).copied() {
            return self
                .guilds
                .get_mut(location.guild)?
                .categories
                .get_mut(location.category)?
                .channels
                .get_mut(location.channel);
        }
        if let Some(location) = self.thread_locations.get(&channel_id).copied() {
            return self
                .guilds
                .get_mut(location.guild)?
                .threads
                .get_mut(location.thread);
        }
        let location = self.private_locations.get(&channel_id).copied()?;
        self.private
            .get_mut(location.index)
            .map(|private| &mut private.channel)
    }

    fn replace_messages(&mut self, channel_id: u64, mut messages: Vec<model::Message>) {
        messages.retain(|m| m.channel_id == channel_id);
        messages.sort_by_key(|m| m.id);
        messages.dedup_by_key(|m| m.id);
        let previous: Vec<_> = self
            .network_channel(channel_id)
            .map(|channel| channel.messages.iter().map(|message| message.id).collect())
            .unwrap_or_default();
        if !self
            .network_channel(channel_id)
            .is_some_and(Channel::has_outbox)
        {
            self.clear_message_info(channel_id);
        }
        let mapped: Vec<_> = messages
            .iter()
            .filter_map(|message| self.map_network_message(message, true))
            .collect();
        let removed: Vec<_> = if let Some(channel) = self.network_channel_mut(channel_id) {
            merge_history(channel, &messages, mapped, true);
            channel.messages_loaded = true;
            previous
                .into_iter()
                .filter(|id| channel.message_index(*id).is_none())
                .collect()
        } else {
            previous
        };
        for id in removed {
            self.forget_message_info(channel_id, id);
        }
    }

    pub fn set_messages(&mut self, channel_id: u64, messages: Vec<model::Message>) {
        self.replace_messages(channel_id, messages);
        self.set_at_present(channel_id, true);
    }

    pub fn set_around_messages(&mut self, channel_id: u64, mut messages: Vec<model::Message>) {
        messages.retain(|m| m.channel_id == channel_id);
        let newest = messages.iter().map(|message| message.id).max().unwrap_or(0);
        let present = newest >= self.latest_message_id(channel_id);
        self.replace_messages(channel_id, messages);
        self.set_at_present(channel_id, present);
    }

    fn set_at_present(&mut self, channel_id: u64, present: bool) {
        if let Some(location) = self.channel_locations.get_mut(&channel_id) {
            location.at_present = present;
        } else if let Some(location) = self.thread_locations.get_mut(&channel_id) {
            location.at_present = present;
        } else if let Some(location) = self.private_locations.get_mut(&channel_id) {
            location.at_present = present;
        }
    }

    pub fn prepend_messages(
        &mut self,
        channel_id: u64,
        mut messages: Vec<model::Message>,
    ) -> usize {
        let oldest = self
            .network_channel(channel_id)
            .and_then(Channel::oldest_history_message)
            .map_or(u64::MAX, |message| message.id);
        messages.retain(|m| m.channel_id == channel_id && m.id < oldest);
        messages.sort_by_key(|m| m.id);
        messages.dedup_by_key(|m| m.id);
        let mapped: Vec<_> = messages
            .iter()
            .filter_map(|message| self.map_network_message(message, true))
            .collect();
        self.network_channel_mut(channel_id).map_or(0, |channel| {
            merge_history(channel, &messages, mapped, false)
        })
    }

    pub fn append_messages(
        &mut self,
        channel_id: u64,
        mut messages: Vec<model::Message>,
        limit: usize,
    ) -> usize {
        let complete = messages.len() < limit;
        let newest = self
            .network_channel(channel_id)
            .and_then(Channel::newest_history_message)
            .map_or(0, |message| message.id);
        messages.retain(|m| m.channel_id == channel_id && m.id > newest);
        messages.sort_by_key(|m| m.id);
        messages.dedup_by_key(|m| m.id);
        let mapped: Vec<_> = messages
            .iter()
            .filter_map(|message| self.map_network_message(message, true))
            .collect();
        let count = self.network_channel_mut(channel_id).map_or(0, |channel| {
            merge_history(channel, &messages, mapped, false)
        });
        let latest = self.latest_message_id(channel_id);
        let reached = self
            .network_channel(channel_id)
            .and_then(Channel::newest_history_message)
            .is_some_and(|message| message.id >= latest);
        if complete || reached {
            self.set_at_present(channel_id, true);
        }
        count
    }

    pub(super) fn message_create(&mut self, message: Box<model::Message>) -> Change {
        let mut loaded = false;
        let present = self
            .channel_locations
            .get(&message.channel_id)
            .map(|location| location.at_present)
            .or_else(|| {
                self.private_locations
                    .get(&message.channel_id)
                    .map(|location| location.at_present)
            })
            .unwrap_or(true);
        if let Some(channel) = self.network_channel_mut(message.channel_id) {
            loaded = channel.messages_loaded;
            if channel.message_index(message.id).is_some() {
                return Change::None;
            }
        }
        let mapped = self.map_network_message(&message, loaded);
        let mentions_me = self.message_mentions_me(&message);
        let previous = self.latest_message_id(message.channel_id);
        let mentions = self.mention_count(message.channel_id);
        if message.id > previous {
            self.read_state_mut(message.channel_id, previous);
        }
        if let Some(source) = &mut self.source {
            if let Some(channel) = self
                .source_channels
                .get(&message.channel_id)
                .and_then(|i| i.get_mut(source))
            {
                channel.last_message_id = Some(Some(previous.max(message.id)));
            }
        }
        self.refresh_request_preview(&message, message.id >= previous);
        self.update_forum_post_message(&message);
        let current_user_id = self.current_user_id;
        let own = message.author.id == current_user_id;
        if mentions_me && !own {
            self.increment_mentions(message.channel_id, message.id);
        }
        self.refresh_badges(message.channel_id, mentions);
        if own
            && self
                .last_read(message.channel_id)
                .is_none_or(|last| message.id > last)
        {
            self.apply_ack(message.channel_id, message.id);
        }
        let Some(channel) = self.network_channel_mut(message.channel_id) else {
            return Change::None;
        };
        if !channel.messages_loaded {
            return Change::Structure;
        }
        let Some(mut mapped) = mapped else {
            return Change::None;
        };
        let pending = if own {
            message.nonce.and_then(|nonce| {
                channel.messages.iter().position(|m| {
                    m.id == nonce && m.delivery != Delivery::Sent && m.author_id == current_user_id
                })
            })
        } else {
            None
        };
        if !present && pending.is_none() {
            self.forget_message_info(message.channel_id, message.id);
            return Change::Structure;
        }
        if let Some(removed) = pending {
            mapped.send_order = channel.messages.remove(removed).send_order;
            channel.messages.push(mapped);
            order_messages(&mut channel.messages);
            let inserted = channel
                .message_index(message.id)
                .expect("confirmed message");
            return Change::MessageReplaced {
                channel_id: message.channel_id,
                removed,
                inserted,
            };
        }
        channel.messages.push(mapped);
        order_messages(&mut channel.messages);
        let index = channel.message_index(message.id).expect("created message");
        Change::MessageInserted {
            channel_id: message.channel_id,
            index,
        }
    }

    pub(super) fn message_update(&mut self, update: Box<model::MessageUpdate>) -> Change {
        let (reply, mentions_me, embeds) = self.update_message_info(&update);
        let Some(channel) = self.network_channel_mut(update.channel_id) else {
            return Change::None;
        };
        let Some(index) = channel.message_index(update.id) else {
            return Change::None;
        };
        let message = &mut channel.messages[index];
        if let Some(kind) = update.kind {
            message.kind = kind;
        }
        if let Some(edited) = &update.edited_timestamp {
            message.edited = edited.is_some();
        }
        message.reply = reply;
        message.mentions_me = mentions_me;
        if let Some(content) = update.content {
            message.content = content;
        }
        if let Some(author) = update.author {
            message.author_id = author.id;
        }
        if let Some(timestamp) = update.timestamp.as_deref().and_then(local_timestamp) {
            message.timestamp = timestamp;
        }
        if let Some(embeds) = embeds {
            message.embeds = embeds;
        }
        if let Some(components) = update.components {
            message.components = map_components(&components);
        }
        if let Some(reactions) = update.reactions {
            message.reactions = map_reactions(&reactions);
        }
        if let Some(attachments) = update.attachments {
            message.attachments = attachments;
        }
        if let Some(stickers) = update.sticker_items {
            message.stickers = map_stickers(&stickers);
        }
        if let Some(snapshots) = &update.message_snapshots {
            let reference = update.message_reference.as_ref().and_then(Option::as_ref);
            let previous = message.forward.as_ref();
            let channel_id = reference
                .and_then(|reference| reference.channel_id)
                .or_else(|| previous.and_then(|forward| forward.channel_id));
            let guild_id = reference
                .and_then(|reference| reference.guild_id)
                .or_else(|| previous.and_then(|forward| forward.guild_id));
            message.forward = snapshots
                .first()
                .map(|snapshot| map_snapshot(&snapshot.message, channel_id, guild_id));
        }
        Change::MessageChanged {
            channel_id: update.channel_id,
            index,
        }
    }

    pub(super) fn message_delete(&mut self, channel_id: u64, id: u64) -> Change {
        self.forget_message_info(channel_id, id);
        let Some(channel) = self.network_channel_mut(channel_id) else {
            return Change::None;
        };
        let Some(index) = channel.message_index(id) else {
            return Change::None;
        };
        channel.messages.remove(index);
        order_messages(&mut channel.messages);
        Change::MessageRemoved { channel_id, index }
    }
}

fn merge_history(
    channel: &mut Channel,
    wire: &[model::Message],
    mut mapped: Vec<Message>,
    replace: bool,
) -> usize {
    let old = std::mem::take(&mut channel.messages);
    let old_len = old.len();
    let outstanding = old.iter().any(|message| message.delivery != Delivery::Sent);
    for mut message in old {
        let replacement = wire
            .iter()
            .find(|wire| {
                wire.id == message.id
                    || (message.delivery != Delivery::Sent
                        && wire.author.id == message.author_id
                        && wire.nonce == Some(message.id))
            })
            .and_then(|wire| mapped.iter_mut().find(|mapped| mapped.id == wire.id));
        if let Some(replacement) = replacement {
            // Only a full history reload clears `send_order`; a page that loaded first must
            // keep it so placeholders stay in place.
            replacement.send_order =
                if replace && message.delivery == Delivery::Sent && !outstanding {
                    None
                } else {
                    message.send_order.map(|order| SendOrder {
                        history_seen: true,
                        ..order
                    })
                };
        } else if !replace || message.delivery != Delivery::Sent || message.send_order.is_some() {
            if replace && let Some(order) = &mut message.send_order {
                order.history_seen = false;
            }
            mapped.push(message);
        }
    }
    order_messages(&mut mapped);
    let inserted = mapped.len().saturating_sub(old_len);
    channel.messages = mapped;
    inserted
}

/// Keep the server chronology intact and anchor placeholders to their next confirmed
/// local successor. Acknowledgements can arrive out of order without swapping local sends.
fn order_messages(messages: &mut Vec<Message>) {
    if messages
        .iter()
        .all(|message| message.delivery == Delivery::Sent)
    {
        messages.sort_by_key(|message| message.id);
        return;
    }
    let (mut sent, mut pending): (Vec<_>, Vec<_>) = std::mem::take(messages)
        .into_iter()
        .partition(|message| message.delivery == Delivery::Sent);
    sent.sort_by_key(|message| message.id);
    pending.sort_by_key(|message| message.id);
    for message in pending {
        let index = sent
            .iter()
            .position(|successor| {
                successor.delivery == Delivery::Sent
                    && successor
                        .send_order
                        .as_ref()
                        .is_some_and(|order| order.nonce > message.id)
            })
            .unwrap_or(sent.len());
        sent.insert(index, message);
    }
    *messages = sent;
}
