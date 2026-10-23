mod mapping;
mod seed;
mod social;
#[cfg(test)]
mod tests;
mod view;

use crate::discord::{self, model, settings_proto};
use mapping::{
    channel_unread, local_timestamp, map_components, map_decoration, map_guild, map_message,
    map_name_style, map_nameplate, map_reactions, map_user, read_state, timestamp_from_nonce,
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
    social: social::Social,
}

#[derive(Clone, Copy)]
enum SourceChannel {
    Guild { guild: usize, channel: usize },
    Thread { guild: usize, channel: usize },
    Private(usize),
}

impl SourceChannel {
    fn get(self, source: &model::Ready) -> Option<&model::Channel> {
        match self {
            Self::Guild { guild, channel } => {
                source.guilds.get(guild)?.channels.as_ref()?.get(channel)
            }
            Self::Thread { guild, channel } => {
                source.guilds.get(guild)?.threads.as_ref()?.get(channel)
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
            Self::Thread { guild, channel } => source
                .guilds
                .get_mut(guild)?
                .threads
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
    fn same_emoji(left: &ReactionEmoji, right: &model::Emoji) -> bool {
        match (left.id, right.id) {
            (Some(left), Some(right)) => left == right,
            (None, None) => Some(left.name.as_str()) == right.name.as_deref(),
            _ => false,
        }
    }

    fn add_reaction(reactions: &mut Vec<Reaction>, emoji: &model::Emoji, count: u32, me: bool) {
        if count == 0 || (emoji.id.is_none() && emoji.name.as_deref().is_none_or(str::is_empty)) {
            return;
        }
        if let Some(reaction) = reactions
            .iter_mut()
            .find(|r| Self::same_emoji(&r.emoji, emoji))
        {
            reaction.count = reaction.count.saturating_add(count);
            reaction.me |= me;
        } else {
            reactions.push(Reaction {
                emoji: mapping::map_emoji(emoji),
                count,
                me,
            });
        }
    }

    fn change_reactions(
        &mut self,
        channel_id: u64,
        message_id: u64,
        update: impl FnOnce(&mut Vec<Reaction>),
    ) -> Change {
        let Some(channel) = self
            .network_channel_mut(channel_id)
            .filter(|channel| channel.messages_loaded)
        else {
            return Change::None;
        };
        let Ok(index) = channel
            .messages
            .binary_search_by_key(&message_id, |message| message.id)
        else {
            return Change::None;
        };
        update(&mut channel.messages[index].reactions);
        Change::MessageChanged { channel_id, index }
    }

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

    pub fn last_read(&self, channel_id: u64) -> Option<u64> {
        let source = self.source.as_ref()?;
        read_state(source, &self.read_states, channel_id)?.last_message_id
    }

    pub fn channel_ack(&self, channel_id: u64) -> Option<u64> {
        let source = self.source.as_ref()?;
        let channel = self.source_channels.get(&channel_id)?.get(source)?;
        channel_unread(channel, read_state(source, &self.read_states, channel_id))
            .then(|| channel.last_message_id.flatten())
            .flatten()
    }

    pub fn unread_acks(&self, guild_id: u64, now_snowflake: u64) -> Vec<(u64, u64)> {
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
        guild
            .channels
            .iter()
            .flatten()
            .filter_map(|channel| {
                let message_id = match channel.kind {
                    Some(0 | 5) => channel.last_message_id.flatten()?,
                    Some(15 | 16) => now_snowflake,
                    _ => return None,
                };
                (discord::can_view(guild, channel, source.user.id)
                    && channel_unread(channel, read_state(source, &self.read_states, channel.id)))
                .then_some((channel.id, message_id))
            })
            .collect()
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
            delivery: Delivery::Sent,
            content,
            embeds: Vec::new(),
            components: Vec::new(),
            reactions: Vec::new(),
            attachments: Vec::new(),
            kind: 0,
            edited: false,
            reply: None,
            mentions_me: false,
        });
        self.next_message_id += 1;
        Some(index)
    }

    fn network_channel(&self, channel_id: u64) -> Option<&Channel> {
        let location = *self.channel_locations.get(&channel_id)?;
        self.guilds
            .get(location.guild)?
            .categories
            .get(location.category)?
            .channels
            .get(location.channel)
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
                .map(|message| Reply {
                    message_id: message.id,
                    author_id: message.author_id,
                    content: message.content.clone(),
                    attachments: !message.attachments.is_empty(),
                    embeds: !message.embeds.is_empty() || !message.components.is_empty(),
                }) {
                Some(reply) => ReplyState::Loaded(reply),
                None => ReplyState::Unavailable,
            }
        });
        let author_id = self.current_user_id;
        let Some(channel) = self.network_channel_mut(channel_id) else {
            return Change::None;
        };
        if !channel.messages_loaded {
            return Change::None;
        }
        let index = channel
            .messages
            .partition_point(|message| message.id < nonce);
        channel.messages.insert(
            index,
            Message {
                id: nonce,
                author_id,
                timestamp: timestamp_from_nonce(nonce),
                delivery: Delivery::Pending,
                content,
                embeds: Vec::new(),
                components: Vec::new(),
                reactions: Vec::new(),
                attachments: Vec::new(),
                kind: 0,
                edited: false,
                reply,
                mentions_me: false,
            },
        );
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
        let Ok(index) = channel
            .messages
            .binary_search_by_key(&message_id, |message| message.id)
        else {
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
        let Ok(index) = channel
            .messages
            .binary_search_by_key(&nonce, |message| message.id)
        else {
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
        let Ok(index) = channel
            .messages
            .binary_search_by_key(&nonce, |message| message.id)
        else {
            return Change::None;
        };
        if channel.messages[index].delivery == Delivery::Sent {
            return Change::None;
        }
        channel.messages.remove(index);
        Change::MessageRemoved { channel_id, index }
    }

    fn source_channel_guild(&self, channel_id: u64) -> Option<(&model::Guild, &model::Channel)> {
        let source = self.source.as_ref()?;
        let entry = self.source_channels.get(&channel_id).copied()?;
        let (SourceChannel::Guild { guild, .. } | SourceChannel::Thread { guild, .. }) = entry
        else {
            return None;
        };
        Some((source.guilds.get(guild)?, entry.get(source)?))
    }

    pub fn can_send(&self, channel_id: u64) -> bool {
        if self.source.is_none() {
            return true;
        }
        if matches!(
            self.source_channels.get(&channel_id),
            Some(SourceChannel::Private(_))
        ) {
            return true;
        }
        self.source_channel_guild(channel_id)
            .is_some_and(|(guild, channel)| {
                let permissions =
                    discord::channel_permissions(guild, channel, self.current_user_id);
                permissions & (discord::VIEW_CHANNEL | discord::SEND_MESSAGES)
                    == discord::VIEW_CHANNEL | discord::SEND_MESSAGES
            })
    }

    pub fn can_manage_messages(&self, channel_id: u64) -> bool {
        if self.source.is_none() {
            return true;
        }
        self.source_channel_guild(channel_id)
            .is_some_and(|(guild, channel)| {
                discord::channel_permissions(guild, channel, self.current_user_id)
                    & discord::MANAGE_MESSAGES
                    != 0
            })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    None,
    MessageInserted {
        channel_id: u64,
        index: usize,
    },
    MessageChanged {
        channel_id: u64,
        index: usize,
    },
    MessageRemoved {
        channel_id: u64,
        index: usize,
    },
    MessageReplaced {
        channel_id: u64,
        removed: usize,
        inserted: usize,
    },
    Structure,
    Members {
        guild_id: Option<u64>,
    },
}

impl Store {
    pub fn from_ready(ready: model::Ready) -> Self {
        let settings = ready
            .user_settings_proto
            .as_deref()
            .and_then(|proto| settings_proto::decode_base64(proto).ok())
            .unwrap_or_default();
        let folder_layout = settings.folders;
        if folder_layout.is_none() {
            log::warn!("Server folder settings unavailable; using READY order");
        }
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
            social: social::Social::default(),
        };
        store.init_social();
        store.apply_own_status(settings.status.as_ref(), false);
        store.index_channels();
        store
    }

    fn index_channels(&mut self) {
        self.index_names();
        self.source_channels.clear();
        self.channel_locations.clear();
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

    pub fn absorb_user(&mut self, user: &model::User) {
        self.register_author(user);
    }

    fn register_author(&mut self, user: &model::User) {
        if user.username.is_empty() {
            if let Some(existing) = self.users.get_mut(&user.id) {
                if let Some(name) = &user.global_name {
                    existing.name.clone_from(name);
                }
                if let Some(avatar) = &user.avatar {
                    existing.avatar = Some(avatar.clone());
                }
                if let Some(discriminator) = &user.discriminator {
                    existing.discriminator = Some(discriminator.clone());
                }
                existing.bot |= user.bot;
                apply_cosmetics(existing, user);
                return;
            }
        }
        let mut mapped = map_user(user);
        if let Some(existing) = self.users.get(&user.id) {
            mapped.status = existing.status;
            mapped.activity.clone_from(&existing.activity);
            mapped.playing = existing.playing;
            mapped.role_color = existing.role_color;
            if user.primary_guild.is_none() {
                mapped.tag.clone_from(&existing.tag);
            }
            if user.avatar_decoration_data.is_none() {
                mapped.decoration.clone_from(&existing.decoration);
            }
            if user.collectibles.is_none() {
                mapped.nameplate.clone_from(&existing.nameplate);
            }
            if user.display_name_styles.is_none() {
                mapped.name_style.clone_from(&existing.name_style);
            }
        }
        self.users.insert(user.id, mapped);
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
        self.clear_message_info(channel_id);
        let mapped = messages
            .iter()
            .filter_map(|message| self.map_network_message(message, true))
            .collect();
        if let Some(channel) = self.network_channel_mut(channel_id) {
            channel.messages = mapped;
            channel.messages_loaded = true;
        }
    }

    pub fn prepend_messages(
        &mut self,
        channel_id: u64,
        mut messages: Vec<model::Message>,
    ) -> usize {
        let oldest = self
            .network_channel_mut(channel_id)
            .and_then(|channel| channel.messages.first())
            .map_or(u64::MAX, |message| message.id);
        messages.retain(|m| m.channel_id == channel_id && m.id < oldest);
        messages.sort_by_key(|m| m.id);
        messages.dedup_by_key(|m| m.id);
        let mapped: Vec<_> = messages
            .iter()
            .filter_map(|message| self.map_network_message(message, true))
            .collect();
        let count = mapped.len();
        if let Some(channel) = self.network_channel_mut(channel_id) {
            channel.messages.splice(0..0, mapped);
        }
        count
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
                    if !self.users.contains_key(&user.id) {
                        self.users.insert(user.id, map_user(user));
                    }
                }
                if let Some(position) = position {
                    if let Some(social) = self.social.guilds.get(&guild_id) {
                        rebuilt.member_ids = social.members.keys().copied().collect();
                    }
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

    fn apply_ack(&mut self, channel_id: u64, message_id: u64) {
        let latest = self.latest_message_id(channel_id);
        let mentions = self.mention_count(channel_id);
        if let Some(state) = self.read_state_mut(channel_id, message_id) {
            state.last_message_id = Some(message_id);
            if message_id >= latest {
                state.mention_count = 0;
            }
        }
        self.refresh_badges(channel_id, mentions);
    }

    pub fn apply(&mut self, event: discord::Event) -> Change {
        let revision = self.social.revision;
        let change = self.apply_inner(event);
        if revision != self.social.revision {
            Change::Structure
        } else {
            change
        }
    }

    fn apply_inner(&mut self, event: discord::Event) -> Change {
        use discord::Event;
        let event = match self.apply_social(event) {
            Ok(change) => return change,
            Err(event) => event,
        };
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
                    Ok(settings) => {
                        let structure = settings.folders.is_some() || !update.partial;
                        let status = settings.status.is_some() || !update.partial;
                        if settings.folders.is_some() || !update.partial {
                            self.folder_layout = settings.folders;
                        }
                        if status {
                            self.apply_own_status(settings.status.as_ref(), update.partial);
                        }
                        if structure {
                            self.rebuild_rail();
                            return Change::Structure;
                        }
                        return if status {
                            Change::Members { guild_id: None }
                        } else {
                            Change::None
                        };
                    }
                    Err(_) => {
                        log::warn!("Server folder settings unavailable; using READY order");
                        self.folder_layout = None;
                    }
                }
                self.rebuild_rail();
                Change::Structure
            }
            Event::MessageCreate(message) => {
                let mut loaded = false;
                if let Some(channel) = self.network_channel_mut(message.channel_id) {
                    loaded = channel.messages_loaded;
                    if channel
                        .messages
                        .binary_search_by_key(&message.id, |m| m.id)
                        .is_ok()
                    {
                        return Change::None;
                    }
                }
                let mapped = self.map_network_message(&message, loaded);
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
                let current_user_id = self.current_user_id;
                let own = message.author.id == current_user_id;
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
                let Some(mapped) = mapped else {
                    return Change::None;
                };
                let pending = if own {
                    message.nonce.and_then(|nonce| {
                        channel.messages.iter().position(|m| {
                            m.id == nonce
                                && m.delivery != Delivery::Sent
                                && m.author_id == current_user_id
                        })
                    })
                } else {
                    None
                };
                if let Some(removed) = pending {
                    channel.messages.remove(removed);
                    let inserted = channel.messages.partition_point(|m| m.id < mapped.id);
                    channel.messages.insert(inserted, mapped);
                    return Change::MessageReplaced {
                        channel_id: message.channel_id,
                        removed,
                        inserted,
                    };
                }
                let index = channel.messages.partition_point(|m| m.id < message.id);
                channel.messages.insert(index, mapped);
                Change::MessageInserted {
                    channel_id: message.channel_id,
                    index,
                }
            }
            Event::MessageUpdate(update) => {
                let (reply, mentions_me, embeds) = self.update_message_info(&update);
                let Some(channel) = self.network_channel_mut(update.channel_id) else {
                    return Change::None;
                };
                let Ok(index) = channel.messages.binary_search_by_key(&update.id, |m| m.id) else {
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
                Change::MessageChanged {
                    channel_id: update.channel_id,
                    index,
                }
            }
            Event::MessageReactionAdd(event) => {
                let current_user_id = self.current_user_id;
                self.change_reactions(event.channel_id, event.message_id, |reactions| {
                    Self::add_reaction(
                        reactions,
                        &event.emoji,
                        1,
                        event.user_id == Some(current_user_id),
                    );
                })
            }
            Event::MessageReactionRemove(event) => {
                let current_user_id = self.current_user_id;
                self.change_reactions(event.channel_id, event.message_id, |reactions| {
                    if let Some(index) = reactions
                        .iter()
                        .position(|r| Self::same_emoji(&r.emoji, &event.emoji))
                    {
                        let reaction = &mut reactions[index];
                        reaction.count = reaction.count.saturating_sub(1);
                        if event.user_id == Some(current_user_id) {
                            reaction.me = false;
                        }
                        if reaction.count == 0 {
                            reactions.remove(index);
                        }
                    }
                })
            }
            Event::MessageReactionRemoveAll(event) => {
                self.change_reactions(event.channel_id, event.message_id, Vec::clear)
            }
            Event::MessageReactionRemoveEmoji(event) => {
                self.change_reactions(event.channel_id, event.message_id, |reactions| {
                    reactions.retain(|r| !Self::same_emoji(&r.emoji, &event.emoji));
                })
            }
            Event::MessageReactionAddMany(event) => {
                let current_user_id = self.current_user_id;
                self.change_reactions(event.channel_id, event.message_id, |reactions| {
                    for reaction in event.reactions {
                        Self::add_reaction(
                            reactions,
                            &reaction.emoji,
                            reaction.users.len().try_into().unwrap_or(u32::MAX),
                            reaction.users.contains(&current_user_id),
                        );
                    }
                })
            }
            Event::MessageDelete { channel_id, id } => {
                self.forget_message_info(channel_id, id);
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
                self.apply_ack(channel_id, message_id);
                Change::Structure
            }
            Event::TypingStart(typing) => {
                if let Some(member) = &typing.member {
                    match typing.guild_id {
                        Some(guild_id) => {
                            self.upsert_member(guild_id, typing.user_id, member);
                        }
                        None => {
                            if let Some(user) = &member.user {
                                self.register_author(user);
                            }
                        }
                    }
                }
                Change::None
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
                self.sync_guild_social(id);
                self.rebuild_structure(Some(id));
                Change::Structure
            }
            Event::GuildDelete { id } => {
                self.social.guilds.remove(&id);
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
                            Some(
                                SourceChannel::Guild { guild, .. }
                                | SourceChannel::Thread { guild, .. },
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
                Change::Structure
            }
            Event::ChannelDelete { id } => {
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

// Partial user objects omit these fields when unchanged; only an explicit null clears them.
fn apply_cosmetics(target: &mut User, user: &model::User) {
    if let Some(decoration) = &user.avatar_decoration_data {
        target.decoration = decoration.as_ref().map(map_decoration);
    }
    if let Some(collectibles) = &user.collectibles {
        target.nameplate = collectibles.as_ref().and_then(map_nameplate);
    }
    if let Some(styles) = &user.display_name_styles {
        target.name_style = styles.as_ref().map(map_name_style);
    }
}
