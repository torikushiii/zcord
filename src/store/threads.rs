use super::*;
use crate::text::markdown;

impl Store {
    pub(super) fn message_thread(&self, thread: &model::Channel) -> MessageThread {
        let current = self
            .source
            .as_ref()
            .and_then(|source| {
                source
                    .guilds
                    .iter()
                    .flat_map(|guild| guild.threads.iter().flatten())
                    .find(|current| current.id == thread.id)
            })
            .unwrap_or(thread);
        let row = self.thread_row(current);
        MessageThread {
            id: current.id,
            name: current.name.clone().unwrap_or_default(),
            message_count: current.message_count.unwrap_or(0),
            archived: row.archived,
            recent: row.recent,
        }
    }

    pub(super) fn refresh_thread_notice(&self, message: &mut Message) {
        let Some(id) = message.thread.as_ref().map(|thread| thread.id) else {
            return;
        };
        let current = self.source.as_ref().and_then(|source| {
            source
                .guilds
                .iter()
                .flat_map(|guild| guild.threads.iter().flatten())
                .find(|thread| thread.id == id)
        });
        if let Some(current) = current {
            message.thread = Some(self.message_thread(current));
        }
    }

    pub(super) fn refresh_thread_chips(&mut self, thread_id: u64) {
        let Some(thread) = self
            .source
            .as_ref()
            .and_then(|source| {
                source
                    .guilds
                    .iter()
                    .flat_map(|guild| guild.threads.iter().flatten())
                    .find(|thread| thread.id == thread_id)
            })
            .cloned()
        else {
            self.clear_thread_chips(thread_id);
            return;
        };
        let preview = self.message_thread(&thread);
        for guild in &mut self.guilds {
            for channel in guild
                .categories
                .iter_mut()
                .flat_map(|category| &mut category.channels)
                .chain(guild.threads.iter_mut())
            {
                for message in &mut channel.messages {
                    if message
                        .thread
                        .as_ref()
                        .is_some_and(|thread| thread.id == thread_id)
                    {
                        message.thread = Some(preview.clone());
                    }
                }
            }
        }
    }

    pub(super) fn clear_thread_chips(&mut self, thread_id: u64) {
        for guild in &mut self.guilds {
            for channel in guild
                .categories
                .iter_mut()
                .flat_map(|category| &mut category.channels)
                .chain(guild.threads.iter_mut())
            {
                for message in &mut channel.messages {
                    if message
                        .thread
                        .as_ref()
                        .is_some_and(|thread| thread.id == thread_id)
                    {
                        message.thread = None;
                    }
                }
            }
        }
    }

    pub fn active_threads(&self, parent_id: u64) -> Vec<ThreadRow> {
        let Some(source) = &self.source else {
            return Vec::new();
        };
        let mut rows: Vec<_> = source
            .guilds
            .iter()
            .flat_map(|guild| guild.threads.iter().flatten())
            .filter(|thread| {
                thread.parent_id.flatten() == Some(parent_id)
                    && !thread
                        .thread_metadata
                        .as_ref()
                        .is_some_and(|metadata| metadata.archived)
            })
            .map(|thread| self.thread_row(thread))
            .collect();
        rows.sort_by(|left, right| {
            right
                .last_active
                .cmp(&left.last_active)
                .then_with(|| right.id.cmp(&left.id))
        });
        rows
    }

    pub fn thread_row_by_id(&self, thread_id: u64) -> Option<ThreadRow> {
        self.source
            .as_ref()?
            .guilds
            .iter()
            .flat_map(|guild| guild.threads.iter().flatten())
            .find(|thread| thread.id == thread_id)
            .map(|thread| self.thread_row(thread))
    }

    pub fn sidebar_threads(&self, parent_id: u64, selected: u64) -> Vec<ThreadRow> {
        let Some(source) = &self.source else {
            return Vec::new();
        };
        let now = chrono::Utc::now().timestamp_millis();
        let mut rows: Vec<(i64, ThreadRow)> = Vec::new();
        for thread in source
            .guilds
            .iter()
            .flat_map(|guild| guild.threads.iter().flatten())
            .filter(|thread| thread.parent_id.flatten() == Some(parent_id))
        {
            let selected_here = thread.id == selected;
            let row = self.thread_row(thread);
            if !selected_here {
                if !row.joined || row.archived {
                    continue;
                }
                let unread = self.thread_unread(thread.id);
                let mentions = self.thread_mentions(thread.id);
                if !unread && mentions == 0 && !self.thread_recent(thread, now) {
                    continue;
                }
            }
            let joined = if selected_here && !row.joined {
                i64::MAX
            } else {
                self.thread_joins.get(&thread.id).copied().unwrap_or(0)
            };
            rows.push((joined, row));
        }
        rows.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.id.cmp(&left.1.id))
        });
        rows.into_iter().map(|(_, row)| row).collect()
    }

    pub fn can_create_thread(&self, channel_id: u64, private: bool) -> bool {
        if self.source.is_none() {
            return true;
        }
        let Some((_, channel)) = self.source_channel_guild(channel_id) else {
            return false;
        };
        if !matches!(channel.kind, Some(0 | 5 | 15 | 16)) || (private && channel.kind != Some(0)) {
            return false;
        }
        let Some(permissions) = self.permissions(channel_id) else {
            return false;
        };
        let required = if private {
            discord::CREATE_PRIVATE_THREADS
        } else {
            discord::CREATE_PUBLIC_THREADS
        };
        permissions & required != 0
    }

    pub fn can_manage_thread(&self, thread_id: u64) -> bool {
        if self.source.is_none() {
            return true;
        }
        let Some((_, thread)) = self.source_channel_guild(thread_id) else {
            return false;
        };
        if !matches!(thread.kind, Some(10..=12)) {
            return false;
        }
        let owner = thread
            .owner
            .as_ref()
            .and_then(model::Member::id)
            .or(thread.owner_id);
        if owner == Some(self.current_user_id) {
            return true;
        }
        let Some(parent) = thread.parent_id.flatten() else {
            return false;
        };
        self.permissions(parent)
            .is_some_and(|permissions| permissions & discord::MANAGE_THREADS != 0)
    }

    pub fn thread_unread(&self, thread_id: u64) -> bool {
        let Some(source) = &self.source else {
            return false;
        };
        self.source_channels
            .get(&thread_id)
            .and_then(|entry| entry.get(source))
            .is_some_and(|thread| {
                channel_unread(thread, read_state(source, &self.read_states, thread_id))
            })
    }

    pub fn thread_mentions(&self, thread_id: u64) -> u32 {
        self.source
            .as_ref()
            .and_then(|source| read_state(source, &self.read_states, thread_id))
            .map_or(0, |state| state.mention_count)
    }

    pub fn thread_row(&self, thread: &model::Channel) -> ThreadRow {
        let member_ids = thread.member_ids_preview.clone().unwrap_or_default();
        let recent = self.thread_recent.get(&thread.id).and_then(|message| {
            Some((
                message.author.id,
                recent_text(&message.content),
                local_timestamp(&message.timestamp)?,
            ))
        });
        let archive = thread
            .thread_metadata
            .as_ref()
            .and_then(|metadata| metadata.archive_timestamp.as_deref())
            .and_then(local_timestamp);
        let last_active = recent.as_ref().map(|recent| recent.2).or(archive);
        ThreadRow {
            id: thread.id,
            name: thread.name.clone().unwrap_or_default(),
            parent_id: thread.parent_id.flatten().unwrap_or(0),
            owner_id: thread
                .owner
                .as_ref()
                .and_then(model::Member::id)
                .or(thread.owner_id),
            joined: thread.member.is_some() || member_ids.contains(&self.current_user_id),
            archived: thread
                .thread_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.archived),
            locked: thread
                .thread_metadata
                .as_ref()
                .is_some_and(|metadata| metadata.locked),
            private: thread.kind == Some(12),
            member_count: thread
                .member_count
                .unwrap_or(member_ids.len().min(u32::MAX as usize) as u32),
            member_ids,
            recent,
            last_active,
        }
    }

    pub fn remember_thread(&mut self, thread: model::Channel) {
        if let Some(owner) = &thread.owner
            && let Some(user) = &owner.user
        {
            self.register_author(user);
        }
        if let Some(member) = &thread.member
            && let Some(user) = &member.user
        {
            self.register_author(user);
        }
        if let Some(member) = &thread.member {
            self.record_thread_join(
                thread.id,
                member
                    .user_id
                    .or_else(|| member.user.as_ref().map(|user| user.id)),
                member.join_timestamp.as_deref(),
            );
        }
        let guild_id = thread.guild_id.or_else(|| {
            thread
                .parent_id
                .flatten()
                .and_then(|parent| self.source_channel_guild(parent).map(|(guild, _)| guild.id))
        });
        let Some(guild_id) = guild_id else {
            return;
        };
        let mut thread = thread;
        thread.guild_id = Some(guild_id);
        if let Some(source) = &mut self.source
            && let Some(guild) = source.guilds.iter_mut().find(|guild| guild.id == guild_id)
        {
            let threads = guild.threads.get_or_insert_with(Vec::new);
            if let Some(existing) = threads.iter_mut().find(|existing| existing.id == thread.id) {
                existing.merge(&thread);
            } else {
                threads.push(thread);
            }
        }
        self.rebuild_structure(Some(guild_id));
    }

    pub fn map_listed_message(&mut self, guild_id: u64, mut message: model::Message) -> Message {
        if message.guild_id.is_none() {
            message.guild_id = Some(guild_id);
        }
        self.map_network_message(&message, true)
            .expect("listed messages have a valid timestamp")
    }

    pub(super) fn thread_members_update(
        &mut self,
        update: Box<model::ThreadMembersUpdate>,
    ) -> Change {
        let current_user_id = self.current_user_id;
        for member in &update.added_members {
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
        if update.removed_member_ids.contains(&current_user_id) {
            self.remove_thread_join(update.id);
        }
        let Some(source) = &mut self.source else {
            return Change::None;
        };
        let Some(thread) = source
            .guilds
            .iter_mut()
            .flat_map(|guild| guild.threads.iter_mut().flatten())
            .find(|thread| thread.id == update.id)
        else {
            return Change::None;
        };
        thread.member_count = Some(update.member_count);
        let mut preview = thread.member_ids_preview.take().unwrap_or_default();
        for member in &update.added_members {
            if let Some(user_id) = member
                .user_id
                .or_else(|| member.member.as_ref().and_then(model::Member::id))
            {
                if !preview.contains(&user_id) {
                    preview.push(user_id);
                }
                if user_id == current_user_id {
                    let mut partial = member
                        .member
                        .as_ref()
                        .map(model::PartialMember::from)
                        .unwrap_or_else(|| model::PartialMember {
                            user_id: Some(user_id),
                            ..Default::default()
                        });
                    partial.join_timestamp = member.join_timestamp.clone();
                    thread.member = Some(partial);
                }
            }
        }
        preview.retain(|id| !update.removed_member_ids.contains(id));
        if update.removed_member_ids.contains(&current_user_id) {
            thread.member = None;
        }
        thread.member_ids_preview = Some(preview);
        Change::Structure
    }

    pub(super) fn thread_member_update(
        &mut self,
        update: Box<model::ThreadMemberUpdate>,
    ) -> Change {
        let current_user_id = self.current_user_id;
        if let Some(user) = update
            .member
            .as_ref()
            .and_then(|member| member.user.as_ref())
        {
            self.register_author(user);
        }
        self.record_thread_join(
            update.id,
            update
                .user_id
                .or_else(|| update.member.as_ref().and_then(model::Member::id)),
            update.join_timestamp.as_deref(),
        );
        let Some(source) = &mut self.source else {
            return Change::None;
        };
        let Some(thread) = source
            .guilds
            .iter_mut()
            .flat_map(|guild| guild.threads.iter_mut().flatten())
            .find(|thread| thread.id == update.id)
        else {
            return Change::None;
        };
        if let Some(count) = update.member_count {
            thread.member_count = Some(count);
        }
        let user_id = update
            .user_id
            .or_else(|| update.member.as_ref().and_then(model::Member::id));
        if user_id.is_none_or(|id| id == current_user_id) {
            let mut partial = update
                .member
                .as_ref()
                .map(model::PartialMember::from)
                .unwrap_or_else(|| model::PartialMember {
                    user_id,
                    ..Default::default()
                });
            partial.join_timestamp = update.join_timestamp.clone();
            thread.member = Some(partial);
        }
        Change::Structure
    }
    pub(super) fn seed_thread_joins(&mut self) {
        let Some(source) = &self.source else {
            return;
        };
        let joins: Vec<_> = source
            .guilds
            .iter()
            .flat_map(|guild| guild.threads.iter().flatten())
            .filter_map(|thread| {
                let member = thread.member.as_ref()?;
                Some((
                    thread.id,
                    member
                        .user_id
                        .or_else(|| member.user.as_ref().map(|user| user.id)),
                    member.join_timestamp.clone(),
                ))
            })
            .collect();
        for (thread_id, user_id, joined_at) in joins {
            self.record_thread_join(thread_id, user_id, joined_at.as_deref());
        }
    }

    pub(super) fn record_thread_join(
        &mut self,
        thread_id: u64,
        user_id: Option<u64>,
        joined_at: Option<&str>,
    ) {
        if user_id.is_some_and(|id| id != self.current_user_id) {
            return;
        }
        let joined = joined_at
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map_or(0, |value| value.timestamp_millis());
        self.thread_joins.insert(thread_id, joined);
    }

    pub(super) fn remove_thread_join(&mut self, thread_id: u64) {
        self.thread_joins.remove(&thread_id);
    }

    fn thread_recent(&self, thread: &model::Channel, now: i64) -> bool {
        let metadata = thread.thread_metadata.as_ref();
        let minutes = metadata
            .and_then(|metadata| metadata.auto_archive_duration)
            .unwrap_or(1440)
            .max(1);
        let window = i64::from(minutes) * 60_000;
        let latest = thread
            .last_message_id
            .flatten()
            .and_then(snowflake_ms)
            .or_else(|| snowflake_ms(thread.id))
            .unwrap_or(now)
            .max(
                metadata
                    .and_then(|metadata| metadata.create_timestamp.as_deref())
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .map_or(0, |value| value.timestamp_millis()),
            )
            .max(
                metadata
                    .and_then(|metadata| metadata.archive_timestamp.as_deref())
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .map_or(0, |value| value.timestamp_millis()),
            );
        now < latest.saturating_add(window)
    }
}

fn snowflake_ms(id: u64) -> Option<i64> {
    let timestamp = (id >> 22).checked_add(1_420_070_400_000)?;
    i64::try_from(timestamp).ok()
}

fn recent_text(content: &str) -> String {
    let text = markdown::plain_text(&markdown::parse(content));
    let text = text.trim();
    let mut end = text.len().min(200);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}
