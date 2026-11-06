use crate::discord::model;
pub use crate::discord::model::Attachment;
use crate::media::images::ImageSpec;
use std::sync::Arc;
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
    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "Online",
            Self::Idle => "Idle",
            Self::DoNotDisturb => "Do Not Disturb",
            Self::Offline => "Offline",
        }
    }

    pub fn is_online(self) -> bool {
        self != Status::Offline
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub on_mobile: bool,
    pub bot: bool,
    pub role_color: Option<u32>,
    pub tag: Option<GuildTag>,
    pub playing: bool,
    pub activity: Option<String>,
    pub activity_kind: Option<u8>,
    pub game: Option<GameActivity>,
    pub listening: Option<GameActivity>,
    pub live_stream: Option<LiveStream>,
    pub custom_status: Option<CustomStatus>,
    pub avatar: Option<String>,
    pub discriminator: Option<String>,
    pub decoration: Option<Decoration>,
    pub nameplate: Option<Nameplate>,
    pub name_style: Option<NameStyle>,
}

/// A stream on an outside service such as Twitch, which is presence activity type 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveStream {
    pub service: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub twitch_login: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameActivity {
    pub name: String,
    pub application_id: Option<u64>,
    /// Raw activity type (0 playing, 2 listening, ...).
    pub kind: u8,
    pub platform: Option<String>,
    /// An embedded Discord Activity (`flags & (1 << 8)` or platform "embedded").
    pub embedded: bool,
    /// The activity advertises a Play action (`flags & (1 << 5)`).
    pub playable: bool,
    pub started_at: Option<SystemTime>,
    pub ends_at: Option<SystemTime>,
    pub details: Option<String>,
    pub state: Option<String>,
    pub large_image: Option<ActivityImage>,
    pub small_image: Option<ActivityImage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityImage {
    pub url: String,
    pub text: Option<String>,
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
        avatar_spec(
            self.id,
            self.avatar.as_deref(),
            self.discriminator.as_deref(),
        )
    }
}

pub fn avatar_spec(user_id: u64, avatar: Option<&str>, discriminator: Option<&str>) -> ImageSpec {
    if let Some(hash) = avatar {
        return ImageSpec::Avatar {
            user_id,
            hash: hash.to_owned(),
        };
    }
    let index = discriminator
        .filter(|value| *value != "0")
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value % 5)
        .unwrap_or((user_id >> 22) % 6) as u8;
    ImageSpec::DefaultAvatar { index }
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
        if crate::settings::clock_24h() {
            return format!("{:02}:{:02}", self.hour, self.minute);
        }
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

    pub fn calendar(self) -> String {
        let Some(date) = chrono::NaiveDate::from_ymd_opt(
            i32::from(self.year),
            u32::from(self.month),
            u32::from(self.day),
        ) else {
            return self.full();
        };
        let days = (date - chrono::Local::now().date_naive()).num_days();
        let day = format!("{:02}/{:02}/{}", self.month, self.day, self.year);
        match days {
            0 => format!("Today at {}", self.time()),
            -1 => format!("Yesterday at {}", self.time()),
            1 => format!("Tomorrow at {}", self.time()),
            2.. => day,
            _ => format!("{day} {}", self.time()),
        }
    }

    pub fn short(self) -> String {
        use chrono::Datelike;
        let now = chrono::Local::now();
        let today = (
            i32::from(self.year),
            u32::from(self.month),
            u32::from(self.day),
        ) == (now.year(), now.month(), now.day());
        if today { self.time() } else { self.full() }
    }

    /// "Jan 2017"
    pub fn month_year(self) -> String {
        let month = MONTHS[(self.month as usize).saturating_sub(1) % 12];
        format!("{} {}", &month[..3], self.year)
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
    /// OpenGraph pages; Discord shows their thumbnail as the large image under the text.
    Article,
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
    ActivityEntry(ActivityEntryView),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityEntryView {
    pub entry_id: u64,
    pub user_id: u64,
    pub game_name: String,
    pub application_id: Option<u64>,
    pub created_ms: i64,
    pub live_since_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    pub streak_days: Option<u32>,
    pub most_played_hours: Option<u32>,
    pub first_time: bool,
    pub trending: bool,
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

pub(super) struct SendOrder {
    pub nonce: u64,
    /// Part of the current loaded history span, rather than a retained outbox-only row.
    pub history_seen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookAuthor {
    pub id: u64,
    pub name: String,
    pub avatar: Option<String>,
}

const FORWARDABLE_FLAGS: u64 = (1 << 0)
    | (1 << 1)
    | (1 << 2)
    | (1 << 4)
    | (1 << 5)
    | (1 << 8)
    | (1 << 9)
    | (1 << 10)
    | (1 << 12)
    | (1 << 13)
    | (1 << 14)
    | (1 << 15)
    | (1 << 19)
    | (1 << 20);

pub(super) fn is_forwardable(message: &model::Message) -> bool {
    matches!(message.kind, 0 | 19 | 20 | 23)
        && (message.flags & !FORWARDABLE_FLAGS) == 0
        && message.poll.is_none()
        && message.call.is_none()
        && message.activity.is_none()
        && message.activity_instance.is_none()
        && message.shared_client_theme.is_none()
}

pub struct Message {
    pub id: u64,
    /// Composer order, retained through confirmation until replacing history observes it.
    pub(super) send_order: Option<SendOrder>,
    pub author_id: u64,
    /// Webhook names and avatars belong to the message, not the shared user cache.
    pub webhook: Option<WebhookAuthor>,
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
    pub forwardable: bool,
    pub mentions_me: bool,
    pub uploads: Vec<PendingUpload>,
    pub thread: Option<MessageThread>,
    /// The message a pin notice points at.
    pub reference_id: Option<u64>,
    pub call: Option<CallRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRecord {
    pub channel_id: u64,
    pub participants: Vec<u64>,
    pub duration_secs: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageThread {
    pub id: u64,
    pub name: String,
    pub message_count: u32,
    pub archived: bool,
    pub recent: Option<(u64, String, Timestamp)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingUpload {
    pub name: String,
    pub size: u64,
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
    pub webhook: Option<WebhookAuthor>,
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

/// What the app reads off an attachment: spoilers, and whether it shows as a video or image.
pub trait AttachmentExt {
    fn spoiler(&self) -> bool;
    fn is_video(&self) -> bool;
    fn video_url(&self) -> Option<String>;
    fn video_poster_spec(&self) -> Option<ImageSpec>;
    fn image_spec(&self) -> Option<ImageSpec>;
}

impl AttachmentExt for Attachment {
    fn spoiler(&self) -> bool {
        self.flags & ATTACHMENT_IS_SPOILER != 0 || self.filename.starts_with("SPOILER_")
    }

    fn is_video(&self) -> bool {
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

    fn video_url(&self) -> Option<String> {
        if !self.is_video() {
            return None;
        }
        crate::media::images::trusted_video_url(&self.proxy_url)
            .or_else(|| crate::media::images::trusted_video_url(&self.url))
    }

    fn video_poster_spec(&self) -> Option<ImageSpec> {
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

    fn image_spec(&self) -> Option<ImageSpec> {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadRow {
    pub id: u64,
    pub name: String,
    pub parent_id: u64,
    pub owner_id: Option<u64>,
    pub joined: bool,
    pub muted: bool,
    /// Thread notification level: 2 all messages, 4 only @mentions, 8 nothing; 0 when unset.
    pub notify: u32,
    pub archived: bool,
    pub locked: bool,
    pub private: bool,
    pub member_count: u32,
    pub member_ids: Vec<u64>,
    /// Newest message: author and flattened text, cut to 200 bytes.
    pub recent: Option<(u64, String, Timestamp)>,
    /// `archive_timestamp`, or the newest message's time when there is one.
    pub last_active: Option<Timestamp>,
}

pub struct Channel {
    pub id: u64,
    pub name: String,
    pub kind: ChannelKind,
    pub parent_id: Option<u64>,
    pub private: bool,
    pub unread: bool,
    pub mentions: u32,
    pub new_posts: u32,
    /// Display order; see `order_messages`.
    pub messages: Vec<Message>,
    pub messages_loaded: bool,
}

impl Channel {
    pub fn has_outbox(&self) -> bool {
        self.messages
            .iter()
            .any(|message| message.send_order.is_some())
    }

    /// Try ID-sorted lookup, falling back to a scan for out-of-order placeholders.
    pub fn message_index(&self, id: u64) -> Option<usize> {
        self.messages
            .binary_search_by_key(&id, |message| message.id)
            .ok()
            .or_else(|| self.messages.iter().position(|message| message.id == id))
    }

    #[cfg(test)]
    pub fn oldest_message(&self) -> Option<&Message> {
        self.messages
            .iter()
            .find(|message| message.delivery == Delivery::Sent)
    }

    #[cfg(test)]
    pub fn newest_message(&self) -> Option<&Message> {
        self.messages
            .iter()
            .rev()
            .find(|message| message.delivery == Delivery::Sent)
    }

    /// Retained outbox rows outside the loaded history page must not bridge pagination gaps.
    pub fn oldest_history_message(&self) -> Option<&Message> {
        self.messages.iter().find(|message| message.in_history())
    }

    pub fn newest_history_message(&self) -> Option<&Message> {
        self.messages
            .iter()
            .rev()
            .find(|message| message.in_history())
    }

    pub fn first_unread_index(&self, since: u64, me: u64) -> Option<usize> {
        self.messages.iter().position(|message| {
            message.delivery == Delivery::Sent && message.author_id != me && message.id > since
        })
    }

    pub fn unread_message_count(&self, since: u64, me: u64) -> usize {
        self.messages
            .iter()
            .filter(|message| {
                message.delivery == Delivery::Sent && message.author_id != me && message.id > since
            })
            .count()
    }
}

impl Message {
    pub fn same_author(&self, other: &Self) -> bool {
        self.author_id == other.author_id && self.webhook == other.webhook
    }

    fn in_history(&self) -> bool {
        self.delivery == Delivery::Sent
            && self
                .send_order
                .as_ref()
                .is_none_or(|order| order.history_seen)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationshipKind {
    Friend,
    Blocked,
    Incoming,
    Outgoing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub user_id: u64,
    pub kind: RelationshipKind,
    pub nickname: Option<String>,
    pub since: Option<Timestamp>,
    pub spam: bool,
    pub ignored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FriendSuggestion {
    pub user_id: u64,
    /// The name Discord matched them by, such as their Steam name.
    pub reason: Option<String>,
}

pub struct PrivateChannel {
    pub channel: Channel,
    pub group: bool,
    pub recipient_ids: Vec<u64>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub owner_id: Option<u64>,
    pub message_request: bool,
    pub spam: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestPreview {
    pub author_id: u64,
    pub content: String,
    pub timestamp: Timestamp,
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
    /// The starter message, which carries the post's body, media and reactions.
    pub first_message: Option<Message>,
    pub unread: bool,
}

impl ForumPost {
    pub fn media(&self) -> Vec<ImageSpec> {
        let Some(message) = &self.first_message else {
            return Vec::new();
        };
        let attachments = message.attachments.iter().filter_map(|attachment| {
            attachment
                .image_spec()
                .or_else(|| attachment.video_poster_spec())
        });
        let embeds = message.embeds.iter().filter_map(|embed| {
            embed
                .images
                .first()
                .or(embed.thumbnail.as_ref())
                .map(Media::spec)
        });
        attachments.chain(embeds).collect()
    }
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
    pub premium_tier: u8,
    pub categories: Vec<Category>,
    pub threads: Vec<Channel>,
    pub member_ids: Vec<u64>,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub emojis: Vec<CustomEmoji>,
    pub tag: Option<GuildTag>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InviteState {
    Loading,
    Invalid,
    Ready(Arc<Invite>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub code: String,
    pub guild_id: u64,
    pub name: String,
    /// Guild icon hash (`icons/{guild_id}/{hash}`).
    pub icon: Option<String>,
    pub banner: Option<InviteBanner>,
    pub description: Option<String>,
    pub tag: Option<GuildTag>,
    pub online: u32,
    pub members: u32,
    /// Derived from the guild id snowflake.
    pub created_at: Timestamp,
    pub games: Vec<u64>,
    /// `games` resolved through the public-application lookup, for icons.
    pub game_apps: Vec<InviteApp>,
    pub expires_at: Option<Timestamp>,
}

impl Invite {
    pub fn from_model(
        invite: &model::Invite,
        applications: &[Arc<model::PublicApplication>],
    ) -> Option<Self> {
        let profile = invite.profile.as_ref();
        let guild = invite.guild.as_ref();
        let guild_id = invite
            .guild_id
            .or_else(|| guild.and_then(|guild| guild.id))
            .or_else(|| profile.and_then(|profile| profile.id))?;
        let hash = |value: Option<&String>| value.filter(|value| !value.is_empty()).cloned();
        let name = profile
            .and_then(|profile| profile.name.clone())
            .or_else(|| guild.map(|guild| guild.name.clone()))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| invite.code.clone());
        let icon = hash(profile.and_then(|profile| profile.icon_hash.as_ref()))
            .or_else(|| hash(guild.and_then(|guild| guild.icon.as_ref())));
        let banner = hash(profile.and_then(|profile| profile.custom_banner_hash.as_ref()))
            .map(InviteBanner::Custom)
            .or_else(|| {
                hash(profile.and_then(|profile| profile.banner_hash.as_ref()))
                    .map(InviteBanner::Banner)
            })
            .or_else(|| {
                hash(guild.and_then(|guild| guild.banner.as_ref())).map(InviteBanner::GuildBanner)
            })
            .or_else(|| {
                hash(guild.and_then(|guild| guild.splash.as_ref())).map(InviteBanner::Splash)
            });
        let description = profile
            .and_then(|profile| profile.description.clone())
            .filter(|description| !description.is_empty())
            .or_else(|| {
                guild
                    .and_then(|guild| guild.description.clone())
                    .filter(|description| !description.is_empty())
            });
        let tag = profile
            .and_then(|profile| hash(profile.tag.as_ref()))
            .map(|text| GuildTag {
                text,
                guild_id,
                badge: profile.and_then(|profile| hash(profile.badge_hash.as_ref())),
            });
        Some(Self {
            code: invite.code.clone(),
            guild_id,
            name,
            icon,
            banner,
            description,
            tag,
            online: profile
                .and_then(|profile| profile.online_count)
                .or(invite.approximate_presence_count)
                .unwrap_or_default(),
            members: profile
                .and_then(|profile| profile.member_count)
                .or(invite.approximate_member_count)
                .unwrap_or_default(),
            created_at: super::mapping::timestamp_from_nonce(guild_id),
            games: profile
                .map(|profile| profile.game_application_ids.clone())
                .unwrap_or_default(),
            game_apps: applications
                .iter()
                .map(|application| InviteApp {
                    id: application.id,
                    name: application.name.clone(),
                    icon: application.icon.clone(),
                })
                .collect(),
            expires_at: invite
                .expires_at
                .as_deref()
                .and_then(super::mapping::local_timestamp),
        })
    }

    pub fn icon_spec(&self) -> Option<ImageSpec> {
        self.icon.as_ref().map(|hash| ImageSpec::GuildIcon {
            guild_id: self.guild_id,
            hash: hash.clone(),
        })
    }

    pub fn banner_spec(&self) -> Option<ImageSpec> {
        self.banner.as_ref()?.spec(self.guild_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InviteBanner {
    /// `profile.custom_banner_hash`
    Custom(String),
    /// `profile.banner_hash`
    Banner(String),
    /// `guild.banner`
    GuildBanner(String),
    /// `guild.splash`
    Splash(String),
}

impl InviteBanner {
    pub fn hash(&self) -> &str {
        match self {
            Self::Custom(hash)
            | Self::Banner(hash)
            | Self::GuildBanner(hash)
            | Self::Splash(hash) => hash,
        }
    }

    pub fn spec(&self, guild_id: u64) -> Option<ImageSpec> {
        let hash = self.hash().to_owned();
        Some(match self {
            Self::Custom(_) => ImageSpec::DiscoverySplash { guild_id, hash },
            Self::Banner(_) | Self::GuildBanner(_) => ImageSpec::GuildBanner { guild_id, hash },
            Self::Splash(_) => ImageSpec::GuildSplash { guild_id, hash },
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteApp {
    pub id: u64,
    pub name: String,
    /// App icon hash (`app-icons/{id}/{hash}`).
    pub icon: Option<String>,
}

impl InviteApp {
    pub fn icon_spec(&self) -> Option<ImageSpec> {
        let hash = self.icon.clone()?;
        let animated = hash.starts_with("a_");
        Some(ImageSpec::AppImage {
            application_id: self.id,
            hash,
            uploaded: false,
            animated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(kind: u8, flags: u64) -> model::Message {
        serde_json::from_value(json!({
            "id": "1",
            "channel_id": "2",
            "author": {"id": "3", "username": "tester", "discriminator": "0"},
            "timestamp": "2024-01-01T00:00:00+00:00",
            "type": kind,
            "flags": flags,
        }))
        .unwrap()
    }

    #[test]
    fn forwardable_plain_message() {
        assert!(is_forwardable(&message(0, 0)));
    }

    #[test]
    fn forwardable_rejects_kind_7() {
        assert!(!is_forwardable(&message(7, 0)));
    }

    #[test]
    fn forwardable_rejects_ephemeral_flag() {
        assert!(!is_forwardable(&message(0, 1 << 6)));
    }

    #[test]
    fn forwardable_rejects_poll() {
        let mut message = message(0, 0);
        message.poll = Some(json!({}));
        assert!(!is_forwardable(&message));
    }
}
