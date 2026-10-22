pub use crate::discord::model::Attachment;
use crate::images::ImageSpec;

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

#[derive(Clone, PartialEq, Eq)]
pub struct GuildTag {
    pub text: String,
    pub guild_id: u64,
    pub badge: Option<String>,
}

impl GuildTag {
    pub fn badge_spec(&self) -> Option<ImageSpec> {
        self.badge.as_ref().map(|hash| ImageSpec::GuildTagBadge {
            guild_id: self.guild_id,
            hash: hash.clone(),
        })
    }
}

pub struct User {
    pub id: u64,
    pub name: String,
    pub initials: String,
    pub status: Status,
    pub bot: bool,
    pub role_color: Option<u32>,
    pub tag: Option<GuildTag>,
    pub playing: bool,
    pub activity: Option<String>,
    pub avatar: Option<String>,
    pub discriminator: Option<String>,
}

impl User {
    pub fn avatar_spec(&self) -> ImageSpec {
        if let Some(hash) = &self.avatar {
            return ImageSpec::Avatar {
                user_id: self.id,
                hash: hash.clone(),
            };
        }
        let index = self
            .discriminator
            .as_deref()
            .filter(|value| *value != "0")
            .and_then(|value| value.parse::<u64>().ok())
            .map(|value| value % 5)
            .unwrap_or((self.id >> 22) % 6) as u8;
        ImageSpec::DefaultAvatar { index }
    }
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

    pub(super) fn plus_minutes(self, minutes: u32) -> Self {
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
    pub attachments: Vec<Attachment>,
    pub kind: u8,
    pub edited: bool,
    pub reply: Option<ReplyState>,
    pub mentions_me: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
    pub id: u64,
    pub name: String,
    pub color: Option<u32>,
    pub position: i64,
    pub hoist: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub message_id: u64,
    pub author_id: u64,
    pub content: String,
    pub attachments: bool,
    pub embeds: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyState {
    Loaded(Reply),
    Deleted,
    Unavailable,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct MemberList {
    pub id: String,
    pub member_count: u32,
    pub online_count: u32,
    pub rows: Vec<Option<MemberRow>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemberRow {
    Group { name: String, count: u32 },
    Member { user_id: u64 },
}

impl Attachment {
    pub fn image_spec(&self) -> Option<ImageSpec> {
        let image_type = self
            .content_type
            .as_deref()
            .and_then(|mime| mime.split(';').next())
            .is_some_and(|mime| {
                matches!(
                    mime.trim().to_ascii_lowercase().as_str(),
                    "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp"
                )
            });
        let extension = self
            .filename
            .rsplit_once('.')
            .is_some_and(|(_, extension)| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "png" | "jpg" | "jpeg" | "gif" | "webp"
                )
            });
        let (width, height) = (self.width?, self.height?);
        if !(image_type || extension) || width == 0 || height == 0 || self.proxy_url.is_empty() {
            return None;
        }
        Some(ImageSpec::Attachment {
            id: self.id,
            url: self.proxy_url.clone(),
            width,
            height,
        })
    }
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

pub struct Guild {
    pub id: u64,
    pub name: String,
    pub initials: String,
    pub icon_color: u32,
    pub unread: bool,
    pub mentions: u32,
    pub categories: Vec<Category>,
    pub member_ids: Vec<u64>,
    pub icon: Option<String>,
}

impl Guild {
    pub fn icon_spec(&self) -> Option<ImageSpec> {
        self.icon.as_ref().map(|hash| ImageSpec::GuildIcon {
            guild_id: self.id,
            hash: hash.clone(),
        })
    }
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

    pub(super) fn channel_mut(&mut self, id: u64) -> Option<&mut Channel> {
        self.categories
            .iter_mut()
            .flat_map(|category| &mut category.channels)
            .find(|channel| channel.id == id)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RailEntry {
    Guild(u64),
    Folder(Folder),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Folder {
    pub id: u64,
    pub name: Option<String>,
    pub color: Option<u32>,
    pub guild_ids: Vec<u64>,
}
