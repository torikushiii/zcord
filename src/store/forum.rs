use super::*;

fn forum_thread_created(thread: &model::Channel) -> Timestamp {
    thread
        .thread_metadata
        .as_ref()
        .and_then(|metadata| metadata.create_timestamp.as_deref())
        .and_then(local_timestamp)
        .unwrap_or_else(|| timestamp_from_nonce(thread.id))
}

fn forum_post_from_thread(thread: &model::Channel, forum_id: u64) -> ForumPost {
    let created_at = forum_thread_created(thread);
    ForumPost {
        id: thread.id,
        forum_id,
        name: thread.name.clone().unwrap_or_default(),
        owner_id: thread.owner_id.unwrap_or(0),
        created_at,
        last_activity: thread
            .last_message_id
            .flatten()
            .map(timestamp_from_nonce)
            .unwrap_or(created_at),
        message_count: thread.message_count.unwrap_or(0),
        tag_ids: thread.applied_tags.clone().unwrap_or_default(),
        pinned: thread.flags.unwrap_or(0) & 2 != 0,
        locked: thread
            .thread_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.locked),
        archived: thread
            .thread_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.archived),
        first_message: None,
        unread: false,
    }
}

impl Store {
    pub(super) fn index_forum_posts(&mut self) {
        let Some(source) = &self.source else {
            self.forum_posts.clear();
            return;
        };
        let mut next = HashMap::new();
        for guild in &source.guilds {
            let forums: std::collections::HashSet<u64> = guild
                .channels
                .iter()
                .flatten()
                .filter(|channel| matches!(channel.kind, Some(15 | 16)))
                .map(|channel| channel.id)
                .collect();
            for thread in guild.threads.iter().flatten() {
                let Some(forum_id) = thread.parent_id.flatten() else {
                    continue;
                };
                if !forums.contains(&forum_id) {
                    continue;
                }
                let mut post = self
                    .forum_posts
                    .remove(&thread.id)
                    .unwrap_or_else(|| forum_post_from_thread(thread, forum_id));
                Self::merge_forum_thread(
                    &mut post,
                    thread,
                    forum_id,
                    read_state(source, &self.read_states, thread.id),
                );
                next.insert(thread.id, post);
            }
        }
        self.forum_posts = next;

        if let Some(source) = &mut self.source {
            for guild in &mut source.guilds {
                let latest: HashMap<u64, u64> = guild
                    .threads
                    .iter()
                    .flatten()
                    .filter_map(|thread| Some((thread.parent_id.flatten()?, thread.id)))
                    .fold(HashMap::new(), |mut latest, (forum_id, thread_id)| {
                        latest
                            .entry(forum_id)
                            .and_modify(|id| *id = (*id).max(thread_id))
                            .or_insert(thread_id);
                        latest
                    });
                for channel in guild.channels.iter_mut().flatten() {
                    if matches!(channel.kind, Some(15 | 16)) {
                        channel.last_message_id = Some(latest.get(&channel.id).copied());
                    }
                }
            }
        }
    }

    fn merge_forum_thread(
        post: &mut ForumPost,
        thread: &model::Channel,
        forum_id: u64,
        state: Option<&model::ReadState>,
    ) {
        post.forum_id = forum_id;
        post.name = thread.name.clone().unwrap_or_default();
        if let Some(owner_id) = thread.owner_id {
            post.owner_id = owner_id;
        }
        post.created_at = forum_thread_created(thread);
        post.last_activity = thread
            .last_message_id
            .flatten()
            .map(timestamp_from_nonce)
            .unwrap_or(post.created_at);
        if let Some(message_count) = thread.message_count {
            post.message_count = message_count;
        }
        post.tag_ids = thread.applied_tags.clone().unwrap_or_default();
        post.pinned = thread.flags.unwrap_or(0) & 2 != 0;
        if let Some(metadata) = &thread.thread_metadata {
            post.locked = metadata.locked;
            post.archived = metadata.archived;
        }
        post.unread = channel_unread(thread, state);
    }

    pub(super) fn forum_new_post_count(&self, forum_id: u64) -> u32 {
        let Some(source) = &self.source else {
            return 0;
        };
        let Some(guild) = source.guilds.iter().find(|guild| {
            guild
                .channels
                .iter()
                .flatten()
                .any(|channel| channel.id == forum_id && matches!(channel.kind, Some(15 | 16)))
        }) else {
            return 0;
        };
        forum_new_posts(
            guild,
            forum_id,
            read_state(source, &self.read_states, forum_id),
        )
    }

    pub(super) fn latest_forum_post_id(&self, forum_id: u64) -> Option<u64> {
        let source = self.source.as_ref()?;
        source.guilds.iter().find_map(|guild| {
            guild
                .threads
                .iter()
                .flatten()
                .filter(|thread| thread.parent_id.flatten() == Some(forum_id))
                .map(|thread| thread.id)
                .max()
        })
    }

    pub fn forum_tags(&self, forum_id: u64) -> Vec<ForumTag> {
        let Some((_, channel)) = self.source_channel_guild(forum_id) else {
            return Vec::new();
        };
        channel
            .available_tags
            .iter()
            .flatten()
            .map(|tag| ForumTag {
                id: tag.id,
                name: tag.name.clone(),
                emoji: tag.emoji.as_ref().and_then(|emoji| {
                    if let Some(id) = emoji.id {
                        Some(StatusEmoji::Custom {
                            id,
                            name: emoji.name.clone().unwrap_or_default(),
                            animated: emoji.animated,
                        })
                    } else {
                        emoji
                            .name
                            .clone()
                            .filter(|name| !name.is_empty())
                            .map(StatusEmoji::Unicode)
                    }
                }),
                moderated: tag.moderated,
            })
            .collect()
    }

    pub fn forum_post(&self, id: u64) -> Option<&ForumPost> {
        self.forum_posts.get(&id)
    }

    pub fn forum_default_sort(&self, forum_id: u64) -> discord::ForumSort {
        match self
            .source_channel_guild(forum_id)
            .and_then(|(_, channel)| channel.default_sort_order)
        {
            Some(1) => discord::ForumSort::Created,
            _ => discord::ForumSort::Activity,
        }
    }

    /// The forum's REQUIRE_TAG flag: new posts must carry at least one tag.
    pub fn forum_requires_tag(&self, forum_id: u64) -> bool {
        self.source_channel_guild(forum_id)
            .is_some_and(|(_, channel)| channel.flags.unwrap_or(0) & (1 << 4) != 0)
    }

    pub fn forum_default_gallery(&self, forum_id: u64) -> bool {
        self.source_channel_guild(forum_id)
            .is_some_and(|(_, channel)| channel.default_forum_layout == Some(2))
    }

    #[cfg(test)]
    pub fn forum_posts(&self, forum_id: u64) -> Vec<&ForumPost> {
        self.forum_posts_sorted(forum_id, self.forum_default_sort(forum_id))
    }

    pub fn forum_posts_sorted(&self, forum_id: u64, sort: discord::ForumSort) -> Vec<&ForumPost> {
        let creation_sort = sort == discord::ForumSort::Created;
        let mut posts: Vec<_> = self
            .forum_posts
            .values()
            .filter(|post| post.forum_id == forum_id)
            .collect();
        if creation_sort {
            posts.sort_by(|a, b| {
                b.pinned
                    .cmp(&a.pinned)
                    .then_with(|| b.created_at.cmp(&a.created_at))
                    .then_with(|| b.id.cmp(&a.id))
            });
        } else {
            posts.sort_by(|a, b| {
                b.pinned
                    .cmp(&a.pinned)
                    .then_with(|| b.last_activity.cmp(&a.last_activity))
                    .then_with(|| b.id.cmp(&a.id))
            });
        }
        posts
    }

    pub fn apply_forum_search(&mut self, forum_id: u64, search: model::ForumSearch) -> Change {
        let Some(SourceChannel::Guild { guild, .. }) = self.source_channels.get(&forum_id).copied()
        else {
            return Change::None;
        };
        let Some(source) = self.source.as_mut() else {
            return Change::None;
        };
        let metadata = &mut source.guilds[guild];
        let guild_id = metadata.id;
        let threads = metadata.threads.get_or_insert_with(Vec::new);
        for mut thread in search.threads {
            if thread.guild_id.is_none() {
                thread.guild_id = Some(guild_id);
            }
            if thread.kind.is_none() {
                thread.kind = Some(11);
            }
            if thread.parent_id.flatten().is_none() {
                thread.parent_id = Some(Some(forum_id));
            }
            if let Some(existing) = threads.iter_mut().find(|existing| existing.id == thread.id) {
                existing.merge(&thread);
            } else {
                threads.push(thread);
            }
        }
        self.rebuild_structure(Some(guild_id));
        for message in &search.first_messages {
            self.apply_forum_first_message(message);
        }
        Change::Structure
    }

    /// Fills a post whose starter message was missing from the search page.
    pub fn apply_forum_starter(&mut self, message: &model::Message) -> bool {
        let missing = self
            .forum_posts
            .get(&message.channel_id)
            .is_some_and(|post| post.first_message.is_none());
        // The starter shares the thread's id; an `around` fetch can return a neighbour instead.
        if !missing || message.id != message.channel_id {
            return false;
        }
        self.apply_forum_first_message(message);
        true
    }

    fn apply_forum_first_message(&mut self, message: &model::Message) {
        if !self.forum_posts.contains_key(&message.channel_id) {
            return;
        }
        // Registers the author and their member entry so the card can name them.
        self.map_network_message(message, false);
        if let Some(post) = self.forum_posts.get_mut(&message.channel_id) {
            post.first_message = mapping::map_message(message);
        }
    }

    pub(super) fn update_forum_post_message(&mut self, message: &model::Message) {
        let read_last = self
            .source
            .as_ref()
            .and_then(|source| read_state(source, &self.read_states, message.channel_id))
            .and_then(|state| state.last_message_id)
            .unwrap_or(0);
        let timestamp =
            local_timestamp(&message.timestamp).unwrap_or_else(|| timestamp_from_nonce(message.id));
        let Some(post) = self.forum_posts.get_mut(&message.channel_id) else {
            return;
        };
        post.message_count = post.message_count.saturating_add(1);
        post.last_activity = timestamp;
        post.unread = message.id > read_last;
        if post.first_message.is_none() {
            post.first_message = mapping::map_message(message);
        }
    }
}
