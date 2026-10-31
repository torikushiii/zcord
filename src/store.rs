mod account;
mod authors;
mod channels;
mod emoji;
mod expressions;
mod forum;
mod guilds;
mod home;
mod mapping;
mod messages;
mod reactions;
mod read_state;
mod seed;
mod settings;
mod social;
#[cfg(test)]
mod tests;
mod view;
mod voice;

use crate::discord::{self, model, settings_proto};
use crate::media::images::ImageSpec;
pub use account::Account;
use base64::{Engine, engine::general_purpose::STANDARD};
use home::PrivateLocation;
use mapping::{
    channel_unread, forum_new_posts, local_timestamp, map_components, map_decoration, map_guild,
    map_message, map_name_style, map_nameplate, map_reactions, map_snapshot, map_stickers,
    map_user, read_state, timestamp_from_nonce,
};
use seed::SEED_START;
use std::{cell::RefCell, collections::HashMap};
pub use view::*;
pub use voice::{VoiceActivity, VoiceMember};

pub struct Store {
    pub guilds: Vec<Guild>,
    pub rail: Vec<RailEntry>,
    pub users: HashMap<u64, User>,
    pub current_user_id: u64,
    account: Option<Account>,
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
    private: Vec<PrivateChannel>,
    private_locations: HashMap<u64, PrivateLocation>,
    relationships: HashMap<u64, Relationship>,
    request_previews: HashMap<u64, RequestPreview>,
    pub friend_suggestion_count: u32,
    guild_indices: HashMap<u64, usize>,
    folder_layout: Option<Vec<settings_proto::GuildFolder>>,
    frecency: Option<Vec<u8>>,
    favorite_gifs: Vec<settings_proto::FavoriteGif>,
    rail_order: Vec<u64>,
    social: social::Social,
    voice_start_times: HashMap<(u64, u64), u64>,
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

impl Store {
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

    pub fn upload_limit(&self, channel_id: u64) -> u64 {
        let premium_tier = self
            .guilds
            .iter()
            .find(|guild| guild.channel(channel_id).is_some())
            .map_or(0, |guild| guild.premium_tier);
        file_limit(self.premium_type, premium_tier)
    }
}

fn file_limit(premium_type: Option<u8>, premium_tier: u8) -> u64 {
    const MIB: u64 = 1024 * 1024;
    let own = match premium_type {
        Some(2) => 500 * MIB,
        Some(1 | 3) => 50 * MIB,
        _ => 20 * MIB,
    };
    let boosted = match premium_tier {
        3 => 100 * MIB,
        2 => 50 * MIB,
        _ => 0,
    };
    own.max(boosted)
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
    VoiceTimers,
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
        let account = Account {
            username: ready.user.username.clone(),
            email: ready.user.email.clone(),
            phone: ready.user.phone.clone(),
            mfa_enabled: ready.user.mfa_enabled,
        };
        let friend_suggestion_count = ready.friend_suggestion_count;
        let mut store = Self {
            guilds,
            rail: Vec::new(),
            users,
            current_user_id: ready.user.id,
            account: Some(account),
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
            private: Vec::new(),
            private_locations: HashMap::new(),
            relationships: HashMap::new(),
            request_previews: HashMap::new(),
            friend_suggestion_count,
            guild_indices: HashMap::new(),
            folder_layout,
            frecency: None,
            favorite_gifs: Vec::new(),
            rail_order,
            social: social::Social::default(),
            voice_start_times: HashMap::new(),
            permission_cache: RefCell::new(HashMap::new()),
        };
        store.init_social();
        store.init_relationships();
        store.apply_own_status(settings.status.as_ref(), false);
        store.index_channels();
        store
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
        let supplemental_voice = match &event {
            Event::ReadySupplemental(update) => self.apply_supplemental_voice(update),
            _ => false,
        };
        let event = match self.apply_social(event) {
            Ok(change) => {
                return if supplemental_voice {
                    Change::Structure
                } else {
                    change
                };
            }
            Err(event) => event,
        };
        match event {
            Event::Ready(ready) => {
                let sessions = ready.sessions.clone();
                *self = Self::from_ready(*ready);
                self.apply_sessions(&sessions);
                Change::Structure
            }
            Event::SettingsUpdate(update) => self.settings_update(update),
            Event::MessageCreate(message) => self.message_create(message),
            Event::MessageUpdate(update) => self.message_update(update),
            Event::MessageReactionAdd(event) => self.reaction_add(event),
            Event::MessageReactionRemove(event) => self.reaction_remove(event),
            Event::MessageReactionRemoveAll(event) => {
                self.change_reactions(event.channel_id, event.message_id, Vec::clear)
            }
            Event::MessageReactionRemoveEmoji(event) => {
                self.change_reactions(event.channel_id, event.message_id, |reactions| {
                    reactions.retain(|r| !Self::same_emoji(&r.emoji, &event.emoji));
                })
            }
            Event::MessageReactionAddMany(event) => self.reaction_add_many(event),
            Event::MessageDelete { channel_id, id } => self.message_delete(channel_id, id),
            Event::MessageAck {
                channel_id,
                message_id,
            } => {
                self.apply_ack(channel_id, message_id);
                Change::Structure
            }
            Event::TypingStart(typing) => self.typing_start(typing),
            Event::GuildUpsert(update) => {
                if let Some(source) = &mut self.source
                    && (update.unavailable == Some(false) || !update.voice_states.is_empty())
                    && let Some(guild) = source.guilds.iter_mut().find(|g| g.id == update.id)
                {
                    guild.voice_states.clone_from(&update.voice_states);
                }
                self.guild_upsert(update)
            }
            Event::VoiceStateUpdate(state) => self.voice_state_update(*state),
            Event::ChannelInfo(info) => self.channel_info(info),
            Event::VoiceChannelStartTimeUpdate(update) => {
                self.voice_start_time_update(update.guild_id, update.id, update.voice_start_time)
            }
            Event::GuildEmojisUpdate(update) => self.guild_emojis_update(update),
            Event::GuildStickersUpdate(update) => self.guild_stickers_update(update),
            Event::GuildDelete { id } => self.guild_delete(id),
            Event::ThreadListSync(sync) => self.thread_list_sync(sync),
            Event::ChannelUpsert(update) => self.channel_upsert(update),
            Event::ChannelDelete { id } => self.channel_delete(id),
            Event::UserUpdate(update) => self.user_update(update),
            Event::RelationshipAdd(relationship) => {
                self.apply_relationship_add(&relationship);
                Change::Structure
            }
            Event::RelationshipUpdate(update) => {
                self.apply_relationship_update(&update);
                Change::Structure
            }
            Event::RelationshipRemove { id } => {
                self.apply_relationship_remove(id);
                Change::Structure
            }
            _ => Change::None,
        }
    }
}
