use super::*;

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
    pub real_name: String,
    pub initials: String,
    pub status: Status,
    pub on_mobile: bool,
    pub bot: bool,
    pub verified_bot: bool,
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
    pub kind: u8,
    pub platform: Option<String>,
    pub embedded: bool,
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
