use super::*;

#[derive(Clone, Copy)]
pub(super) struct PrivateLocation {
    pub(super) index: usize,
    pub(super) at_present: bool,
}

const ACTIVE_NOW_MAX_RANK: u32 = 15;

fn relationship_kind(kind: u8) -> Option<RelationshipKind> {
    match kind {
        1 => Some(RelationshipKind::Friend),
        2 => Some(RelationshipKind::Blocked),
        3 => Some(RelationshipKind::Incoming),
        4 => Some(RelationshipKind::Outgoing),
        _ => None,
    }
}

fn map_relationship(relationship: &model::Relationship) -> Option<Relationship> {
    Some(Relationship {
        user_id: relationship.id,
        kind: relationship_kind(relationship.kind)?,
        nickname: relationship.nickname.clone(),
        since: relationship.since.as_deref().and_then(local_timestamp),
        spam: relationship.is_spam_request,
        ignored: relationship.user_ignored,
    })
}

fn display_name(
    channel: &model::Channel,
    users: &HashMap<u64, User>,
    current_user_id: u64,
) -> String {
    if channel.kind == Some(3) {
        if let Some(name) = channel.name.as_deref().filter(|name| !name.is_empty()) {
            return name.to_owned();
        }
        let names: Vec<_> = channel
            .recipient_ids
            .iter()
            .filter(|id| **id != current_user_id)
            .filter_map(|id| users.get(id))
            .map(|user| user.name.clone())
            .collect();
        if !names.is_empty() {
            return names.join(", ");
        }
        return "Unnamed".to_owned();
    }
    channel
        .recipient_ids
        .iter()
        .find(|id| **id != current_user_id)
        .and_then(|id| users.get(id))
        .map(|user| user.name.clone())
        .unwrap_or_else(|| "Unknown User".to_owned())
}

impl Store {
    pub(super) fn init_relationships(&mut self) {
        self.relationships.clear();
        let relationships = self
            .source
            .as_ref()
            .map(|source| source.relationships.clone())
            .unwrap_or_default();
        for relationship in relationships {
            if let Some(user) = &relationship.user {
                self.register_author(user);
            }
            if let Some(mapped) = map_relationship(&relationship) {
                self.relationships.insert(mapped.user_id, mapped);
            }
        }
    }

    pub(super) fn rebuild_private(&mut self, present: &HashMap<u64, bool>) {
        let Some(source) = &self.source else {
            self.private.clear();
            self.private_locations.clear();
            return;
        };
        let mut histories: HashMap<u64, (bool, Vec<Message>)> = self
            .private
            .iter_mut()
            .map(|private| {
                let channel = &mut private.channel;
                (
                    channel.id,
                    (
                        channel.messages_loaded,
                        std::mem::take(&mut channel.messages),
                    ),
                )
            })
            .collect();
        let mut private_channels = Vec::new();
        let mut locations = HashMap::new();
        for channel in source
            .private_channels
            .iter()
            .filter(|channel| matches!(channel.kind, Some(1 | 3)))
        {
            let group = channel.kind == Some(3);
            let state = read_state(source, &self.read_states, channel.id);
            let (messages_loaded, messages) = histories.remove(&channel.id).unwrap_or_default();
            locations.insert(
                channel.id,
                PrivateLocation {
                    index: private_channels.len(),
                    at_present: present.get(&channel.id).copied().unwrap_or(messages_loaded),
                },
            );
            private_channels.push(PrivateChannel {
                channel: Channel {
                    id: channel.id,
                    name: display_name(channel, &self.users, self.current_user_id),
                    kind: ChannelKind::Text,
                    parent_id: None,
                    private: false,
                    unread: channel_unread(channel, state),
                    mentions: state.map_or(0, |state| state.mention_count),
                    new_posts: 0,
                    messages,
                    messages_loaded,
                },
                group,
                recipient_ids: channel.recipient_ids.clone(),
                name: channel.name.clone().filter(|_| group),
                icon: channel.icon.clone().flatten(),
                owner_id: channel.owner_id,
                message_request: channel.is_message_request == Some(true),
                spam: channel.is_spam == Some(true),
            });
        }
        self.private = private_channels;
        self.private_locations = locations;
    }

    pub(super) fn update_private_badges(&mut self, channel_id: u64, unread: bool, mentions: u32) {
        let Some(location) = self.private_locations.get(&channel_id).copied() else {
            return;
        };
        if let Some(private) = self.private.get_mut(location.index) {
            private.channel.unread = unread;
            private.channel.mentions = mentions;
        }
    }

    pub(super) fn mark_private_read(&mut self, channel_id: u64) {
        let Some(location) = self.private_locations.get(&channel_id).copied() else {
            return;
        };
        let Some(private) = self.private.get_mut(location.index) else {
            return;
        };
        let mentions = private.channel.mentions;
        private.channel.unread = false;
        private.channel.mentions = 0;
        self.dm_badges = self.dm_badges.saturating_sub(u64::from(mentions));
        self.dm_mentions = self.dm_badges.min(u64::from(u32::MAX)) as u32;
    }

    pub fn set_request_previews(&mut self, previews: Vec<model::MessageRequestPreview>) {
        for preview in previews {
            let Some(message) = preview.message_preview else {
                self.request_previews.remove(&preview.channel_id);
                continue;
            };
            self.register_author(&message.author);
            let Some(timestamp) = local_timestamp(&message.timestamp) else {
                continue;
            };
            self.request_previews.insert(
                preview.channel_id,
                RequestPreview {
                    author_id: message.author.id,
                    content: message.content,
                    timestamp,
                },
            );
        }
    }

    pub fn request_preview(&self, channel_id: u64) -> Option<&RequestPreview> {
        self.request_previews.get(&channel_id)
    }

    pub(super) fn refresh_request_preview(&mut self, message: &model::Message, newest: bool) {
        if !newest
            || !self
                .private_channel(message.channel_id)
                .is_some_and(|private| private.message_request)
        {
            return;
        }
        let Some(timestamp) = local_timestamp(&message.timestamp) else {
            return;
        };
        self.request_previews.insert(
            message.channel_id,
            RequestPreview {
                author_id: message.author.id,
                content: message.content.clone(),
                timestamp,
            },
        );
    }

    pub fn accept_request_locally(&mut self, channel_id: u64) -> Change {
        let location = self.source_channels.get(&channel_id).copied();
        if let (Some(source), Some(location)) = (&mut self.source, location)
            && let Some(channel) = location.get_mut(source)
        {
            channel.is_message_request = Some(false);
        }
        self.rebuild_structure(None);
        Change::Structure
    }

    pub fn remove_private_locally(&mut self, channel_id: u64) -> Change {
        self.dm_calls.remove(&channel_id);
        self.clear_message_info(channel_id);
        if let Some(source) = &mut self.source {
            source
                .private_channels
                .retain(|channel| channel.id != channel_id);
        }
        self.request_previews.remove(&channel_id);
        self.rebuild_structure(None);
        Change::Structure
    }

    pub(super) fn apply_relationship_add(&mut self, relationship: &model::Relationship) {
        if let Some(user) = &relationship.user {
            self.register_author(user);
        }
        match map_relationship(relationship) {
            Some(mapped) => {
                self.relationships.insert(mapped.user_id, mapped);
            }
            None => {
                self.relationships.remove(&relationship.id);
            }
        }
    }

    pub(super) fn apply_relationship_update(&mut self, update: &model::RelationshipUpdate) {
        if let Some(kind) = update.kind {
            match relationship_kind(kind) {
                Some(kind) => {
                    let relationship =
                        self.relationships
                            .entry(update.id)
                            .or_insert_with(|| Relationship {
                                user_id: update.id,
                                kind,
                                nickname: None,
                                since: None,
                                spam: false,
                                ignored: false,
                            });
                    relationship.kind = kind;
                }
                None => {
                    self.relationships.remove(&update.id);
                    return;
                }
            }
        }
        let Some(relationship) = self.relationships.get_mut(&update.id) else {
            return;
        };
        if let Some(nickname) = &update.nickname {
            relationship.nickname.clone_from(nickname);
        }
        if let Some(since) = &update.since {
            relationship.since = local_timestamp(since);
        }
        if let Some(spam) = update.is_spam_request {
            relationship.spam = spam;
        }
        if let Some(ignored) = update.user_ignored {
            relationship.ignored = ignored;
        }
    }

    pub(super) fn apply_relationship_remove(&mut self, id: u64) {
        self.relationships.remove(&id);
    }

    pub fn private_channel(&self, id: u64) -> Option<&PrivateChannel> {
        let location = self.private_locations.get(&id)?;
        self.private.get(location.index)
    }

    fn private_activity(&self, id: u64) -> u64 {
        match self.latest_message_id(id) {
            0 => id,
            latest => latest,
        }
    }

    fn sort_private_list(&self, list: &mut [&PrivateChannel]) {
        list.sort_by(|left, right| {
            self.private_activity(right.channel.id)
                .cmp(&self.private_activity(left.channel.id))
                .then_with(|| right.channel.id.cmp(&left.channel.id))
        });
    }

    pub fn dm_list(&self) -> Vec<&PrivateChannel> {
        let mut list: Vec<_> = self
            .private
            .iter()
            .filter(|private| !private.message_request && !private.spam)
            .collect();
        self.sort_private_list(&mut list);
        list
    }

    pub fn unread_dms(&self) -> Vec<&PrivateChannel> {
        let mut list: Vec<_> = self
            .private
            .iter()
            .filter(|private| {
                !private.message_request && !private.spam && private.channel.mentions > 0
            })
            .collect();
        self.sort_private_list(&mut list);
        list
    }

    pub fn message_requests(&self) -> Vec<&PrivateChannel> {
        let mut list: Vec<_> = self
            .private
            .iter()
            .filter(|private| private.message_request && !private.spam)
            .collect();
        self.sort_private_list(&mut list);
        list
    }

    pub fn message_request_count(&self) -> u32 {
        self.private
            .iter()
            .filter(|private| private.message_request && !private.spam)
            .count()
            .min(u32::MAX as usize) as u32
    }

    pub fn spam_requests(&self) -> Vec<&PrivateChannel> {
        let mut list: Vec<_> = self
            .private
            .iter()
            .filter(|private| private.message_request && private.spam)
            .collect();
        self.sort_private_list(&mut list);
        list
    }

    pub fn spam_request_count(&self) -> u32 {
        self.private
            .iter()
            .filter(|private| private.message_request && private.spam)
            .count()
            .min(u32::MAX as usize) as u32
    }

    /// Returns whether anything changed, so the caller knows to repaint.
    pub fn set_conceal_usernames(&mut self, on: bool) -> bool {
        if !mapping::set_conceal_usernames(on) {
            return false;
        }
        for user in self.users.values_mut() {
            mapping::refresh_name(user);
        }
        let names: Vec<String> = self
            .private
            .iter()
            .map(|private| self.private_channel_name(private.channel.id))
            .collect();
        for (private, name) in self.private.iter_mut().zip(names) {
            if !name.is_empty() {
                private.channel.name = name;
            }
        }
        true
    }

    pub fn private_channel_name(&self, id: u64) -> String {
        let Some(source) = &self.source else {
            return String::new();
        };
        let Some(channel) = self
            .source_channels
            .get(&id)
            .and_then(|index| index.get(source))
        else {
            return String::new();
        };
        display_name(channel, &self.users, self.current_user_id)
    }

    pub fn dm_channel_with(&self, user_id: u64) -> Option<u64> {
        self.private
            .iter()
            .find(|private| {
                !private.group
                    && private.recipient_ids.len() == 1
                    && private.recipient_ids[0] == user_id
            })
            .map(|private| private.channel.id)
    }

    pub fn relationship(&self, user_id: u64) -> Option<&Relationship> {
        self.relationships.get(&user_id)
    }

    fn sort_by_display_name(&self, ids: &mut [u64]) {
        ids.sort_by(|left, right| {
            self.friend_name(*left)
                .to_lowercase()
                .cmp(&self.friend_name(*right).to_lowercase())
                .then_with(|| left.cmp(right))
        });
    }

    pub fn friends(&self) -> Vec<u64> {
        let mut ids: Vec<_> = self
            .relationships
            .values()
            .filter(|relationship| relationship.kind == RelationshipKind::Friend)
            .map(|relationship| relationship.user_id)
            .collect();
        self.sort_by_display_name(&mut ids);
        ids
    }

    pub fn online_friends(&self) -> Vec<u64> {
        self.friends()
            .into_iter()
            .filter(|id| {
                self.users
                    .get(id)
                    .is_some_and(|user| user.status != Status::Offline)
            })
            .collect()
    }

    pub fn pending_requests(&self) -> Vec<u64> {
        let mut incoming: Vec<_> = self
            .relationships
            .values()
            .filter(|relationship| {
                relationship.kind == RelationshipKind::Incoming
                    && !relationship.spam
                    && !relationship.ignored
            })
            .map(|relationship| relationship.user_id)
            .collect();
        let mut outgoing: Vec<_> = self
            .relationships
            .values()
            .filter(|relationship| relationship.kind == RelationshipKind::Outgoing)
            .map(|relationship| relationship.user_id)
            .collect();
        self.sort_by_display_name(&mut incoming);
        self.sort_by_display_name(&mut outgoing);
        incoming.extend(outgoing);
        incoming
    }

    pub fn pending_count(&self) -> u32 {
        self.relationships
            .values()
            .filter(|relationship| {
                relationship.kind == RelationshipKind::Incoming
                    && !relationship.spam
                    && !relationship.ignored
            })
            .count()
            .min(u32::MAX as usize) as u32
    }

    /// The home button's badge: DM mentions plus the friend and message requests waiting on
    /// the user, leaving out spam, like Discord's.
    pub fn home_badge(&self) -> u32 {
        self.dm_mentions
            .saturating_add(self.pending_count())
            .saturating_add(self.message_request_count())
    }

    pub fn set_friend_suggestions(&mut self, suggestions: Vec<model::FriendSuggestion>) {
        let mapped = suggestions
            .iter()
            .map(|suggestion| self.map_friend_suggestion(suggestion))
            .collect();
        self.friend_suggestions = Some(mapped);
    }

    fn map_friend_suggestion(&mut self, suggestion: &model::FriendSuggestion) -> FriendSuggestion {
        self.register_author(&suggestion.suggested_user);
        FriendSuggestion {
            user_id: suggestion.suggested_user.id,
            reason: suggestion
                .reasons
                .iter()
                .find_map(|reason| reason.name.clone().filter(|name| !name.is_empty())),
        }
    }

    pub(super) fn add_friend_suggestion(&mut self, suggestion: &model::FriendSuggestion) {
        let mapped = self.map_friend_suggestion(suggestion);
        match &mut self.friend_suggestions {
            Some(list) => {
                list.retain(|existing| existing.user_id != mapped.user_id);
                list.push(mapped);
            }
            None => self.friend_suggestion_count = self.friend_suggestion_count.saturating_add(1),
        }
    }

    /// Drops a suggestion that was dismissed here or elsewhere.
    pub fn remove_friend_suggestion(&mut self, user_id: u64) {
        match &mut self.friend_suggestions {
            Some(list) => list.retain(|suggestion| suggestion.user_id != user_id),
            None => self.friend_suggestion_count = self.friend_suggestion_count.saturating_sub(1),
        }
    }

    /// Suggestions for people the user has no relationship with yet; accepting one sends a
    /// friend request, which moves them to Pending.
    pub fn friend_suggestions(&self) -> Vec<&FriendSuggestion> {
        self.friend_suggestions
            .iter()
            .flatten()
            .filter(|suggestion| !self.relationships.contains_key(&suggestion.user_id))
            .collect()
    }

    pub fn friend_suggestions_loaded(&self) -> bool {
        self.friend_suggestions.is_some()
    }

    pub fn friend_suggestion_count(&self) -> u32 {
        match &self.friend_suggestions {
            Some(_) => self.friend_suggestions().len().min(u32::MAX as usize) as u32,
            None => self.friend_suggestion_count,
        }
    }

    pub fn personalization_consented(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|ready| ready.personalization_consented)
    }

    pub fn set_user_affinities(&mut self, affinities: &[model::UserAffinity]) {
        self.social.close_contacts = affinities
            .iter()
            .filter(|affinity| affinity.communication_rank <= ACTIVE_NOW_MAX_RANK)
            .map(|affinity| affinity.other_user_id)
            .collect();
    }

    /// Friends and close contacts playing, streaming or listening to music, like Discord's
    /// Active Now column.
    pub fn active_now(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .friends()
            .into_iter()
            .chain(self.social.close_contacts.iter().copied())
            .filter(|id| {
                !self.relationship(*id).is_some_and(|relationship| {
                    relationship.kind == RelationshipKind::Blocked || relationship.ignored
                })
            })
            .filter(|id| {
                self.users.get(id).is_some_and(|user| {
                    !user.bot
                        && (user.game.is_some()
                            || user.listening.is_some()
                            || user.live_stream.is_some())
                        && user.status != Status::Offline
                })
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        self.sort_by_display_name(&mut ids);
        ids
    }

    pub fn friend_name(&self, user_id: u64) -> String {
        self.relationships
            .get(&user_id)
            .and_then(|relationship| relationship.nickname.clone())
            .or_else(|| self.users.get(&user_id).map(|user| user.name.clone()))
            .unwrap_or_else(|| "Unknown User".to_owned())
    }
}

#[cfg(test)]
mod tests;
