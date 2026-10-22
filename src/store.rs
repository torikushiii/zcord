mod mapping;
mod seed;
#[cfg(test)]
mod tests;
mod view;

use crate::discord::{self, model, settings_proto};
use mapping::{
    channel_unread, local_timestamp, map_embed, map_guild, map_message, map_user, read_state,
};
use seed::SEED_START;
use std::collections::HashMap;
pub use view::*;

pub struct Store {
    pub guilds: Vec<Guild>,
    pub rail: Vec<RailEntry>,
    pub users: HashMap<u64, User>,
    pub current_user_id: u64,
    pub dm_mentions: u32,
    next_message_id: u64,
    source: Option<model::Ready>,
    read_states: HashMap<u64, usize>,
    source_channels: HashMap<u64, SourceChannel>,
    channel_locations: HashMap<u64, ChannelLocation>,
    guild_badges: Vec<GuildBadges>,
    dm_badges: u64,
    guild_indices: HashMap<u64, usize>,
    folder_layout: Option<Vec<settings_proto::GuildFolder>>,
    rail_order: Vec<u64>,
}

#[derive(Clone, Copy)]
enum SourceChannel {
    Guild { guild: usize, channel: usize },
    Private(usize),
}

impl SourceChannel {
    fn get(self, source: &model::Ready) -> Option<&model::Channel> {
        match self {
            Self::Guild { guild, channel } => {
                source.guilds.get(guild)?.channels.as_ref()?.get(channel)
            }
            Self::Private(index) => source.private_channels.get(index),
        }
    }

    fn get_mut(self, source: &mut model::Ready) -> Option<&mut model::Channel> {
        match self {
            Self::Guild { guild, channel } => source
                .guilds
                .get_mut(guild)?
                .channels
                .as_mut()?
                .get_mut(channel),
            Self::Private(index) => source.private_channels.get_mut(index),
        }
    }
}

#[derive(Clone, Copy)]
struct ChannelLocation {
    guild: usize,
    category: usize,
    channel: usize,
}

struct GuildBadges {
    unread_channels: usize,
    mentions: u64,
}

impl Store {
    pub fn user(&self, id: u64) -> Option<&User> {
        self.users.get(&id)
    }

    pub fn guild_by_id(&self, id: u64) -> Option<&Guild> {
        self.guilds.get(*self.guild_indices.get(&id)?)
    }

    pub fn mark_guild_read(&mut self, guild_id: u64) {
        if let Some(guild) = self.guilds.iter_mut().find(|guild| guild.id == guild_id) {
            guild.unread = false;
            guild.mentions = 0;
        }
    }

    pub fn mark_channel_read(&mut self, guild_id: u64, channel_id: u64) {
        if let Some(location) = self.channel_locations.get(&channel_id).copied() {
            let channel = &self.guilds[location.guild].categories[location.category].channels
                [location.channel];
            if self.guilds[location.guild].id == guild_id && channel.unread {
                self.guild_badges[location.guild].unread_channels = self.guild_badges
                    [location.guild]
                    .unread_channels
                    .saturating_sub(1);
            }
        }
        if let Some(channel) = self
            .guilds
            .iter_mut()
            .find(|guild| guild.id == guild_id)
            .and_then(|guild| guild.channel_mut(channel_id))
        {
            channel.unread = false;
        }
    }

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
            .filter(|channel| channel.kind == ChannelKind::Text)?;
        let timestamp = channel
            .messages
            .last()
            .map_or(SEED_START, |message| message.timestamp.plus_minutes(1));
        let index = channel.messages.len();
        channel.messages.push(Message {
            id: self.next_message_id,
            author_id: self.current_user_id,
            timestamp,
            content,
            embed: None,
            attachments: Vec::new(),
        });
        self.next_message_id += 1;
        Some(index)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    None,
    MessageInserted { channel_id: u64, index: usize },
    MessageChanged { channel_id: u64, index: usize },
    MessageRemoved { channel_id: u64, index: usize },
    Structure,
}

impl Store {
    pub fn from_ready(ready: model::Ready) -> Self {
        let folder_layout = match ready
            .user_settings_proto
            .as_deref()
            .map(settings_proto::decode_base64)
        {
            Some(Ok(Some(folders))) => Some(folders),
            _ => {
                log::warn!("Server folder settings unavailable; using READY order");
                None
            }
        };
        let rail_order = ready.guilds.iter().map(|guild| guild.id).collect();
        let mut read_states = HashMap::with_capacity(ready.read_state.entries.len());
        for (index, state) in ready
            .read_state
            .entries
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind == 0)
        {
            read_states.entry(state.id).or_insert(index);
        }
        let mut users: HashMap<u64, User> = ready
            .users
            .iter()
            .chain(std::iter::once(&ready.user))
            .map(|u| (u.id, map_user(u)))
            .collect();
        for user in ready
            .guilds
            .iter()
            .flat_map(|g| g.members.iter().flatten())
            .filter_map(|m| m.user.as_ref())
        {
            users.insert(user.id, map_user(user));
        }
        let guilds = ready
            .guilds
            .iter()
            .map(|g| map_guild(g, &ready, &read_states))
            .collect();
        let mut store = Self {
            guilds,
            rail: Vec::new(),
            users,
            current_user_id: ready.user.id,
            dm_mentions: 0,
            next_message_id: 1,
            source: Some(ready),
            read_states,
            source_channels: HashMap::new(),
            channel_locations: HashMap::new(),
            guild_badges: Vec::new(),
            dm_badges: 0,
            guild_indices: HashMap::new(),
            folder_layout,
            rail_order,
        };
        store.index_channels();
        store
    }

    fn index_channels(&mut self) {
        self.source_channels.clear();
        self.channel_locations.clear();
        self.guild_badges.clear();
        self.guild_indices.clear();
        if let Some(source) = &self.source {
            for (guild, metadata) in source.guilds.iter().enumerate() {
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
        for (guild_index, guild) in self.guilds.iter().enumerate() {
            self.guild_indices.insert(guild.id, guild_index);
            let mut badges = GuildBadges {
                unread_channels: 0,
                mentions: 0,
            };
            for (category_index, category) in guild.categories.iter().enumerate() {
                for (channel_index, channel) in category.channels.iter().enumerate() {
                    self.channel_locations.insert(
                        channel.id,
                        ChannelLocation {
                            guild: guild_index,
                            category: category_index,
                            channel: channel_index,
                        },
                    );
                    badges.unread_channels +=
                        usize::from(channel.kind == ChannelKind::Text && channel.unread);
                    if let Some(source) = &self.source {
                        badges.mentions += u64::from(
                            read_state(source, &self.read_states, channel.id)
                                .map_or(0, |s| s.mention_count),
                        );
                    }
                }
            }
            if self.source.is_none() {
                badges.mentions = u64::from(guild.mentions);
            }
            self.guild_badges.push(badges);
        }
        self.rebuild_rail();
    }

    fn rebuild_rail(&mut self) {
        let mut listed = std::collections::HashSet::new();
        let mut ordered = Vec::new();
        for folder in self.folder_layout.iter().flatten() {
            let ids: Vec<_> = folder
                .guild_ids
                .iter()
                .copied()
                .filter(|id| self.guild_indices.contains_key(id) && listed.insert(*id))
                .collect();
            if ids.is_empty() {
                continue;
            }
            if let Some(id) = folder.id {
                ordered.push(RailEntry::Folder(Folder {
                    id,
                    name: folder.name.clone(),
                    color: folder.color,
                    guild_ids: ids,
                }));
            } else {
                ordered.extend(ids.into_iter().map(RailEntry::Guild));
            }
        }
        self.rail = self
            .rail_order
            .iter()
            .copied()
            .filter(|id| self.guild_indices.contains_key(id) && !listed.contains(id))
            .map(RailEntry::Guild)
            .chain(ordered)
            .collect();
    }

    fn register_author(&mut self, user: &model::User) {
        self.users.insert(user.id, map_user(user));
    }

    fn network_channel_mut(&mut self, channel_id: u64) -> Option<&mut Channel> {
        let location = self.channel_locations.get(&channel_id)?;
        self.guilds
            .get_mut(location.guild)?
            .categories
            .get_mut(location.category)?
            .channels
            .get_mut(location.channel)
    }

    pub fn set_messages(&mut self, channel_id: u64, mut messages: Vec<model::Message>) {
        messages.retain(|m| m.channel_id == channel_id);
        messages.sort_by_key(|m| m.id);
        messages.dedup_by_key(|m| m.id);
        for message in &messages {
            self.register_author(&message.author);
        }
        if let Some(channel) = self.network_channel_mut(channel_id) {
            channel.messages = messages.iter().filter_map(map_message).collect();
            channel.messages_loaded = true;
        }
    }

    fn rebuild_structure(&mut self, guild_id: Option<u64>) {
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
                    self.users.insert(user.id, map_user(user));
                }
                if let Some(position) = position {
                    let mut histories: HashMap<_, _> = self.guilds[position]
                        .categories
                        .iter_mut()
                        .flat_map(|c| &mut c.channels)
                        .map(|c| (c.id, (c.messages_loaded, std::mem::take(&mut c.messages))))
                        .collect();
                    for channel in rebuilt.categories.iter_mut().flat_map(|c| &mut c.channels) {
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

    fn mention_count(&self, channel_id: u64) -> u32 {
        self.source
            .as_ref()
            .and_then(|source| read_state(source, &self.read_states, channel_id))
            .map_or(0, |s| s.mention_count)
    }

    fn latest_message_id(&self, channel_id: u64) -> u64 {
        self.source
            .as_ref()
            .and_then(|source| self.source_channels.get(&channel_id)?.get(source))
            .and_then(|c| c.last_message_id.flatten())
            .unwrap_or(0)
    }

    fn read_state_mut(
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
            });
            index
        });
        source.read_state.entries.get_mut(index)
    }

    fn refresh_badges(&mut self, channel_id: u64, previous_mentions: u32) {
        let mentions = self.mention_count(channel_id);
        if let Some(location) = self.channel_locations.get(&channel_id).copied() {
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
            let guild = &mut self.guilds[location.guild];
            let channel = &mut guild.categories[location.category].channels[location.channel];
            let was_unread = channel.kind == ChannelKind::Text && channel.unread;
            channel.unread = channel.kind == ChannelKind::Text && unread;
            let badges = &mut self.guild_badges[location.guild];
            badges.unread_channels = badges
                .unread_channels
                .saturating_sub(usize::from(was_unread))
                + usize::from(channel.unread);
            badges.mentions =
                badges.mentions.saturating_sub(u64::from(previous_mentions)) + u64::from(mentions);
            guild.unread = badges.unread_channels > 0;
            guild.mentions = badges.mentions.min(u64::from(u32::MAX)) as u32;
        } else if self
            .source
            .as_ref()
            .and_then(|s| self.source_channels.get(&channel_id)?.get(s))
            .is_some_and(|c| matches!(c.kind, Some(1 | 3)))
        {
            self.dm_badges =
                self.dm_badges.saturating_sub(u64::from(previous_mentions)) + u64::from(mentions);
            self.dm_mentions = self.dm_badges.min(u64::from(u32::MAX)) as u32;
        }
    }

    pub fn apply(&mut self, event: discord::Event) -> Change {
        use discord::Event;
        match event {
            Event::Ready(ready) => {
                *self = Self::from_ready(*ready);
                Change::Structure
            }
            Event::SettingsUpdate(update) => {
                if update.settings.kind != 1 {
                    return Change::None;
                }
                match settings_proto::decode_base64(&update.settings.proto) {
                    Ok(None) if update.partial => return Change::None,
                    Ok(Some(folders)) => self.folder_layout = Some(folders),
                    _ => {
                        log::warn!("Server folder settings unavailable; using READY order");
                        self.folder_layout = None;
                    }
                }
                self.rebuild_rail();
                Change::Structure
            }
            Event::MessageCreate(message) => {
                if let Some(channel) = self.network_channel_mut(message.channel_id) {
                    if channel
                        .messages
                        .binary_search_by_key(&message.id, |m| m.id)
                        .is_ok()
                    {
                        return Change::None;
                    }
                }
                self.register_author(&message.author);
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
                self.refresh_badges(message.channel_id, mentions);
                let Some(channel) = self.network_channel_mut(message.channel_id) else {
                    return Change::None;
                };
                if !channel.messages_loaded {
                    return Change::Structure;
                }
                let Some(mapped) = map_message(&message) else {
                    return Change::None;
                };
                let index = channel.messages.partition_point(|m| m.id < message.id);
                channel.messages.insert(index, mapped);
                Change::MessageInserted {
                    channel_id: message.channel_id,
                    index,
                }
            }
            Event::MessageUpdate(update) => {
                if let Some(author) = &update.author {
                    self.register_author(author);
                }
                let Some(channel) = self.network_channel_mut(update.channel_id) else {
                    return Change::None;
                };
                let Ok(index) = channel.messages.binary_search_by_key(&update.id, |m| m.id) else {
                    return Change::None;
                };
                let message = &mut channel.messages[index];
                if let Some(content) = update.content {
                    message.content = content;
                }
                if let Some(author) = update.author {
                    message.author_id = author.id;
                }
                if let Some(timestamp) = update.timestamp.as_deref().and_then(local_timestamp) {
                    message.timestamp = timestamp;
                }
                if let Some(embeds) = update.embeds {
                    message.embed = embeds.first().map(map_embed);
                }
                if let Some(attachments) = update.attachments {
                    message.attachments = attachments;
                }
                Change::MessageChanged {
                    channel_id: update.channel_id,
                    index,
                }
            }
            Event::MessageDelete { channel_id, id } => {
                let Some(channel) = self.network_channel_mut(channel_id) else {
                    return Change::None;
                };
                let Ok(index) = channel.messages.binary_search_by_key(&id, |m| m.id) else {
                    return Change::None;
                };
                channel.messages.remove(index);
                Change::MessageRemoved { channel_id, index }
            }
            Event::MessageAck {
                channel_id,
                message_id,
            } => {
                let latest = self.latest_message_id(channel_id);
                let mentions = self.mention_count(channel_id);
                if let Some(state) = self.read_state_mut(channel_id, message_id) {
                    state.last_message_id = Some(message_id);
                    if message_id >= latest {
                        state.mention_count = 0;
                    }
                }
                self.refresh_badges(channel_id, mentions);
                Change::Structure
            }
            Event::GuildUpsert(update) => {
                let id = update.id;
                if !self.guild_indices.contains_key(&id) {
                    self.rail_order.insert(0, id);
                    for folder in self.folder_layout.iter_mut().flatten() {
                        folder.guild_ids.retain(|guild| *guild != id);
                    }
                }
                if let Some(source) = &mut self.source {
                    if let Some(guild) = source.guilds.iter_mut().find(|g| g.id == id) {
                        guild.merge(&update);
                    } else {
                        source.guilds.push(*update);
                    }
                }
                self.rebuild_structure(Some(id));
                Change::Structure
            }
            Event::GuildDelete { id } => {
                self.rail_order.retain(|guild| *guild != id);
                if let Some(source) = &mut self.source {
                    source.guilds.retain(|g| g.id != id);
                }
                self.rebuild_structure(Some(id));
                Change::Structure
            }
            Event::ChannelUpsert(update) => {
                let mut guild_id = None;
                if let Some(source) = &mut self.source {
                    let existing = self.source_channels.get(&update.id).copied();
                    let guild_index = update
                        .guild_id
                        .and_then(|id| source.guilds.iter().position(|g| g.id == id))
                        .or_else(|| match existing {
                            Some(SourceChannel::Guild { guild, .. }) => Some(guild),
                            _ => None,
                        });
                    if let Some(index) = guild_index {
                        let guild = &mut source.guilds[index];
                        guild_id = Some(guild.id);
                        let channels = guild.channels.get_or_insert_with(Vec::new);
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
                Change::Structure
            }
            Event::ChannelDelete { id } => {
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
                        Some(SourceChannel::Private(_)) => {
                            source.private_channels.retain(|c| c.id != id)
                        }
                        None => {}
                    }
                }
                self.rebuild_structure(guild_id);
                Change::Structure
            }
            _ => Change::None,
        }
    }
}
