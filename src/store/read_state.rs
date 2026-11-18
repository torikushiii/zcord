use super::*;

pub struct UnreadChannel {
    pub guild_id: u64,
    pub channel_id: u64,
    pub last_read: u64,
}

impl Store {
    pub fn inbox_unreads(&self, now_ms: u64) -> Vec<UnreadChannel> {
        let Some(source) = &self.source else {
            return Vec::new();
        };
        let mut unreads: Vec<_> = source
            .guilds
            .iter()
            .filter(|guild| guild.unavailable != Some(true) && !self.guild_muted(guild.id, now_ms))
            .flat_map(|guild| {
                guild.channels.iter().flatten().filter_map(move |channel| {
                    if !matches!(channel.kind, Some(0 | 5)) {
                        return None;
                    }
                    let state = read_state(source, &self.read_states, channel.id);
                    let muted = self.channel_muted(guild.id, channel.id, now_ms)
                        || channel
                            .parent_id
                            .flatten()
                            .is_some_and(|parent| self.channel_muted(guild.id, parent, now_ms));
                    (channel_unread(channel, state)
                        && !muted
                        && discord::can_view(guild, channel, source.user.id))
                    .then(|| {
                        let latest = channel.last_message_id.flatten().unwrap_or(0);
                        let unread = UnreadChannel {
                            guild_id: guild.id,
                            channel_id: channel.id,
                            last_read: state.and_then(|state| state.last_message_id).unwrap_or(0),
                        };
                        (latest, unread)
                    })
                })
            })
            .collect();
        unreads.sort_by_key(|(latest, _)| std::cmp::Reverse(*latest));
        unreads.into_iter().map(|(_, unread)| unread).collect()
    }

    pub fn mark_guild_read(&mut self, guild_id: u64) {
        let Some(index) = self.guilds.iter().position(|guild| guild.id == guild_id) else {
            return;
        };
        let guild = &mut self.guilds[index];
        guild.unread = false;
        guild.mentions = 0;
        for channel in guild
            .categories
            .iter_mut()
            .flat_map(|category| &mut category.channels)
        {
            channel.mentions = 0;
            channel.new_posts = 0;
        }
        self.guild_badges[index].mentions = 0;
    }

    pub fn mark_channel_read(&mut self, guild_id: u64, channel_id: u64) {
        if self.private_locations.contains_key(&channel_id) {
            self.mark_private_read(channel_id);
            return;
        }
        if let Some(location) = self.channel_locations.get(&channel_id).copied() {
            let guild = &mut self.guilds[location.guild];
            let channel = &mut guild.categories[location.category].channels[location.channel];
            if guild.id == guild_id {
                let badges = &mut self.guild_badges[location.guild];
                if channel.unread {
                    badges.unread_channels = badges.unread_channels.saturating_sub(1);
                }
                if channel.mentions > 0 {
                    badges.mentions = badges.mentions.saturating_sub(u64::from(channel.mentions));
                    channel.mentions = 0;
                }
                channel.new_posts = 0;
                channel.unread = false;
                guild.mentions = badges.mentions.min(u64::from(u32::MAX)) as u32;
            }
        }
    }

    pub fn last_read(&self, channel_id: u64) -> Option<u64> {
        let source = self.source.as_ref()?;
        read_state(source, &self.read_states, channel_id)?.last_message_id
    }

    pub fn mark_unread(&mut self, channel_id: u64, message_id: u64) -> Option<(u64, u32)> {
        let previous = self.last_read(channel_id)?;
        if message_id == 0 || message_id > previous {
            return None;
        }
        let cursor =
            self.network_channel(channel_id)
                .and_then(|channel| {
                    channel.messages.iter().rev().find(|message| {
                        message.delivery == Delivery::Sent && message.id < message_id
                    })
                })
                .map_or(message_id - 1, |message| message.id);
        let private = self.private_locations.contains_key(&channel_id);
        let mut count = if private {
            0
        } else {
            self.mention_count(channel_id)
        };
        if let Some(channel) = self.network_channel(channel_id) {
            for message in &channel.messages {
                if message.delivery != Delivery::Sent
                    || message.id < message_id
                    || message.author_id == self.current_user_id
                {
                    continue;
                }
                if private || (message.mentions_me && message.id <= previous) {
                    count = count.saturating_add(1);
                }
            }
        }
        let previous_mentions = self.mention_count(channel_id);
        self.apply_ack(channel_id, cursor);
        if let Some(state) = self.read_state_mut(channel_id, cursor) {
            state.mention_count = count;
        }
        self.refresh_badges(channel_id, previous_mentions);
        Some((cursor, count))
    }

    pub fn channel_ack(&self, channel_id: u64) -> Option<u64> {
        let source = self.source.as_ref()?;
        let channel = self.source_channels.get(&channel_id)?.get(source)?;
        let unread = channel_unread(channel, read_state(source, &self.read_states, channel_id))
            || (matches!(channel.kind, Some(15 | 16)) && self.forum_new_post_count(channel_id) > 0);
        if !unread {
            return None;
        }
        if matches!(channel.kind, Some(15 | 16)) {
            self.latest_forum_post_id(channel_id)
                .or_else(|| channel.last_message_id.flatten())
        } else {
            channel.last_message_id.flatten()
        }
    }

    pub fn unread_acks(&self, guild_id: u64, now_snowflake: u64) -> Vec<(u64, u64, u8)> {
        let Some(source) = &self.source else {
            return Vec::new();
        };
        let Some(guild) = source
            .guilds
            .iter()
            .find(|guild| guild.id == guild_id && guild.unavailable != Some(true))
        else {
            return Vec::new();
        };
        let user_id = source.user.id;
        let unread = |channel: &model::Channel| {
            let state = read_state(source, &self.read_states, channel.id);
            channel_unread(channel, state) || state.is_some_and(|state| state.mention_count > 0)
        };
        let channels = guild
            .channels
            .iter()
            .flatten()
            .filter(|channel| discord::can_view(guild, channel, user_id))
            .filter_map(|channel| {
                let message_id = match channel.kind {
                    Some(0 | 2 | 5 | 13) if unread(channel) => channel.last_message_id.flatten()?,
                    Some(15 | 16) => now_snowflake,
                    _ => return None,
                };
                Some((channel.id, message_id, 0))
            });
        let threads = guild
            .threads
            .iter()
            .flatten()
            .filter(|thread| thread.member.is_some() && unread(thread))
            .filter_map(|thread| Some((thread.id, thread.last_message_id.flatten()?, 0)));
        channels
            .chain(threads)
            .chain(guild_event_ack(source, guild))
            .collect()
    }

    pub fn mention_count(&self, channel_id: u64) -> u32 {
        self.source
            .as_ref()
            .and_then(|source| read_state(source, &self.read_states, channel_id))
            .map_or(0, |s| s.mention_count)
    }

    pub fn latest_message_id(&self, channel_id: u64) -> u64 {
        self.source
            .as_ref()
            .and_then(|source| self.source_channels.get(&channel_id)?.get(source))
            .and_then(|c| c.last_message_id.flatten())
            .unwrap_or(0)
    }

    pub(super) fn read_state_mut(
        &mut self,
        channel_id: u64,
        last_message_id: u64,
    ) -> Option<&mut model::ReadState> {
        let source = self.source.as_mut()?;
        let index = *self.read_states.entry(channel_id).or_insert_with(|| {
            let index = source.read_state.entries.len();
            source.read_state.entries.push(model::ReadState {
                id: channel_id,
                last_message_id: Some(last_message_id),
                mention_count: 0,
                kind: 0,
                last_pin_timestamp: None,
                flags: None,
            });
            index
        });
        source.read_state.entries.get_mut(index)
    }

    /// Not marked as sent here: an ack cancelled or failed before reaching Discord would drop them.
    pub fn ack_flags(&self, channel_id: u64) -> Option<u32> {
        let source = self.source.as_ref()?;
        let kind = self.source_channels.get(&channel_id)?.get(source)?.kind;
        let expected = match kind {
            Some(10..=12) => 2,
            Some(1 | 3) => 0,
            _ => 1,
        };
        let current = read_state(source, &self.read_states, channel_id).and_then(|s| s.flags);
        (current != Some(expected)).then_some(expected)
    }

    pub fn confirm_ack_flags(&mut self, channel_id: u64, flags: u32) {
        if let Some(state) = self.read_state_mut(channel_id, 0) {
            state.flags = Some(flags);
        }
    }

    pub(super) fn is_private_conversation(&self, channel_id: u64) -> bool {
        self.source
            .as_ref()
            .and_then(|s| self.source_channels.get(&channel_id)?.get(s))
            .is_some_and(|c| matches!(c.kind, Some(1 | 3)))
    }

    pub(super) fn increment_mentions(&mut self, channel_id: u64, message_id: u64) {
        if let Some(state) = self.read_state_mut(channel_id, message_id) {
            state.mention_count = state.mention_count.saturating_add(1);
        }
    }

    pub(super) fn refresh_badges(&mut self, channel_id: u64, previous_mentions: u32) {
        let mentions = self.mention_count(channel_id);
        let new_posts = self.forum_new_post_count(channel_id);
        let unread = if let Some(source) = &self.source {
            self.source_channels
                .get(&channel_id)
                .and_then(|index| index.get(source))
                .is_some_and(|c| {
                    channel_unread(c, read_state(source, &self.read_states, channel_id))
                })
        } else {
            true
        };
        if let Some(location) = self.channel_locations.get(&channel_id).copied() {
            let guild = &mut self.guilds[location.guild];
            let channel = &mut guild.categories[location.category].channels[location.channel];
            let was_unread = channel.kind.is_unreadable() && channel.unread;
            channel.unread = channel.kind.is_unreadable() && (unread || new_posts > 0);
            channel.mentions = mentions;
            channel.new_posts = new_posts;
            let badges = &mut self.guild_badges[location.guild];
            badges.unread_channels = badges
                .unread_channels
                .saturating_sub(usize::from(was_unread))
                + usize::from(channel.unread);
            badges.mentions =
                badges.mentions.saturating_sub(u64::from(previous_mentions)) + u64::from(mentions);
            guild.unread = badges.unread_channels > 0;
            guild.mentions = badges.mentions.min(u64::from(u32::MAX)) as u32;
        } else if let Some(location) = self.thread_locations.get(&channel_id).copied() {
            let thread = &mut self.guilds[location.guild].threads[location.thread];
            thread.unread = unread;
            thread.mentions = mentions;
        } else if self.is_private_conversation(channel_id) {
            self.dm_badges =
                self.dm_badges.saturating_sub(u64::from(previous_mentions)) + u64::from(mentions);
            self.dm_mentions = self.dm_badges.min(u64::from(u32::MAX)) as u32;
            self.update_private_badges(channel_id, unread, mentions);
        }
    }

    pub(super) fn apply_ack(&mut self, channel_id: u64, message_id: u64) {
        let latest = self.latest_message_id(channel_id);
        let mentions = self.mention_count(channel_id);
        if let Some(state) = self.read_state_mut(channel_id, message_id) {
            state.last_message_id = Some(message_id);
            if message_id >= latest {
                state.mention_count = 0;
            }
        }
        self.refresh_badges(channel_id, mentions);
        let read_last = self
            .source
            .as_ref()
            .and_then(|source| read_state(source, &self.read_states, channel_id))
            .and_then(|state| state.last_message_id)
            .unwrap_or(0);
        if let Some(post) = self.forum_posts.get_mut(&channel_id) {
            post.unread = latest > read_last;
        }
    }
}

const GUILD_EVENT_READ_STATE: u8 = 3;

fn guild_event_ack(source: &model::Ready, guild: &model::Guild) -> Option<(u64, u64, u8)> {
    let latest = guild
        .guild_scheduled_events
        .iter()
        .flatten()
        .map(|event| event.id)
        .max()?;
    let acked = source
        .read_state
        .entries
        .iter()
        .find(|state| state.kind == GUILD_EVENT_READ_STATE && state.id == guild.id)
        .and_then(|state| state.last_message_id)
        .or_else(|| {
            let joined = chrono::DateTime::parse_from_rfc3339(guild.joined_at.as_deref()?).ok()?;
            let since_epoch = u64::try_from(joined.timestamp_millis())
                .ok()?
                .checked_sub(crate::app::DISCORD_EPOCH_MS)?;
            Some(since_epoch << 22)
        })
        .unwrap_or(0);
    (latest > acked).then_some((guild.id, latest, GUILD_EVENT_READ_STATE))
}
