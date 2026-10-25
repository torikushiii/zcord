mod mapping;
mod seed;
mod social;
#[cfg(test)]
mod tests;
mod view;

use crate::discord::{self, model, settings_proto};
use crate::images::ImageSpec;
use base64::{Engine, engine::general_purpose::STANDARD};
use mapping::{
    channel_unread, forum_new_posts, local_timestamp, map_components, map_decoration, map_guild,
    map_message, map_name_style, map_nameplate, map_reactions, map_snapshot, map_stickers,
    map_user, read_state, timestamp_from_nonce,
};
use seed::SEED_START;
use std::{cell::RefCell, collections::HashMap};
pub use view::*;

pub struct Store {
    pub guilds: Vec<Guild>,
    pub rail: Vec<RailEntry>,
    pub users: HashMap<u64, User>,
    pub current_user_id: u64,
    pub dm_mentions: u32,
    premium_type: Option<u8>,
    next_message_id: u64,
    source: Option<model::Ready>,
    read_states: HashMap<u64, usize>,
    source_channels: HashMap<u64, SourceChannel>,
    channel_locations: HashMap<u64, ChannelLocation>,
    thread_locations: HashMap<u64, ThreadLocation>,
    forum_posts: HashMap<u64, ForumPost>,
    guild_badges: Vec<GuildBadges>,
    dm_badges: u64,
    guild_indices: HashMap<u64, usize>,
    folder_layout: Option<Vec<settings_proto::GuildFolder>>,
    frecency: Option<Vec<u8>>,
    favorite_gifs: Vec<settings_proto::FavoriteGif>,
    rail_order: Vec<u64>,
    social: social::Social,
    permission_cache: RefCell<HashMap<u64, u64>>,
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
    at_present: bool,
}

#[derive(Clone, Copy)]
struct ThreadLocation {
    guild: usize,
    thread: usize,
    at_present: bool,
}

struct GuildBadges {
    unread_channels: usize,
    mentions: u64,
}

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
        preview: None,
        thumbnail: None,
        unread: false,
    }
}

fn forum_post_thumbnail(message: &model::Message) -> Option<ImageSpec> {
    message
        .attachments
        .iter()
        .find_map(Attachment::image_spec)
        .or_else(|| {
            let embeds = mapping::map_embeds(&message.embeds, message.flags);
            embeds.iter().find_map(|embed| {
                embed
                    .images
                    .first()
                    .or(embed.thumbnail.as_ref())
                    .map(Media::spec)
            })
        })
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

    pub fn guild_banner(&self, guild_id: u64) -> Option<ImageSpec> {
        self.guild_by_id(guild_id)?.banner_spec()
    }

    pub fn has_nitro(&self) -> bool {
        self.premium_type
            .is_some_and(|premium_type| premium_type != 0)
    }

    pub fn emoji_access(&self, emoji: &CustomEmoji, guild_id: u64, channel_id: u64) -> EmojiAccess {
        if !emoji.available {
            return EmojiAccess::Unavailable;
        }
        if !emoji.roles.is_empty()
            && !self
                .member_roles(emoji.guild_id, self.current_user_id)
                .iter()
                .any(|role| emoji.roles.contains(&role.id))
        {
            return EmojiAccess::NeedsRole;
        }
        if emoji.animated && !self.has_nitro() {
            return EmojiAccess::NeedsNitro;
        }
        if emoji.guild_id != guild_id {
            if !self.has_nitro() {
                return EmojiAccess::NeedsNitro;
            }
            let external = self
                .permissions(channel_id)
                .is_some_and(|permissions| permissions & discord::USE_EXTERNAL_EMOJIS != 0);
            if !external {
                return EmojiAccess::NoExternalPermission;
            }
        }
        EmojiAccess::Usable
    }

    pub fn search_emoji(
        &self,
        query: &str,
        guild_id: u64,
        channel_id: u64,
        limit: usize,
    ) -> Vec<(EmojiChoice, EmojiAccess)> {
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let needle = query.to_lowercase();
        let rank = |name: &str| {
            let name = name.to_lowercase();
            if name.starts_with(&needle) {
                Some(true)
            } else if name.contains(&needle) {
                Some(false)
            } else {
                None
            }
        };
        let mut output: Vec<(EmojiChoice, EmojiAccess)> = Vec::new();
        for prefix in [true, false] {
            let Some(guild) = self.guild_by_id(guild_id) else {
                break;
            };
            for emoji in &guild.emojis {
                if rank(&emoji.name) == Some(prefix) {
                    output.push((
                        EmojiChoice::Custom(emoji.clone()),
                        self.emoji_access(emoji, guild_id, channel_id),
                    ));
                    if output.len() >= limit {
                        return output;
                    }
                }
            }
        }
        for prefix in [true, false] {
            for guild in &self.guilds {
                if guild.id == guild_id {
                    continue;
                }
                for emoji in &guild.emojis {
                    if rank(&emoji.name) == Some(prefix) {
                        output.push((
                            EmojiChoice::Custom(emoji.clone()),
                            self.emoji_access(emoji, guild_id, channel_id),
                        ));
                        if output.len() >= limit {
                            return output;
                        }
                    }
                }
            }
        }
        for prefix in [true, false] {
            for &(shortcode, emoji) in crate::emoji::shortcodes() {
                if rank(shortcode) == Some(prefix) {
                    output.push((
                        EmojiChoice::Unicode {
                            emoji: emoji.to_owned(),
                            shortcode: shortcode.to_owned(),
                        },
                        EmojiAccess::Usable,
                    ));
                    if output.len() >= limit {
                        return output;
                    }
                }
            }
        }
        output
    }

    pub fn emoji_guilds(&self) -> Vec<(u64, String, Option<ImageSpec>, Vec<CustomEmoji>)> {
        self.rail
            .iter()
            .flat_map(|entry| match entry {
                RailEntry::Guild(id) => std::slice::from_ref(id),
                RailEntry::Folder(folder) => &folder.guild_ids,
            })
            .filter_map(|id| {
                let guild = self.guild_by_id(*id)?;
                if guild.emojis.is_empty() {
                    return None;
                }
                let mut emojis = guild.emojis.clone();
                emojis.sort_by(|left, right| left.name.cmp(&right.name));
                Some((guild.id, guild.name.clone(), guild.icon_spec(), emojis))
            })
            .collect()
    }

    pub fn reaction_state(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: &ReactionEmoji,
    ) -> Option<bool> {
        let channel = self.network_channel(channel_id)?;
        let index = channel
            .messages
            .binary_search_by_key(&message_id, |message| message.id)
            .ok()?;
        channel.messages[index]
            .reactions
            .iter()
            .find(|reaction| Self::same_view_emoji(&reaction.emoji, emoji))
            .map(|reaction| reaction.me)
    }

    pub fn set_reaction(
        &mut self,
        channel_id: u64,
        message_id: u64,
        emoji: &ReactionEmoji,
        me: bool,
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
        let reactions = &mut channel.messages[index].reactions;
        let existing = reactions
            .iter()
            .position(|reaction| Self::same_view_emoji(&reaction.emoji, emoji));
        if me {
            match existing {
                Some(reaction) => {
                    if reactions[reaction].me {
                        return Change::None;
                    }
                    reactions[reaction].me = true;
                    reactions[reaction].count = reactions[reaction].count.saturating_add(1);
                }
                None => reactions.push(Reaction {
                    emoji: emoji.clone(),
                    count: 1,
                    me: true,
                }),
            }
        } else {
            let Some(reaction) = existing else {
                return Change::None;
            };
            if !reactions[reaction].me {
                return Change::None;
            }
            reactions[reaction].me = false;
            reactions[reaction].count = reactions[reaction].count.saturating_sub(1);
            if reactions[reaction].count == 0 {
                reactions.remove(reaction);
            }
        }
        Change::MessageChanged { channel_id, index }
    }

    fn same_view_emoji(left: &ReactionEmoji, right: &ReactionEmoji) -> bool {
        match (left.id, right.id) {
            (Some(left), Some(right)) => left == right,
            (None, None) => left.name == right.name,
            _ => false,
        }
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

    pub fn unread_acks(&self, guild_id: u64, _now_snowflake: u64) -> Vec<(u64, u64)> {
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
                    Some(15 | 16) => self.latest_forum_post_id(channel.id)?,
                    _ => return None,
                };
                let unread =
                    channel_unread(channel, read_state(source, &self.read_states, channel.id))
                        || (matches!(channel.kind, Some(15 | 16))
                            && self.forum_new_post_count(channel.id) > 0);
                (discord::can_view(guild, channel, source.user.id) && unread)
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
            .filter(|channel| channel.kind.is_text())?;
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

    fn network_channel(&self, channel_id: u64) -> Option<&Channel> {
        if let Some(location) = self.channel_locations.get(&channel_id).copied() {
            return self
                .guilds
                .get(location.guild)?
                .categories
                .get(location.category)?
                .channels
                .get(location.channel);
        }
        let location = *self.thread_locations.get(&channel_id)?;
        self.guilds
            .get(location.guild)?
            .threads
            .get(location.thread)
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
                stickers: Vec::new(),
                kind: 0,
                edited: false,
                reply,
                forward: None,
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

    fn permissions(&self, channel_id: u64) -> Option<u64> {
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
        if self
            .source_channel_guild(channel_id)
            .is_some_and(|(_, channel)| matches!(channel.kind, Some(15 | 16)))
        {
            return false;
        }
        if matches!(
            self.source_channels.get(&channel_id),
            Some(SourceChannel::Private(_))
        ) {
            return true;
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

    pub fn set_frecency(&mut self, proto: Vec<u8>) {
        match settings_proto::favorite_gifs(&proto) {
            Ok(gifs) => {
                self.frecency = Some(proto);
                self.favorite_gifs = gifs;
            }
            Err(_) => {
                self.frecency = None;
                self.favorite_gifs.clear();
            }
        }
    }

    pub fn frecency(&self) -> Option<&[u8]> {
        self.frecency.as_deref()
    }

    pub fn favorite_gifs(&self) -> &[settings_proto::FavoriteGif] {
        &self.favorite_gifs
    }

    pub fn is_favorite_gif(&self, url: &str) -> bool {
        self.favorite_gifs.iter().any(|gif| gif.url == url)
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
    FavoriteGifs,
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
        let premium_type = ready.user.premium_type;
        let mut store = Self {
            guilds,
            rail: Vec::new(),
            users,
            current_user_id: ready.user.id,
            dm_mentions: 0,
            premium_type,
            next_message_id: 1,
            source: Some(ready),
            read_states,
            source_channels: HashMap::new(),
            channel_locations: HashMap::new(),
            thread_locations: HashMap::new(),
            forum_posts: HashMap::new(),
            guild_badges: Vec::new(),
            dm_badges: 0,
            guild_indices: HashMap::new(),
            folder_layout,
            frecency: None,
            favorite_gifs: Vec::new(),
            rail_order,
            social: social::Social::default(),
            permission_cache: RefCell::new(HashMap::new()),
        };
        store.init_social();
        store.apply_own_status(settings.status.as_ref(), false);
        store.index_channels();
        store
    }

    fn index_channels(&mut self) {
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

    fn index_forum_posts(&mut self) {
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

    fn forum_new_post_count(&self, forum_id: u64) -> u32 {
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

    fn latest_forum_post_id(&self, forum_id: u64) -> Option<u64> {
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

    pub fn forum_posts(&self, forum_id: u64) -> Vec<&ForumPost> {
        let creation_sort = self
            .source_channel_guild(forum_id)
            .is_some_and(|(_, channel)| channel.default_sort_order == Some(1));
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

    fn apply_forum_first_message(&mut self, message: &model::Message) {
        let Some(post) = self.forum_posts.get_mut(&message.channel_id) else {
            return;
        };
        if post.preview.is_none() {
            post.preview = Some(message.content.clone());
        }
        if post.thumbnail.is_none() {
            post.thumbnail = forum_post_thumbnail(message);
        }
    }

    fn update_forum_post_message(&mut self, message: &model::Message) {
        let read_last = self
            .source
            .as_ref()
            .and_then(|source| read_state(source, &self.read_states, message.channel_id))
            .and_then(|state| state.last_message_id)
            .unwrap_or(0);
        let timestamp =
            local_timestamp(&message.timestamp).unwrap_or_else(|| timestamp_from_nonce(message.id));
        let thumbnail = forum_post_thumbnail(message);
        let Some(post) = self.forum_posts.get_mut(&message.channel_id) else {
            return;
        };
        post.message_count = post.message_count.saturating_add(1);
        post.last_activity = timestamp;
        post.unread = message.id > read_last;
        if post.preview.is_none() {
            post.preview = Some(message.content.clone());
        }
        if post.thumbnail.is_none() {
            post.thumbnail = thumbnail;
        }
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

    fn apply_user_update(&mut self, update: &model::UserUpdate) -> bool {
        let Some(user) = self.users.get_mut(&update.id) else {
            return false;
        };
        mapping::apply_user_update(user, update);
        true
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
            mapped.custom_status.clone_from(&existing.custom_status);
            mapped.playing = existing.playing;
            mapped.game.clone_from(&existing.game);
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
        if let Some(location) = self.channel_locations.get(&channel_id).copied() {
            return self
                .guilds
                .get_mut(location.guild)?
                .categories
                .get_mut(location.category)?
                .channels
                .get_mut(location.channel);
        }
        let location = *self.thread_locations.get(&channel_id)?;
        self.guilds
            .get_mut(location.guild)?
            .threads
            .get_mut(location.thread)
    }

    fn replace_messages(&mut self, channel_id: u64, mut messages: Vec<model::Message>) {
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

    pub fn append_messages(
        &mut self,
        channel_id: u64,
        mut messages: Vec<model::Message>,
        limit: usize,
    ) -> usize {
        let complete = messages.len() < limit;
        let newest = self
            .network_channel(channel_id)
            .and_then(|channel| channel.messages.last())
            .map_or(0, |message| message.id);
        messages.retain(|m| m.channel_id == channel_id && m.id > newest);
        messages.sort_by_key(|m| m.id);
        messages.dedup_by_key(|m| m.id);
        let mapped: Vec<_> = messages
            .iter()
            .filter_map(|message| self.map_network_message(message, true))
            .collect();
        let count = mapped.len();
        if let Some(channel) = self.network_channel_mut(channel_id) {
            channel.messages.extend(mapped);
        }
        let latest = self.latest_message_id(channel_id);
        let reached = self
            .network_channel(channel_id)
            .and_then(|channel| channel.messages.last())
            .is_some_and(|message| message.id >= latest);
        if complete || reached {
            self.set_at_present(channel_id, true);
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

    fn increment_mentions(&mut self, channel_id: u64, message_id: u64) {
        if let Some(state) = self.read_state_mut(channel_id, message_id) {
            state.mention_count = state.mention_count.saturating_add(1);
        }
    }

    fn refresh_badges(&mut self, channel_id: u64, previous_mentions: u32) {
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

    fn apply_frecency(&mut self, update: &model::SettingsUpdate) -> Change {
        let previous = self.favorite_gifs.clone();
        let decoded = STANDARD.decode(&update.settings.proto);
        let proto = match (update.partial, decoded) {
            (false, Ok(proto)) => Some(proto),
            (false, Err(_)) => None,
            (true, Ok(update)) => {
                let Some(base) = self.frecency.as_deref() else {
                    return Change::None;
                };
                match settings_proto::merge_partial(base, &update) {
                    Ok(merged) => Some(merged),
                    Err(_) => return Change::None,
                }
            }
            (true, Err(_)) => return Change::None,
        };
        match proto {
            Some(proto) => self.set_frecency(proto),
            None => {
                self.frecency = None;
                self.favorite_gifs.clear();
            }
        }
        if self.favorite_gifs == previous {
            Change::None
        } else {
            Change::FavoriteGifs
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

    pub fn apply(&mut self, event: discord::Event) -> Change {
        let revision = self.social.revision;
        let change = self.apply_inner(event);
        let change = if revision != self.social.revision {
            Change::Structure
        } else {
            change
        };
        if matches!(change, Change::Structure) {
            self.permission_cache.get_mut().clear();
        }
        change
    }

    fn apply_inner(&mut self, event: discord::Event) -> Change {
        use discord::Event;
        let event = match self.apply_social(event) {
            Ok(change) => return change,
            Err(event) => event,
        };
        match event {
            Event::Ready(ready) => {
                let sessions = ready.sessions.clone();
                *self = Self::from_ready(*ready);
                self.apply_sessions(&sessions);
                Change::Structure
            }
            Event::SettingsUpdate(update) => {
                // Proto type 1 is PreloadedUserSettings, type 2 is FrecencyUserSettings.
                match update.settings.kind {
                    1 => {}
                    2 => return self.apply_frecency(&update),
                    _ => return Change::None,
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
                let present = self
                    .channel_locations
                    .get(&message.channel_id)
                    .is_none_or(|location| location.at_present);
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
                if !present {
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
            Event::MessageReactionAdd(event) => {
                let current_user_id = self.current_user_id;
                let own = event.user_id == Some(current_user_id);
                self.change_reactions(event.channel_id, event.message_id, |reactions| {
                    // Our optimistic toggle already added it; the echo must not double count.
                    if own
                        && reactions.iter().any(|reaction| {
                            reaction.me && Self::same_emoji(&reaction.emoji, &event.emoji)
                        })
                    {
                        return;
                    }
                    Self::add_reaction(reactions, &event.emoji, 1, own);
                })
            }
            Event::MessageReactionRemove(event) => {
                let current_user_id = self.current_user_id;
                let own = event.user_id == Some(current_user_id);
                self.change_reactions(event.channel_id, event.message_id, |reactions| {
                    if let Some(index) = reactions
                        .iter()
                        .position(|r| Self::same_emoji(&r.emoji, &event.emoji))
                    {
                        // Our optimistic toggle already removed it.
                        if own && !reactions[index].me {
                            return;
                        }
                        let reaction = &mut reactions[index];
                        reaction.count = reaction.count.saturating_sub(1);
                        if own {
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
                        let own = reaction.users.contains(&current_user_id);
                        let mut count = reaction.users.len().try_into().unwrap_or(u32::MAX);
                        if own
                            && reactions
                                .iter()
                                .any(|r| r.me && Self::same_emoji(&r.emoji, &reaction.emoji))
                        {
                            count = count.saturating_sub(1);
                        }
                        Self::add_reaction(reactions, &reaction.emoji, count, own);
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
            Event::GuildEmojisUpdate(update) => {
                if let Some(guild) = self
                    .source
                    .as_mut()
                    .and_then(|source| source.guilds.iter_mut().find(|g| g.id == update.guild_id))
                {
                    guild.emojis = Some(update.emojis);
                    let id = update.guild_id;
                    self.rebuild_structure(Some(id));
                    Change::Structure
                } else {
                    Change::None
                }
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
            Event::ThreadListSync(sync) => {
                let sync = *sync;
                let guild_id = sync.guild_id;
                if let Some(source) = &mut self.source {
                    if let Some(guild) = source.guilds.iter_mut().find(|g| g.id == guild_id) {
                        let threads = guild.threads.get_or_insert_with(Vec::new);
                        for thread in sync.threads {
                            if let Some(existing) =
                                threads.iter_mut().find(|existing| existing.id == thread.id)
                            {
                                existing.merge(&thread);
                            } else {
                                threads.push(thread);
                            }
                        }
                    }
                }
                self.rebuild_structure(Some(guild_id));
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
            Event::UserUpdate(update) => {
                if self.apply_user_update(&update) {
                    Change::Members { guild_id: None }
                } else {
                    Change::None
                }
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
