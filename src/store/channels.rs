use super::*;

impl Store {
    pub(super) fn source_channel_guild(
        &self,
        channel_id: u64,
    ) -> Option<(&model::Guild, &model::Channel)> {
        let source = self.source.as_ref()?;
        let entry = self.source_channels.get(&channel_id).copied()?;
        let (SourceChannel::Guild { guild, .. } | SourceChannel::Thread { guild, .. }) = entry
        else {
            return None;
        };
        Some((source.guilds.get(guild)?, entry.get(source)?))
    }

    pub(super) fn permissions(&self, channel_id: u64) -> Option<u64> {
        if let Some(permissions) = self.permission_cache.borrow().get(&channel_id) {
            return Some(*permissions);
        }
        let (guild, channel) = self.source_channel_guild(channel_id)?;
        let permissions = discord::channel_permissions(guild, channel, self.current_user_id);
        self.permission_cache
            .borrow_mut()
            .insert(channel_id, permissions);
        Some(permissions)
    }

    pub fn can_send(&self, channel_id: u64) -> bool {
        if self.source.is_none() {
            return true;
        }
        if let Some((_, thread)) = self.source_channel_guild(channel_id)
            && matches!(thread.kind, Some(10..=12))
        {
            let permissions = self.permissions(channel_id).unwrap_or(0);
            let locked = thread
                .thread_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.locked);
            return permissions & discord::VIEW_CHANNEL != 0
                && permissions & discord::SEND_MESSAGES_IN_THREADS != 0
                && (!locked || permissions & discord::MANAGE_THREADS != 0);
        }
        if self
            .source_channel_guild(channel_id)
            .is_some_and(|(_, channel)| matches!(channel.kind, Some(15 | 16)))
        {
            return false;
        }
        if let Some(SourceChannel::Private(_)) = self.source_channels.get(&channel_id) {
            // Discord refuses replies until the request is accepted.
            return !self
                .private_channel(channel_id)
                .is_some_and(|dm| dm.message_request);
        }
        self.permissions(channel_id).is_some_and(|permissions| {
            permissions & (discord::VIEW_CHANNEL | discord::SEND_MESSAGES)
                == discord::VIEW_CHANNEL | discord::SEND_MESSAGES
        })
    }

    pub fn can_manage_messages(&self, channel_id: u64) -> bool {
        if self.source.is_none() {
            return true;
        }
        self.permissions(channel_id)
            .is_some_and(|permissions| permissions & discord::MANAGE_MESSAGES != 0)
    }

    pub fn can_mention_everyone(&self, channel_id: u64) -> bool {
        self.source_channel_guild(channel_id).is_some()
            && self
                .permissions(channel_id)
                .is_some_and(|permissions| permissions & discord::MENTION_EVERYONE != 0)
    }

    pub(super) fn index_channels(&mut self) {
        self.index_names();
        self.index_forum_posts();
        let present: HashMap<u64, bool> = self
            .channel_locations
            .iter()
            .map(|(id, location)| (*id, location.at_present))
            .chain(
                self.thread_locations
                    .iter()
                    .map(|(id, location)| (*id, location.at_present)),
            )
            .chain(
                self.private_locations
                    .iter()
                    .map(|(id, location)| (*id, location.at_present)),
            )
            .collect();
        self.source_channels.clear();
        self.channel_locations.clear();
        self.thread_locations.clear();
        self.guild_badges.clear();
        self.guild_indices.clear();
        if let Some(source) = &self.source {
            for (guild, metadata) in source.guilds.iter().enumerate() {
                for (channel, metadata) in metadata.threads.iter().flatten().enumerate() {
                    self.source_channels
                        .insert(metadata.id, SourceChannel::Thread { guild, channel });
                }
                for (channel, metadata) in metadata.channels.iter().flatten().enumerate() {
                    self.source_channels
                        .insert(metadata.id, SourceChannel::Guild { guild, channel });
                }
            }
            for (index, channel) in source.private_channels.iter().enumerate() {
                self.source_channels
                    .insert(channel.id, SourceChannel::Private(index));
            }
            self.dm_badges = source
                .private_channels
                .iter()
                .filter(|c| matches!(c.kind, Some(1 | 3)))
                .map(|c| {
                    u64::from(
                        read_state(source, &self.read_states, c.id).map_or(0, |s| s.mention_count),
                    )
                })
                .sum();
            self.dm_mentions = self.dm_badges.min(u64::from(u32::MAX)) as u32;
        }
        self.rebuild_private(&present);
        for (guild_index, guild) in self.guilds.iter_mut().enumerate() {
            self.guild_indices.insert(guild.id, guild_index);
            let mut badges = GuildBadges {
                unread_channels: 0,
                mentions: 0,
            };
            for (category_index, category) in guild.categories.iter_mut().enumerate() {
                for (channel_index, channel) in category.channels.iter_mut().enumerate() {
                    self.channel_locations.insert(
                        channel.id,
                        ChannelLocation {
                            guild: guild_index,
                            category: category_index,
                            channel: channel_index,
                            at_present: present
                                .get(&channel.id)
                                .copied()
                                .unwrap_or(channel.messages_loaded),
                        },
                    );
                    if let Some(source) = &self.source {
                        channel.mentions = read_state(source, &self.read_states, channel.id)
                            .map_or(0, |state| state.mention_count);
                        if channel.kind.is_forum() {
                            channel.new_posts = source.guilds.get(guild_index).map_or(0, |guild| {
                                forum_new_posts(
                                    guild,
                                    channel.id,
                                    read_state(source, &self.read_states, channel.id),
                                )
                            });
                        }
                    }
                    if channel.kind.is_unreadable() {
                        channel.unread |= channel.new_posts > 0;
                    } else {
                        channel.unread = false;
                    }
                    badges.unread_channels +=
                        usize::from(channel.kind.is_unreadable() && channel.unread);
                    badges.mentions += u64::from(channel.mentions);
                }
            }
            for (thread_index, thread) in guild.threads.iter_mut().enumerate() {
                self.thread_locations.insert(
                    thread.id,
                    ThreadLocation {
                        guild: guild_index,
                        thread: thread_index,
                        at_present: present
                            .get(&thread.id)
                            .copied()
                            .unwrap_or(thread.messages_loaded),
                    },
                );
                if let Some(source) = &self.source {
                    thread.mentions = read_state(source, &self.read_states, thread.id)
                        .map_or(0, |state| state.mention_count);
                }
            }
            guild.mentions = badges.mentions.min(u64::from(u32::MAX)) as u32;
            self.guild_badges.push(badges);
        }
        self.rebuild_rail();
    }

    pub(super) fn rebuild_structure(&mut self, guild_id: Option<u64>) {
        if let (Some(source), Some(guild_id)) = (&self.source, guild_id) {
            let metadata = source.guilds.iter().find(|g| g.id == guild_id);
            let position = self.guilds.iter().position(|g| g.id == guild_id);
            if let Some(metadata) = metadata {
                let mut rebuilt = map_guild(metadata, source, &self.read_states);
                for user in metadata
                    .members
                    .iter()
                    .flatten()
                    .filter_map(|m| m.user.as_ref())
                {
                    if !self.users.contains_key(&user.id) {
                        self.users.insert(user.id, map_user(user));
                    }
                }
                if let Some(position) = position {
                    if let Some(social) = self.social.guilds.get(&guild_id) {
                        rebuilt.member_ids = social.members.keys().copied().collect();
                    }
                    let old = &mut self.guilds[position];
                    let mut histories: HashMap<_, _> = old
                        .categories
                        .iter_mut()
                        .flat_map(|c| &mut c.channels)
                        .chain(old.threads.iter_mut())
                        .map(|c| (c.id, (c.messages_loaded, std::mem::take(&mut c.messages))))
                        .collect();
                    for channel in rebuilt
                        .categories
                        .iter_mut()
                        .flat_map(|c| &mut c.channels)
                        .chain(rebuilt.threads.iter_mut())
                    {
                        if let Some((loaded, messages)) = histories.remove(&channel.id) {
                            channel.messages_loaded = loaded;
                            channel.messages = messages;
                        }
                    }
                    self.guilds[position] = rebuilt;
                } else {
                    self.guilds.push(rebuilt);
                }
            } else if let Some(position) = position {
                self.guilds.remove(position);
            }
        }
        self.index_channels();
    }

    pub(super) fn thread_list_sync(&mut self, sync: Box<model::ThreadListSync>) -> Change {
        let model::ThreadListSync {
            guild_id,
            threads,
            most_recent_messages,
            members,
        } = *sync;
        let current_user_id = self.current_user_id;
        let thread_ids: Vec<_> = threads.iter().map(|thread| thread.id).collect();
        for thread in &threads {
            if let Some(member) = &thread.member {
                self.record_thread_join(
                    thread.id,
                    member
                        .user_id
                        .or_else(|| member.user.as_ref().map(|user| user.id)),
                    member.join_timestamp.as_deref(),
                );
            }
            if let Some(owner) = &thread.owner
                && let Some(user) = &owner.user
            {
                self.register_author(user);
            }
        }
        for member in &members {
            if let Some(user) = member
                .member
                .as_ref()
                .and_then(|member| member.user.as_ref())
            {
                self.register_author(user);
            }
            self.record_thread_join(
                member.id,
                member
                    .user_id
                    .or_else(|| member.member.as_ref().and_then(model::Member::id)),
                member.join_timestamp.as_deref(),
            );
        }
        for message in most_recent_messages {
            self.register_author(&message.author);
            let entry = self
                .thread_recent
                .entry(message.channel_id)
                .or_insert_with(|| message.clone());
            if message.id > entry.id {
                *entry = message;
            }
        }
        if let Some(source) = &mut self.source
            && let Some(guild) = source.guilds.iter_mut().find(|g| g.id == guild_id)
        {
            let stored = guild.threads.get_or_insert_with(Vec::new);
            for mut thread in threads {
                if let Some(member) = members.iter().find(|member| {
                    member.id == thread.id && member.user_id.is_none_or(|id| id == current_user_id)
                }) {
                    thread.member = Some(
                        member
                            .member
                            .as_ref()
                            .map(model::PartialMember::from)
                            .unwrap_or_else(|| model::PartialMember {
                                user_id: member.user_id,
                                ..Default::default()
                            }),
                    );
                }
                if let Some(existing) = stored.iter_mut().find(|existing| existing.id == thread.id)
                {
                    existing.merge(&thread);
                } else {
                    stored.push(thread);
                }
            }
        }
        self.rebuild_structure(Some(guild_id));
        for thread_id in thread_ids {
            self.refresh_thread_chips(thread_id);
        }
        Change::Structure
    }

    pub(super) fn channel_upsert(&mut self, mut update: Box<model::Channel>) -> Change {
        for user in &update.recipients {
            self.register_author(user);
        }
        for user in update
            .owner
            .iter()
            .filter_map(|owner| owner.user.as_ref())
            .chain(
                update
                    .member
                    .iter()
                    .filter_map(|member| member.user.as_ref()),
            )
        {
            self.register_author(user);
        }
        if update.recipient_ids.is_empty() {
            update.recipient_ids = update.recipients.iter().map(|user| user.id).collect();
        }
        let thread_id = (matches!(update.kind, Some(10..=12))
            || matches!(
                self.source_channels.get(&update.id),
                Some(SourceChannel::Thread { .. })
            ))
        .then_some(update.id);
        if let Some(thread_id) = thread_id
            && let Some(member) = &update.member
        {
            self.record_thread_join(
                thread_id,
                member
                    .user_id
                    .or_else(|| member.user.as_ref().map(|user| user.id)),
                member.join_timestamp.as_deref(),
            );
        }
        let mut guild_id = None;
        if let Some(source) = &mut self.source {
            let existing = self.source_channels.get(&update.id).copied();
            let guild_index = update
                .guild_id
                .and_then(|id| source.guilds.iter().position(|g| g.id == id))
                .or_else(|| match existing {
                    Some(
                        SourceChannel::Guild { guild, .. } | SourceChannel::Thread { guild, .. },
                    ) => Some(guild),
                    _ => None,
                });
            if let Some(index) = guild_index {
                let guild = &mut source.guilds[index];
                guild_id = Some(guild.id);
                let channels = if matches!(existing, Some(SourceChannel::Thread { .. }))
                    || matches!(update.kind, Some(10..=12))
                {
                    guild.threads.get_or_insert_with(Vec::new)
                } else {
                    guild.channels.get_or_insert_with(Vec::new)
                };
                if let Some(channel) = channels.iter_mut().find(|c| c.id == update.id) {
                    channel.merge(&update);
                } else {
                    channels.push(*update);
                }
            } else if let Some(channel) = existing.and_then(|index| index.get_mut(source)) {
                channel.merge(&update);
            } else if matches!(update.kind, Some(1 | 3)) {
                source.private_channels.push(*update);
            }
        }
        self.rebuild_structure(guild_id);
        if let Some(thread_id) = thread_id {
            self.refresh_thread_chips(thread_id);
        }
        Change::Structure
    }

    pub(super) fn channel_delete(&mut self, id: u64) -> Change {
        self.dm_calls.remove(&id);
        self.request_previews.remove(&id);
        self.thread_recent.remove(&id);
        self.thread_joins.remove(&id);
        self.clear_thread_chips(id);
        self.voice_start_times
            .retain(|(_, channel), _| *channel != id);
        self.clear_message_info(id);
        let mut guild_id = None;
        if let Some(source) = &mut self.source {
            match self.source_channels.get(&id).copied() {
                Some(SourceChannel::Guild { guild, .. }) => {
                    let guild = &mut source.guilds[guild];
                    guild_id = Some(guild.id);
                    if let Some(channels) = &mut guild.channels {
                        channels.retain(|c| c.id != id);
                    }
                }
                Some(SourceChannel::Thread { guild, .. }) => {
                    let guild = &mut source.guilds[guild];
                    guild_id = Some(guild.id);
                    if let Some(threads) = &mut guild.threads {
                        threads.retain(|channel| channel.id != id);
                    }
                }
                Some(SourceChannel::Private(_)) => source.private_channels.retain(|c| c.id != id),
                None => {}
            }
        }
        self.rebuild_structure(guild_id);
        Change::Structure
    }
}
