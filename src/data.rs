use crate::discord::{self, model};
use chrono::{Datelike, Timelike};
use std::collections::HashMap;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Online,
    Idle,
    DoNotDisturb,
    Offline,
}

impl Status {
    pub fn is_online(self) -> bool {
        self != Status::Offline
    }
}

pub struct User {
    pub id: u64,
    pub name: String,
    pub initials: String,
    pub status: Status,
    pub bot: bool,
    pub role_color: Option<u32>,
    pub tag: Option<String>,
    pub activity: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

impl Timestamp {
    fn total_minutes(self) -> u32 {
        (self.day as u32 * 24 + self.hour as u32) * 60 + self.minute as u32
    }

    fn plus_minutes(self, minutes: u32) -> Self {
        let total = self.total_minutes() + minutes;
        Self {
            day: (total / (24 * 60)) as u8,
            hour: (total / 60 % 24) as u8,
            minute: (total % 60) as u8,
            ..self
        }
    }

    pub fn same_day(self, other: Self) -> bool {
        (self.year, self.month, self.day) == (other.year, other.month, other.day)
    }

    pub fn minutes_since(self, earlier: Self) -> u32 {
        self.total_minutes().saturating_sub(earlier.total_minutes())
    }

    pub fn time(self) -> String {
        let hour = match self.hour % 12 {
            0 => 12,
            hour => hour,
        };
        let meridiem = if self.hour < 12 { "AM" } else { "PM" };
        format!("{hour}:{:02} {meridiem}", self.minute)
    }

    pub fn full(self) -> String {
        format!("{}/{}/{} {}", self.month, self.day, self.year, self.time())
    }

    pub fn date(self) -> String {
        let month = MONTHS[(self.month as usize).saturating_sub(1) % 12];
        format!("{month} {}, {}", self.day, self.year)
    }
}

pub struct Embed {
    pub color: u32,
    pub title: String,
    pub lines: Vec<String>,
    pub footer: String,
}

pub struct Message {
    pub id: u64,
    pub author_id: u64,
    pub timestamp: Timestamp,
    pub content: String,
    pub embed: Option<Embed>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    Voice,
}

pub struct Channel {
    pub id: u64,
    pub name: String,
    pub kind: ChannelKind,
    pub unread: bool,
    pub messages: Vec<Message>,
    pub messages_loaded: bool,
}

pub struct Category {
    pub id: u64,
    pub name: String,
    pub channels: Vec<Channel>,
}

pub struct HoistedRole {
    pub name: String,
    pub member_ids: Vec<u64>,
}

pub struct Guild {
    pub id: u64,
    pub name: String,
    pub initials: String,
    pub icon_color: u32,
    pub unread: bool,
    pub mentions: u32,
    pub categories: Vec<Category>,
    pub hoisted_roles: Vec<HoistedRole>,
    pub member_ids: Vec<u64>,
    pub icon: Option<String>,
}

impl Guild {
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.categories
            .iter()
            .flat_map(|category| &category.channels)
    }

    pub fn first_text_channel(&self) -> Option<&Channel> {
        self.channels()
            .find(|channel| channel.kind == ChannelKind::Text)
    }

    pub fn channel(&self, id: u64) -> Option<&Channel> {
        self.channels().find(|channel| channel.id == id)
    }

    fn channel_mut(&mut self, id: u64) -> Option<&mut Channel> {
        self.categories
            .iter_mut()
            .flat_map(|category| &mut category.channels)
            .find(|channel| channel.id == id)
    }
}

pub struct Store {
    pub guilds: Vec<Guild>,
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

const STATUS_BOT_ID: u64 = 9;

impl Store {
    pub fn seed() -> Self {
        use Status::*;
        let users = [
            (
                "Demo User One",
                "D1",
                Online,
                false,
                Some(0xC77DFF),
                Some("DEMO"),
                Some("Building a Discord client"),
            ),
            (
                "Demo User Two",
                "D2",
                Online,
                false,
                Some(0x3BA55C),
                None,
                Some("Playing Example Adventure"),
            ),
            ("Demo User Three", "D3", Idle, false, None, None, None),
            (
                "Demo User Four",
                "D4",
                DoNotDisturb,
                false,
                Some(0xE67E22),
                Some("TEST"),
                Some("Focus mode"),
            ),
            ("Demo User Five", "D5", Online, false, None, None, None),
            ("Demo User Six", "D6", Offline, false, None, None, None),
            (
                "Demo User Seven",
                "D7",
                Offline,
                false,
                Some(0x3BA55C),
                None,
                None,
            ),
            ("Demo User Eight", "D8", Offline, false, None, None, None),
            (
                "Demo Status Bot",
                "SB",
                Online,
                true,
                Some(0xF23F43),
                None,
                Some("Watching over 100 demo users"),
            ),
            ("Demo Helper Bot", "HB", Offline, true, Some(0xF23F43), None, None),
        ]
        .into_iter()
        .enumerate()
        .map(
            |(index, (name, initials, status, bot, role_color, tag, activity))| User {
                id: index as u64 + 1,
                name: name.into(),
                initials: initials.into(),
                status,
                bot,
                role_color,
                tag: tag.map(Into::into),
                activity: activity.map(Into::into),
                avatar: None,
            },
        )
        .collect::<Vec<_>>();

        let snippets = [
            "Good morning! How is everyone's week going?",
            "I've been working on the desktop layout. Having the channels and conversation visible together makes it much easier to keep track of what is happening, even when a few discussions are moving at once.",
            "That sounds great.",
            "One small detail: let's keep the important notes in the channel topic so people joining later can find them without scrolling through the whole conversation.",
            "Agreed! I'll put together a short summary after lunch.",
            "Just got back from a walk. The weather is finally getting cooler here.",
            "Sharing a longer update for anyone catching up: the first draft is ready, the feedback has been collected, and the next step is to review the details together.",
            "Nice work, everyone!",
            "lol",
            "anyone up for raids tonight?",
        ];
        let human_ids = [1_u64, 2, 3, 4, 5, 6, 7];

        let guild_specs: [(&str, &str, u32, &[&str], bool, u32); 6] = [
            (
                "Demo Server One",
                "S1",
                0x5865F2,
                &["general", "introductions", "projects", "show-and-tell"],
                true,
                0,
            ),
            (
                "Demo Server Two",
                "S2",
                0xE67E22,
                &["general", "raids", "gacha-salt", "fan-art"],
                true,
                790,
            ),
            (
                "Demo Server Three",
                "S3",
                0x2F3136,
                &["general", "music", "off-topic"],
                true,
                329,
            ),
            (
                "Demo Server Four",
                "S4",
                0xEB459E,
                &["general", "design", "inspiration", "feedback", "resources"],
                false,
                0,
            ),
            (
                "Demo Server Five",
                "S5",
                0xCE422B,
                &["general", "help", "showcase"],
                true,
                1200,
            ),
            (
                "Demo Server Six",
                "S6",
                0x23A55A,
                &["general", "schedule"],
                false,
                89,
            ),
        ];

        let mut next_message_id = 1;
        let guilds = guild_specs
            .into_iter()
            .enumerate()
            .map(
                |(guild_index, (name, initials, icon_color, channel_names, unread, mentions))| {
                    let guild_id = guild_index as u64 + 1;
                    let text_channels = channel_names
                        .iter()
                        .enumerate()
                        .map(|(channel_index, channel_name)| {
                            let count = if channel_index == 0 {
                                30
                            } else {
                                6 + (guild_index * 7 + channel_index * 5) % 20
                            };
                            let messages = seed_messages(
                                count,
                                guild_index + channel_index,
                                &human_ids,
                                &snippets,
                                &mut next_message_id,
                            );
                            Channel {
                                id: guild_id * 100 + channel_index as u64,
                                name: (*channel_name).into(),
                                kind: ChannelKind::Text,
                                unread: unread && channel_index % 2 == 1,
                                messages,
                                messages_loaded: true,
                            }
                        })
                        .collect();
                    let voice_channels = ["General", "Raid Room"]
                        .into_iter()
                        .take(1 + guild_index % 2)
                        .enumerate()
                        .map(|(index, name)| Channel {
                            id: guild_id * 100 + 50 + index as u64,
                            name: name.into(),
                            kind: ChannelKind::Voice,
                            unread: false,
                            messages: Vec::new(),
                            messages_loaded: false,
                        })
                        .collect();

                    Guild {
                        id: guild_id,
                        name: name.into(),
                        initials: initials.into(),
                        icon_color,
                        unread,
                        mentions,
                        categories: vec![
                            Category {
                                id: guild_id * 10,
                                name: "Text Channels".into(),
                                channels: text_channels,
                            },
                            Category {
                                id: guild_id * 10 + 1,
                                name: "Voice Channels".into(),
                                channels: voice_channels,
                            },
                        ],
                        hoisted_roles: vec![HoistedRole {
                            name: "Bots".into(),
                            member_ids: vec![9, 10],
                        }],
                        member_ids: users.iter().map(|user| user.id).collect(),
                        icon: None,
                    }
                },
            )
            .collect();

        let mut store = Self {
            guilds,
            users: users.into_iter().map(|user| (user.id, user)).collect(),
            current_user_id: 1,
            dm_mentions: 77,
            next_message_id,
            source: None,
            read_states: HashMap::new(),
            source_channels: HashMap::new(),
            channel_locations: HashMap::new(),
            guild_badges: Vec::new(),
            dm_badges: 77,
        };
        store.index_channels();
        store
    }

    pub fn user(&self, id: u64) -> Option<&User> {
        self.users.get(&id)
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

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(3)
        .collect()
}

fn local_timestamp(value: &str) -> Option<Timestamp> {
    let local = chrono::DateTime::parse_from_rfc3339(value)
        .ok()?
        .with_timezone(&chrono::Local);
    Some(Timestamp {
        year: local.year().try_into().ok()?,
        month: local.month() as u8,
        day: local.day() as u8,
        hour: local.hour() as u8,
        minute: local.minute() as u8,
    })
}

fn map_embed(embed: &model::Embed) -> Embed {
    let mut lines: Vec<String> = embed
        .description
        .as_deref()
        .unwrap_or("")
        .lines()
        .map(str::to_owned)
        .collect();
    lines.extend(
        embed
            .fields
            .iter()
            .map(|field| format!("{}: {}", field.name, field.value)),
    );
    Embed {
        color: embed.color.unwrap_or(0x202225),
        title: embed
            .title
            .clone()
            .or_else(|| embed.author.as_ref().map(|a| a.name.clone()))
            .unwrap_or_default(),
        lines,
        footer: embed
            .footer
            .as_ref()
            .map(|f| f.text.clone())
            .unwrap_or_default(),
    }
}

fn map_message(message: &model::Message) -> Option<Message> {
    Some(Message {
        id: message.id,
        author_id: message.author.id,
        timestamp: local_timestamp(&message.timestamp)?,
        content: message.content.clone(),
        embed: message.embeds.first().map(map_embed),
    })
}

fn map_user(user: &model::User) -> User {
    let name = user.global_name.as_ref().unwrap_or(&user.username).clone();
    User {
        id: user.id,
        initials: initials(&name),
        name,
        status: Status::Offline,
        bot: user.bot,
        role_color: None,
        tag: None,
        activity: None,
        avatar: user.avatar.clone(),
    }
}

fn read_state<'a>(
    source: &'a model::Ready,
    index: &HashMap<u64, usize>,
    channel_id: u64,
) -> Option<&'a model::ReadState> {
    source.read_state.entries.get(*index.get(&channel_id)?)
}

fn channel_unread(channel: &model::Channel, state: Option<&model::ReadState>) -> bool {
    state.is_some_and(|state| {
        channel.last_message_id.flatten().unwrap_or(0) > state.last_message_id.unwrap_or(0)
    })
}

fn map_guild(
    guild: &model::Guild,
    source: &model::Ready,
    read_states: &HashMap<u64, usize>,
) -> Guild {
    let channels = guild.channels.as_deref().unwrap_or_default();
    let mut categories: Vec<_> = channels.iter().filter(|c| c.kind == Some(4)).collect();
    categories.sort_by_key(|c| (c.position.unwrap_or(0), c.id));
    let mut mapped = vec![Category {
        id: 0,
        name: String::new(),
        channels: Vec::new(),
    }];
    mapped.extend(categories.iter().map(|c| Category {
        id: c.id,
        name: c.name.clone().unwrap_or_default(),
        channels: Vec::new(),
    }));
    let category_indices: HashMap<_, _> =
        mapped.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
    let mut ordered: Vec<_> = channels
        .iter()
        .filter(|c| {
            matches!(c.kind, Some(0 | 5 | 2 | 13))
                && guild.unavailable != Some(true)
                && discord::can_view(guild, c, source.user.id)
        })
        .collect();
    ordered.sort_by_key(|c| (c.position.unwrap_or(0), c.id));
    let mut mentions = 0u64;
    for channel in ordered {
        let kind = if matches!(channel.kind, Some(2 | 13)) {
            ChannelKind::Voice
        } else {
            ChannelKind::Text
        };
        let state = read_state(source, read_states, channel.id);
        mentions += u64::from(state.map_or(0, |s| s.mention_count));
        let index = channel
            .parent_id
            .flatten()
            .and_then(|id| category_indices.get(&id))
            .copied()
            .unwrap_or(0);
        mapped[index].channels.push(Channel {
            id: channel.id,
            name: channel.name.clone().unwrap_or_default(),
            kind,
            unread: kind == ChannelKind::Text && channel_unread(channel, state),
            messages: Vec::new(),
            messages_loaded: false,
        });
    }
    let visible_categories: std::collections::HashSet<_> = categories
        .iter()
        .filter(|c| discord::can_view(guild, c, source.user.id))
        .map(|c| c.id)
        .collect();
    mapped.retain(|category| {
        !category.channels.is_empty() || visible_categories.contains(&category.id)
    });
    let members = guild.members.as_deref().unwrap_or_default();
    let color = (guild.id.wrapping_mul(0x9E3779B97F4A7C15) >> 32) as u32 & 0xFFFFFF;
    Guild {
        id: guild.id,
        name: guild.name().to_owned(),
        initials: initials(guild.name()),
        icon_color: color,
        unread: mapped
            .iter()
            .flat_map(|c| &c.channels)
            .any(|c| c.kind == ChannelKind::Text && c.unread),
        mentions: mentions.min(u64::from(u32::MAX)) as u32,
        categories: mapped,
        hoisted_roles: Vec::new(),
        member_ids: members.iter().filter_map(|m| m.id()).collect(),
        icon: guild.icon(),
    }
}

impl Store {
    pub fn from_ready(ready: model::Ready) -> Self {
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
        };
        store.index_channels();
        store
    }

    fn index_channels(&mut self) {
        self.source_channels.clear();
        self.channel_locations.clear();
        self.guild_badges.clear();
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

const SEED_START: Timestamp = Timestamp {
    year: 2026,
    month: 10,
    day: 18,
    hour: 21,
    minute: 4,
};

fn seed_messages(
    count: usize,
    salt: usize,
    human_ids: &[u64],
    snippets: &[&str],
    next_message_id: &mut u64,
) -> Vec<Message> {
    let mut messages = Vec::with_capacity(count);
    let mut timestamp = SEED_START.plus_minutes((salt as u32 * 17) % 60);
    let mut burst = 0;
    let mut author = human_ids[salt % human_ids.len()];
    for index in 0..count {
        let (author_id, content, embed) = if index == 2 {
            (STATUS_BOT_ID, String::new(), Some(status_embed(salt)))
        } else {
            if burst == 0 {
                author = human_ids[(index * 3 + salt) % human_ids.len()];
                burst = 1 + (index + salt) % 3;
            }
            burst -= 1;
            (
                author,
                snippets[(index * 7 + salt) % snippets.len()].to_owned(),
                None,
            )
        };
        let gap = match messages.last() {
            Some(Message {
                author_id: previous,
                ..
            }) if *previous == author_id => 1,
            _ => 20 + ((index * 53 + salt * 11) % 240) as u32,
        };
        timestamp = timestamp.plus_minutes(gap);
        messages.push(Message {
            id: *next_message_id,
            author_id,
            timestamp,
            content,
            embed,
        });
        *next_message_id += 1;
    }
    messages
}

fn status_embed(salt: usize) -> Embed {
    let items = [
        ("Arcarum Sephira Evolite", "252 days, 14 hours"),
        ("Sample Item One", "1 year, 238 days"),
        ("Sample Item Two", "16 days, 14 hours"),
        ("Sample Item Three", "92 days, 14 hours"),
        ("Sample Item Four", "1 year, 166 days"),
        ("Sample Item Five", "228 days, 14 hours"),
        ("Sample Item Six", "56 days, 14 hours"),
    ];
    Embed {
        color: [0x23A55A, 0x5865F2, 0xF0B232][salt % 3],
        title: "Item Status".into(),
        lines: items
            .iter()
            .map(|(item, ago)| format!("{ago} ago since last {item}"))
            .collect(),
        footer: "Data Tracker || Demo Status Bot".into(),
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;
    use crate::discord::Event;
    use serde_json::json;

    fn ready() -> model::Ready {
        serde_json::from_str(include_str!("discord/fixtures/ready.json")).unwrap()
    }
    fn messages() -> Vec<model::Message> {
        model::parse_entries(
            serde_json::from_str(include_str!("discord/fixtures/messages.json")).unwrap(),
        )
    }

    #[test]
    fn ready_mapping_order_permissions_and_badges() {
        let ready = ready();
        assert_eq!(
            ready.guilds[1].members.as_ref().unwrap()[0].roles,
            Vec::<u64>::new()
        );
        let mut store = Store::seed();
        assert_eq!(
            store.apply(Event::Ready(Box::new(ready))),
            Change::Structure
        );
        assert_eq!(
            store.guilds.iter().map(|g| g.id).collect::<Vec<_>>(),
            [200, 100]
        );
        let guild = &store.guilds[0];
        assert_eq!(
            guild
                .categories
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["", "Earlier", "Later"]
        );
        assert_eq!(
            guild.categories[1]
                .channels
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            [2003, 2002]
        );
        assert!(guild.channel(2001).is_none());
        assert!(guild.channel(2002).unwrap().unread);
        assert!(!guild.channel(2003).unwrap().unread);
        assert!(!guild.channel(2004).unwrap().unread);
        assert!(guild.channel(2007).is_none());
        assert!(guild.channel(2008).is_none());
        assert_eq!(guild.mentions, 3);
        assert!(guild.unread);
        assert_eq!(store.dm_mentions, 4);
        assert_eq!(guild.initials, "SGN");
        assert_eq!(guild.icon.as_deref(), Some("guild-hash"));
        assert!(store.guilds[1].channel(1001).is_some());
        assert_eq!(store.user(2).unwrap().name, "A Writer");
        assert_eq!(
            store.user(2).unwrap().avatar.as_deref(),
            Some("avatar-hash")
        );
    }

    #[test]
    fn message_history_order_local_time_and_first_embed() {
        let mut store = Store::from_ready(ready());
        assert!(!store.guilds[0].channel(2002).unwrap().messages_loaded);
        store.set_messages(2002, messages());
        let channel = store.guilds[0].channel(2002).unwrap();
        assert!(channel.messages_loaded);
        assert_eq!(
            channel.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            [61, 62]
        );
        let timestamp = channel.messages[1].timestamp;
        let expected = chrono::DateTime::parse_from_rfc3339("2026-10-22T12:34:56.123+00:00")
            .unwrap()
            .with_timezone(&chrono::Local);
        assert_eq!(
            (
                timestamp.year,
                timestamp.month,
                timestamp.day,
                timestamp.hour,
                timestamp.minute
            ),
            (
                expected.year() as u16,
                expected.month() as u8,
                expected.day() as u8,
                expected.hour() as u8,
                expected.minute() as u8
            )
        );
        let embed = channel.messages[1].embed.as_ref().unwrap();
        assert_eq!(embed.title, "Status");
        assert_eq!(embed.lines, ["one", "two", "Count: 3"]);
        assert_eq!(embed.footer, "footer");
        assert_eq!(embed.color, 0x202225);
        assert_eq!(channel.messages[1].content, "<@1> **raw**");
        assert!(store.user(3).unwrap().bot);
    }

    #[test]
    fn apply_messages_loaded_unloaded_update_delete_and_ack() {
        let mut store = Store::from_ready(ready());
        let mut message = messages().remove(0);
        message.channel_id = 2004;
        message.id = 100;
        assert_eq!(
            store.apply(Event::MessageCreate(Box::new(message.clone()))),
            Change::Structure
        );
        assert!(store.guilds[0].channel(2004).unwrap().messages.is_empty());
        assert!(store.guilds[0].channel(2004).unwrap().unread);
        store.set_messages(2004, vec![]);
        message.id = 101;
        assert_eq!(
            store.apply(Event::MessageCreate(Box::new(message.clone()))),
            Change::MessageInserted {
                channel_id: 2004,
                index: 0
            }
        );
        assert_eq!(
            store.apply(Event::MessageCreate(Box::new(message))),
            Change::None
        );
        let update = serde_json::from_value(
            json!({"id":"101", "channel_id":"2004", "content":"", "embeds":[]}),
        )
        .unwrap();
        assert_eq!(
            store.apply(Event::MessageUpdate(Box::new(update))),
            Change::MessageChanged {
                channel_id: 2004,
                index: 0
            }
        );
        let updated = &store.guilds[0].channel(2004).unwrap().messages[0];
        assert!(updated.content.is_empty());
        assert!(updated.embed.is_none());
        assert_eq!(updated.author_id, 3);
        let update = serde_json::from_value(json!({"id":"101", "channel_id":"2004"})).unwrap();
        store.apply(Event::MessageUpdate(Box::new(update)));
        assert!(
            store.guilds[0].channel(2004).unwrap().messages[0]
                .content
                .is_empty()
        );
        store.apply(Event::MessageAck {
            channel_id: 2004,
            message_id: 100,
        });
        assert!(store.guilds[0].channel(2004).unwrap().unread);
        store.apply(Event::MessageAck {
            channel_id: 2004,
            message_id: 101,
        });
        assert!(!store.guilds[0].channel(2004).unwrap().unread);
        assert_eq!(
            store.apply(Event::MessageDelete {
                channel_id: 2004,
                id: 101
            }),
            Change::MessageRemoved {
                channel_id: 2004,
                index: 0
            }
        );
        assert_eq!(
            store.apply(Event::MessageDelete {
                channel_id: 2004,
                id: 101
            }),
            Change::None
        );
    }

    #[test]
    fn partial_structure_updates_preserve_history_and_permissions() {
        let ready = ready();
        let source_users = ready.users.as_ptr();
        let mut store = Store::from_ready(ready);
        assert_eq!(store.source.as_ref().unwrap().users.as_ptr(), source_users);
        let other_guild_name = store.guilds[1].name.as_ptr();
        store.set_messages(2002, messages());
        let update =
            serde_json::from_value(json!({"id":"2002", "guild_id":"200", "name":"renamed"}))
                .unwrap();
        store.apply(Event::ChannelUpsert(Box::new(update)));
        assert_eq!(store.source.as_ref().unwrap().users.as_ptr(), source_users);
        assert_eq!(store.guilds[1].name.as_ptr(), other_guild_name);
        assert_eq!(store.guilds[0].channel(2002).unwrap().name, "renamed");
        assert_eq!(store.guilds[0].channel(2002).unwrap().messages.len(), 2);
        let guild =
            serde_json::from_value(json!({"id":"200", "name":"Changed", "icon":null})).unwrap();
        store.apply(Event::GuildUpsert(Box::new(guild)));
        assert_eq!(store.guilds[0].name, "Changed");
        assert!(store.guilds[0].icon.is_none());
        assert!(store.guilds[0].channel(2001).is_none());
        let hide = serde_json::from_value(json!({"id":"2002", "permission_overwrites":[{"id":"1", "type":1, "allow":"0", "deny":"1024"}]})).unwrap();
        store.apply(Event::ChannelUpsert(Box::new(hide)));
        assert!(store.guilds[0].channel(2002).is_none());
    }

    #[test]
    fn badge_deltas_and_indexes_survive_structure_changes() {
        let mut store = Store::from_ready(ready());
        store.apply(Event::MessageAck {
            channel_id: 2002,
            message_id: 60,
        });
        assert_eq!(store.guilds[0].mentions, 1);
        assert!(!store.guilds[0].unread);
        store.apply(Event::MessageAck {
            channel_id: 900,
            message_id: 1,
        });
        assert_eq!(store.dm_mentions, 0);
        store.apply(Event::ChannelDelete { id: 2003 });
        assert_eq!(store.guilds[0].mentions, 0);
        let mut message = messages().remove(0);
        message.id = 100;
        message.channel_id = 2004;
        store.apply(Event::MessageCreate(Box::new(message.clone())));
        assert!(store.guilds[0].unread);
        store.apply(Event::MessageAck {
            channel_id: 2004,
            message_id: 100,
        });
        assert!(!store.guilds[0].unread);
        store.apply(Event::GuildDelete { id: 200 });
        assert_eq!(store.guilds[0].id, 100);
        message.id = 101;
        message.channel_id = 1001;
        store.apply(Event::MessageCreate(Box::new(message)));
        assert!(store.guilds[0].channel(1001).unwrap().unread);
        assert!(store.guilds[0].unread);
    }

    #[test]
    fn thousand_message_creates_with_large_ready_are_fast() {
        let mut ready = ready();
        ready.guilds = (0..100u64)
            .map(|index| {
                let id = 10_000 + index;
                let channels: Vec<_> = (0..20u64)
                    .map(|channel| {
                        json!({
                            "id": (100_000 + index * 20 + channel).to_string(), "type": 0,
                            "name": "busy", "last_message_id": "5000"
                        })
                    })
                    .collect();
                serde_json::from_value(json!({
                    "id": id.to_string(), "properties": {"name": "Busy Guild", "owner_id": "99"},
                    "roles": [{"id": id.to_string(), "permissions": "1024"}],
                    "members": [{"user_id": "1", "roles": []}], "channels": channels
                }))
                .unwrap()
            })
            .collect();
        ready.read_state.entries = (0..2000)
            .map(|index| model::ReadState {
                id: 100_000 + index,
                last_message_id: Some(5000),
                mention_count: 0,
                kind: 0,
            })
            .collect();
        ready.users = (0..20_000)
            .map(|index| model::User {
                id: 10 + index,
                username: format!("user-{index}"),
                global_name: None,
                avatar: None,
                bot: false,
            })
            .collect();
        ready.private_channels.clear();
        let mut store = Store::from_ready(ready);
        let template = messages().remove(0);
        let events: Vec<_> = (0..1000)
            .map(|index| {
                let mut message = template.clone();
                message.id = 6000 + index;
                message.channel_id = 100_000 + index * 2;
                message.author.id = 20_009 - index;
                store.set_messages(message.channel_id, Vec::new());
                Event::MessageCreate(Box::new(message))
            })
            .collect();
        let start = std::time::Instant::now();
        for event in events {
            assert!(matches!(
                store.apply(event),
                Change::MessageInserted { index: 0, .. }
            ));
        }
        let elapsed = start.elapsed();
        println!(
            "1000 MESSAGE_CREATE events / 2000 channels / 2000 read states / 20000 users: {elapsed:?}"
        );
        assert!(
            elapsed < std::time::Duration::from_millis(250),
            "message apply took {elapsed:?}"
        );
        assert_eq!(
            store
                .guilds
                .iter()
                .flat_map(|g| g.channels())
                .filter(|c| c.unread)
                .count(),
            1000
        );
        assert!(store.guilds.iter().all(|g| g.unread));
    }

    #[test]
    fn permission_admin_member_and_category_rules() {
        let mut ready = ready();
        let guild = &mut ready.guilds[0];
        let member_deny: model::Channel =
            serde_json::from_value(json!({"id":"2010", "type":0, "permission_overwrites":[
            {"id":"201", "type":0, "allow":"1024", "deny":"0"},
            {"id":"1", "type":1, "allow":"0", "deny":"1024"}]}))
            .unwrap();
        assert!(!discord::can_view(guild, &member_deny, 1));
        guild.roles.as_mut().unwrap()[1].permissions = 8;
        assert!(discord::can_view(guild, &member_deny, 1));
        guild.roles.as_mut().unwrap()[1].permissions = 0;
        let parent: model::Channel = serde_json::from_value(json!({"id":"230", "type":4, "permission_overwrites":[{"id":"200", "type":0, "deny":"1024", "allow":"0"}]})).unwrap();
        guild.channels.as_mut().unwrap().push(parent);
        let mut child: model::Channel =
            serde_json::from_value(json!({"id":"2011", "type":0, "parent_id":"230"})).unwrap();
        assert!(!discord::can_view(guild, &child, 1));
        child.permission_overwrites = Some(Vec::new());
        assert!(discord::can_view(guild, &child, 1));
    }
}
