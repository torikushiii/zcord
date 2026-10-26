use super::*;

#[derive(Clone, Copy)]
pub(super) struct PrivateLocation {
    pub(super) index: usize,
    pub(super) at_present: bool,
}

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
        if let (Some(source), Some(location)) = (&mut self.source, location) {
            if let Some(channel) = location.get_mut(source) {
                channel.is_message_request = Some(false);
            }
        }
        self.rebuild_structure(None);
        Change::Structure
    }

    pub fn remove_private_locally(&mut self, channel_id: u64) -> Change {
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

    fn sort_private_list<'a>(&self, list: &mut [&'a PrivateChannel]) {
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

    /// People playing a game or listening to music, like Discord's Active Now column.
    /// Besides friends, Discord draws on the server members it sent presences for.
    pub fn active_now(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .friends()
            .into_iter()
            .chain(self.social.supplemental_presences.iter().copied())
            .filter(|id| *id != self.current_user_id)
            .filter(|id| {
                self.users.get(id).is_some_and(|user| {
                    !user.bot
                        && (user.game.is_some() || user.listening.is_some())
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
mod tests {
    use crate::discord::Event;
    use crate::discord::model;
    use crate::store::{Change, PrivateChannel, RelationshipKind, Status, Store};
    use serde_json::json;

    fn ready() -> model::Ready {
        serde_json::from_str(include_str!("../discord/fixtures/home.json")).unwrap()
    }

    fn store() -> Store {
        Store::from_ready(ready())
    }

    fn message(id: u64, channel_id: u64, author: u64) -> model::Message {
        serde_json::from_value(json!({
            "id": id.to_string(),
            "channel_id": channel_id.to_string(),
            "author": {"id": author.to_string(), "username": "someone"},
            "timestamp": "2026-10-22T12:00:00Z",
            "content": "hello"
        }))
        .unwrap()
    }

    fn dm_ids(list: &[&PrivateChannel]) -> Vec<u64> {
        list.iter().map(|private| private.channel.id).collect()
    }

    #[test]
    fn ready_parses_home_state() {
        let ready = ready();
        assert_eq!(ready.relationships.len(), 8);
        assert_eq!(ready.friend_suggestion_count, 2);
        let request = ready
            .private_channels
            .iter()
            .find(|channel| channel.id == 904)
            .unwrap();
        assert_eq!(request.is_message_request, Some(true));
        let group = ready
            .private_channels
            .iter()
            .find(|channel| channel.id == 902)
            .unwrap();
        assert_eq!(group.icon, Some(Some("group-icon".to_owned())));
    }

    #[test]
    fn dm_list_orders_by_last_activity_and_hides_requests() {
        let store = store();
        assert_eq!(dm_ids(&store.dm_list()), [906, 901, 902, 900, 903]);
        let ids = dm_ids(&store.dm_list());
        assert!(!ids.contains(&904));
        assert!(!ids.contains(&905));
    }

    #[test]
    fn message_requests_are_listed_separately() {
        let store = store();
        assert_eq!(dm_ids(&store.message_requests()), [904]);
        assert_eq!(store.message_request_count(), 1);
    }

    #[test]
    fn spam_requests_are_listed_and_counted_separately() {
        let store = store();
        assert_eq!(dm_ids(&store.spam_requests()), [905]);
        assert_eq!(store.spam_request_count(), 1);
        assert_eq!(store.message_request_count(), 1);
    }

    #[test]
    fn request_previews_register_authors_and_follow_new_messages() {
        let mut store = store();
        let previews: Vec<model::MessageRequestPreview> = serde_json::from_value(json!([{
            "channel_id": "904",
            "message_preview": {
                "id": "650",
                "channel_id": "904",
                "author": {"id": "14", "username": "dave", "global_name": "Dave"},
                "timestamp": "2025-02-03T04:05:06Z",
                "content": "wanna play?"
            }
        }]))
        .unwrap();
        store.set_request_previews(previews);
        assert_eq!(store.user(14).unwrap().name, "Dave");
        let preview = store.request_preview(904).unwrap();
        assert_eq!(preview.author_id, 14);
        assert_eq!(preview.content, "wanna play?");

        store.apply(Event::MessageCreate(Box::new(message(651, 904, 17))));
        let preview = store.request_preview(904).unwrap();
        assert_eq!(preview.author_id, 17);
        assert_eq!(preview.content, "hello");
    }

    #[test]
    fn accepting_and_removing_requests_updates_the_lists() {
        let mut store = store();
        assert!(matches!(
            store.accept_request_locally(904),
            Change::Structure
        ));
        assert!(dm_ids(&store.dm_list()).contains(&904));
        assert_eq!(store.message_request_count(), 0);
        assert!(!store.private_channel(904).unwrap().message_request);

        assert!(matches!(
            store.remove_private_locally(905),
            Change::Structure
        ));
        assert!(store.private_channel(905).is_none());
        assert_eq!(store.spam_request_count(), 0);
        assert!(store.spam_requests().is_empty());
    }

    #[test]
    fn relationships_split_friends_and_pending() {
        let store = store();
        assert_eq!(store.pending_count(), 1);
        assert_eq!(store.pending_requests(), [14, 17]);
        assert_eq!(store.friends(), [10, 11, 12]);
        assert_eq!(
            store.relationship(10).unwrap().kind,
            RelationshipKind::Friend
        );
        assert!(!store.friends().contains(&13));
    }

    #[test]
    fn online_friends_follow_presence() {
        let mut store = store();
        let supplemental: model::ReadySupplemental = serde_json::from_value(json!({
            "merged_presences": {
                "friends": [
                    {"user_id": "10", "status": "online"},
                    {"user_id": "12", "status": "idle"}
                ]
            }
        }))
        .unwrap();
        store.apply(Event::ReadySupplemental(Box::new(supplemental)));
        assert_eq!(store.online_friends(), [10, 12]);
        assert!(store.user(12).unwrap().status == Status::Idle);
    }

    #[test]
    fn active_now_includes_supplemental_players_and_listeners() {
        let mut store = store();
        let supplemental: model::ReadySupplemental = serde_json::from_value(json!({
            "merged_presences": {
                "friends": [
                    {"user_id": "10", "status": "online", "activities": [{"type": 0, "name": "Example Game"}]},
                    {"user_id": "11", "status": "online", "activities": []}
                ],
                "guilds": [[
                    {"user_id": "40", "status": "online", "activities": [{"type": 0, "name": "Example Action Game"}]},
                    {"user_id": "41", "status": "dnd", "activities": [{"type": 0, "name": "Example Multiplayer Game"}]},
                    {"user_id": "42", "status": "idle", "activities": [{"type": 4, "name": "Custom Status", "state": "hi"}]},
                    {"user_id": "43", "status": "dnd", "activities": [{"type": 2, "name": "Spotify", "details": "Song", "state": "Artist"}]},
                    {"user_id": "44", "status": "offline", "activities": [{"type": 0, "name": "Hidden"}]}
                ]]
            }
        }))
        .unwrap();
        store.apply(Event::ReadySupplemental(Box::new(supplemental)));
        let mut active = store.active_now();
        active.sort_unstable();
        assert_eq!(active, [10, 40, 41, 43]);
    }

    #[test]
    fn private_channel_names_use_group_name_or_recipients() {
        let store = store();
        assert_eq!(store.private_channel_name(900), "Alice");
        assert_eq!(store.private_channel_name(902), "Raid night");
        assert_eq!(store.private_channel_name(903), "Alice, bob, Carol");
        assert_eq!(store.private_channel_name(906), "Frank");
    }

    #[test]
    fn dm_channel_lookup_uses_single_recipient() {
        let store = store();
        assert_eq!(store.dm_channel_with(10), Some(900));
        assert_eq!(store.dm_channel_with(13), None);
    }

    #[test]
    fn dm_messages_use_the_network_channel_path_and_badges() {
        let mut store = store();
        assert!(store.private_channel(900).unwrap().channel.unread);
        assert_eq!(store.private_channel(900).unwrap().channel.mentions, 2);
        assert_eq!(store.dm_mentions, 2);

        store.set_messages(900, vec![message(490, 900, 10), message(500, 900, 10)]);
        assert_eq!(store.network_channel(900).unwrap().messages.len(), 2);
        assert!(store.at_present(900));

        assert!(matches!(
            store.apply(Event::MessageCreate(Box::new(message(501, 900, 10)))),
            Change::MessageInserted {
                channel_id: 900,
                index: 2
            }
        ));
        assert_eq!(store.network_channel(900).unwrap().messages.len(), 3);

        store.apply(Event::MessageCreate(Box::new(message(701, 901, 11))));
        assert!(store.private_channel(901).unwrap().channel.unread);

        store.mark_channel_read(0, 900);
        let channel = &store.private_channel(900).unwrap().channel;
        assert!(!channel.unread);
        assert_eq!(channel.mentions, 0);
        assert_eq!(store.dm_mentions, 0);

        store.apply(Event::MessageCreate(Box::new(message(10_000, 903, 10))));
        assert_eq!(store.dm_list().first().map(|dm| dm.channel.id), Some(903));
    }

    #[test]
    fn relationship_events_update_friends_and_pending() {
        let mut store = store();
        let added: model::Relationship = serde_json::from_value(json!({
            "id": "20",
            "type": 3,
            "user": {"id": "20", "username": "newbie", "global_name": "Newbie"}
        }))
        .unwrap();
        assert!(matches!(
            store.apply(Event::RelationshipAdd(Box::new(added))),
            Change::Structure
        ));
        assert!(store.user(20).is_some());
        assert_eq!(store.pending_count(), 2);

        let update: model::RelationshipUpdate =
            serde_json::from_value(json!({"id": "20", "type": 1})).unwrap();
        store.apply(Event::RelationshipUpdate(Box::new(update)));
        assert!(store.friends().contains(&20));

        let update: model::RelationshipUpdate =
            serde_json::from_value(json!({"id": "11", "nickname": null})).unwrap();
        store.apply(Event::RelationshipUpdate(Box::new(update)));
        assert!(store.relationship(11).unwrap().nickname.is_none());
        assert_eq!(store.friend_name(11), "bob");

        store.apply(Event::RelationshipRemove { id: 10 });
        assert!(!store.friends().contains(&10));
        assert!(store.relationship(10).is_none());
    }

    #[test]
    fn channel_upserts_move_requests_and_register_recipients() {
        let mut store = store();
        let update: model::Channel = serde_json::from_value(json!({
            "id": "904",
            "type": 1,
            "is_message_request": false
        }))
        .unwrap();
        store.apply(Event::ChannelUpsert(Box::new(update)));
        assert!(dm_ids(&store.dm_list()).contains(&904));
        assert_eq!(store.message_request_count(), 0);

        let created: model::Channel = serde_json::from_value(json!({
            "id": "907",
            "type": 1,
            "recipients": [{"id": "30", "username": "zed"}]
        }))
        .unwrap();
        store.apply(Event::ChannelUpsert(Box::new(created)));
        assert!(store.user(30).is_some());
        assert_eq!(store.dm_channel_with(30), Some(907));
    }
}
