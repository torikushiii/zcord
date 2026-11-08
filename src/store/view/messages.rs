use super::*;

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

    pub(crate) fn plus_minutes(self, minutes: u32) -> Self {
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

    pub fn header(self) -> String {
        self.header_on(chrono::Local::now().date_naive())
    }

    pub(super) fn header_on(self, today: chrono::NaiveDate) -> String {
        let date = chrono::NaiveDate::from_ymd_opt(
            i32::from(self.year),
            u32::from(self.month),
            u32::from(self.day),
        );
        match date.map(|date| (today - date).num_days()) {
            Some(0) => self.time(),
            Some(1) => format!("Yesterday at {}", self.time()),
            _ => format!("{}/{}/{} {}", self.month, self.day, self.year, self.time()),
        }
    }

    pub fn forwarded(self) -> String {
        self.forwarded_on(chrono::Local::now().date_naive())
    }

    pub(super) fn forwarded_on(self, today: chrono::NaiveDate) -> String {
        let date = chrono::NaiveDate::from_ymd_opt(
            i32::from(self.year),
            u32::from(self.month),
            u32::from(self.day),
        );
        match date.map(|date| ((today - date).num_days(), date)) {
            Some((0, _)) => self.time(),
            Some((1, _)) => "Yesterday".to_owned(),
            Some((2..=6, date)) => date.format("%A").to_string(),
            _ => format!("{:02}/{:02}/{}", self.month, self.day, self.year),
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

pub(crate) struct SendOrder {
    pub nonce: u64,
    pub history_seen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookAuthor {
    pub id: u64,
    pub name: String,
    pub avatar: Option<String>,
    pub verified_bot: bool,
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

pub(crate) fn is_forwardable(message: &model::Message) -> bool {
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
    pub(crate) send_order: Option<SendOrder>,
    pub author_id: u64,
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
    pub message_id: Option<u64>,
}

const ATTACHMENT_IS_SPOILER: u64 = 1 << 3;

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

impl Message {
    pub fn same_author(&self, other: &Self) -> bool {
        self.author_id == other.author_id && self.webhook == other.webhook
    }

    pub(super) fn in_history(&self) -> bool {
        self.delivery == Delivery::Sent
            && self
                .send_order
                .as_ref()
                .is_none_or(|order| order.history_seen)
    }
}
