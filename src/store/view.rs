pub use crate::discord::model::Attachment;
use crate::images::ImageSpec;
use std::time::SystemTime;

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
    pub username: String,
    pub name: String,
    pub initials: String,
    pub status: Status,
    pub bot: bool,
    pub role_color: Option<u32>,
    pub tag: Option<GuildTag>,
    pub playing: bool,
    pub activity: Option<String>,
    pub custom_status: Option<CustomStatus>,
    pub avatar: Option<String>,
    pub discriminator: Option<String>,
    pub decoration: Option<Decoration>,
    pub nameplate: Option<Nameplate>,
    pub name_style: Option<NameStyle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomStatus {
    pub text: Option<String>,
    pub emoji: Option<StatusEmoji>,
    pub expires_at: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusEmoji {
    Unicode(String),
    Custom {
        id: u64,
        name: String,
        animated: bool,
    },
}

impl CustomStatus {
    pub fn is_expired(&self, now: SystemTime) -> bool {
        self.expires_at.is_some_and(|expires_at| expires_at <= now)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Decoration {
    pub asset: String,
    pub sku_id: Option<u64>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Nameplate {
    pub asset: String,
    pub palette: Option<String>,
    pub label: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NameStyle {
    pub font_id: Option<u32>,
    pub effect_id: Option<u32>,
    pub colors: Vec<u32>,
}

impl Decoration {
    pub fn image_spec(&self, animated: bool) -> ImageSpec {
        ImageSpec::AvatarDecoration {
            asset: self.asset.clone(),
            animated,
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
        format!(
            "{}/{}/{:02}, {}",
            self.month,
            self.day,
            self.year % 100,
            self.time()
        )
    }

    pub fn date(self) -> String {
        let month = MONTHS[(self.month as usize).saturating_sub(1) % 12];
        format!("{month} {}, {}", self.day, self.year)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Media {
    pub url: String,
    /// Page-facing original URL (pre-proxy) for opening in a browser.
    pub source: String,
    pub width: u32,
    pub height: u32,
    pub animated: bool,
}

impl Media {
    pub fn spec(&self) -> ImageSpec {
        ImageSpec::Media {
            url: self.url.clone(),
            width: self.width,
            height: self.height,
            animated: self.animated,
        }
    }

    pub fn video_spec(&self) -> ImageSpec {
        ImageSpec::Video {
            url: self.url.clone(),
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EmbedKind {
    Rich,
    Image,
    Gifv,
    Video,
    Link,
    Components,
}

#[derive(Clone)]
pub struct EmbedText {
    pub text: String,
    pub url: Option<String>,
    pub icon: Option<Media>,
}

#[derive(Clone)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

#[derive(Clone)]
pub struct Embed {
    pub kind: EmbedKind,
    pub url: Option<String>,
    pub color: Option<u32>,
    pub provider: Option<EmbedText>,
    pub author: Option<EmbedText>,
    pub title: Option<EmbedText>,
    pub description: Option<String>,
    pub fields: Vec<EmbedField>,
    pub components: Vec<Component>,
    pub thumbnail: Option<Media>,
    pub images: Vec<Media>,
    pub video: Option<Media>,
    pub footer: Option<EmbedText>,
    pub timestamp: Option<Timestamp>,
}

pub fn hides_embed_content(content: &str, embeds: &[Embed]) -> bool {
    let [embed] = embeds else {
        return false;
    };
    matches!(embed.kind, EmbedKind::Image | EmbedKind::Gifv)
        && embed.url.as_deref() == Some(content.trim())
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReactionEmoji {
    pub id: Option<u64>,
    pub name: String,
    pub animated: bool,
}

#[derive(Clone)]
pub struct Reaction {
    pub emoji: ReactionEmoji,
    pub count: u32,
    pub me: bool,
}

#[derive(Clone)]
pub struct GalleryItem {
    pub media: Media,
    pub description: Option<String>,
    pub spoiler: bool,
}

#[derive(Clone)]
pub enum Component {
    Row(Vec<Component>),
    Button {
        style: u8,
        label: Option<String>,
        emoji: Option<ReactionEmoji>,
        url: Option<String>,
        disabled: bool,
    },
    Select {
        placeholder: Option<String>,
        disabled: bool,
    },
    Section {
        components: Vec<Component>,
        accessory: Option<Box<Component>>,
    },
    Text(String),
    Thumbnail {
        media: Media,
        description: Option<String>,
        spoiler: bool,
    },
    Gallery(Vec<GalleryItem>),
    File {
        name: String,
        size: u64,
        spoiler: bool,
    },
    Separator {
        divider: bool,
        large: bool,
    },
    Container {
        accent: Option<u32>,
        spoiler: bool,
        components: Vec<Component>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    Pending,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sticker {
    pub id: u64,
    pub name: String,
    /// Discord's `format_type`: 1 = PNG, 2 = APNG, 3 = Lottie, 4 = GIF.
    pub format: u8,
}

impl Sticker {
    pub fn image_spec(&self) -> Option<ImageSpec> {
        if self.id == 0 {
            return None;
        }
        match self.format {
            1 | 2 | 4 => Some(ImageSpec::Sticker {
                id: self.id,
                format: self.format,
            }),
            _ => None,
        }
    }
}

pub struct Message {
    pub id: u64,
    pub author_id: u64,
    pub timestamp: Timestamp,
    pub delivery: Delivery,
    pub content: String,
    pub embeds: Vec<Embed>,
    pub components: Vec<Component>,
    pub reactions: Vec<Reaction>,
    pub attachments: Vec<Attachment>,
    pub stickers: Vec<Sticker>,
    pub kind: u8,
    pub edited: bool,
    pub reply: Option<ReplyState>,
    pub forward: Option<Forward>,
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
    pub timestamp: Timestamp,
    pub content: String,
    pub attachments: bool,
    pub embeds: bool,
    pub sticker: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyState {
    Loaded(Reply),
    Deleted,
    Unavailable,
}

pub struct Forward {
    pub content: String,
    pub embeds: Vec<Embed>,
    pub components: Vec<Component>,
    pub attachments: Vec<Attachment>,
    pub stickers: Vec<Sticker>,
    pub timestamp: Option<Timestamp>,
    pub edited: bool,
    pub channel_id: Option<u64>,
    pub guild_id: Option<u64>,
}

const ATTACHMENT_IS_SPOILER: u64 = 1 << 3;

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
    pub fn spoiler(&self) -> bool {
        self.flags & ATTACHMENT_IS_SPOILER != 0 || self.filename.starts_with("SPOILER_")
    }

    pub fn is_video(&self) -> bool {
        let video_type = self
            .content_type
            .as_deref()
            .and_then(|mime| mime.split(';').next())
            .is_some_and(|mime| mime.trim().to_ascii_lowercase().starts_with("video/"));
        let extension = self
            .filename
            .rsplit_once('.')
            .is_some_and(|(_, extension)| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "mp4" | "mov" | "webm" | "m4v"
                )
            });
        (video_type || extension)
            && self.width.is_some_and(|width| width > 0)
            && self.height.is_some_and(|height| height > 0)
    }

    pub fn video_url(&self) -> Option<String> {
        if !self.is_video() {
            return None;
        }
        crate::images::trusted_video_url(&self.proxy_url)
            .or_else(|| crate::images::trusted_video_url(&self.url))
    }

    pub fn video_poster_spec(&self) -> Option<ImageSpec> {
        if !self.is_video() || self.proxy_url.is_empty() {
            return None;
        }
        Some(ImageSpec::VideoPoster {
            id: self.id,
            url: self.proxy_url.clone(),
            width: self.width?,
            height: self.height?,
        })
    }

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
    Announcement,
    Forum,
    Voice,
}

impl ChannelKind {
    pub fn is_text(self) -> bool {
        matches!(self, Self::Text | Self::Announcement)
    }

    pub fn is_selectable(self) -> bool {
        matches!(self, Self::Text | Self::Announcement | Self::Forum)
    }

    pub fn is_unreadable(self) -> bool {
        matches!(self, Self::Text | Self::Announcement | Self::Forum)
    }

    pub fn is_forum(self) -> bool {
        self == Self::Forum
    }
}

pub struct Channel {
    pub id: u64,
    pub name: String,
    pub kind: ChannelKind,
    pub parent_id: Option<u64>,
    pub unread: bool,
    pub mentions: u32,
    pub new_posts: u32,
    pub messages: Vec<Message>,
    pub messages_loaded: bool,
}

pub struct Category {
    pub id: u64,
    pub name: String,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForumTag {
    pub id: u64,
    pub name: String,
    pub emoji: Option<StatusEmoji>,
    pub moderated: bool,
}

pub struct ForumPost {
    pub id: u64,
    pub forum_id: u64,
    pub name: String,
    pub owner_id: u64,
    pub created_at: Timestamp,
    pub last_activity: Timestamp,
    pub message_count: u32,
    pub tag_ids: Vec<u64>,
    pub pinned: bool,
    pub locked: bool,
    pub archived: bool,
    pub preview: Option<String>,
    pub thumbnail: Option<ImageSpec>,
    pub unread: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomEmoji {
    pub id: u64,
    pub name: String,
    pub animated: bool,
    pub guild_id: u64,
    pub available: bool,
    pub roles: Vec<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmojiAccess {
    Usable,
    NeedsNitro,
    NeedsRole,
    Unavailable,
    NoExternalPermission,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmojiChoice {
    Unicode { emoji: String, shortcode: String },
    Custom(CustomEmoji),
}

pub struct Guild {
    pub id: u64,
    pub name: String,
    pub initials: String,
    pub icon_color: u32,
    pub unread: bool,
    pub mentions: u32,
    pub categories: Vec<Category>,
    pub threads: Vec<Channel>,
    pub member_ids: Vec<u64>,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub emojis: Vec<CustomEmoji>,
}

impl Guild {
    pub fn icon_spec(&self) -> Option<ImageSpec> {
        self.icon.as_ref().map(|hash| ImageSpec::GuildIcon {
            guild_id: self.id,
            hash: hash.clone(),
        })
    }
    pub fn banner_spec(&self) -> Option<ImageSpec> {
        self.banner.as_ref().map(|hash| ImageSpec::GuildBanner {
            guild_id: self.id,
            hash: hash.clone(),
        })
    }
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.categories
            .iter()
            .flat_map(|category| &category.channels)
    }

    pub fn all_channels(&self) -> impl Iterator<Item = &Channel> {
        self.channels().chain(&self.threads)
    }

    pub fn first_text_channel(&self) -> Option<&Channel> {
        self.channels().find(|channel| channel.kind.is_text())
    }

    pub fn first_selectable_channel(&self) -> Option<&Channel> {
        self.channels().find(|channel| channel.kind.is_selectable())
    }

    pub fn channel(&self, id: u64) -> Option<&Channel> {
        self.all_channels().find(|channel| channel.id == id)
    }

    pub(super) fn channel_mut(&mut self, id: u64) -> Option<&mut Channel> {
        self.categories
            .iter_mut()
            .flat_map(|category| &mut category.channels)
            .chain(&mut self.threads)
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
