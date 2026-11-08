use super::messages::Timestamp;
use super::users::GuildTag;
use super::*;

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
    pub icon: Option<String>,
    pub banner: Option<InviteBanner>,
    pub description: Option<String>,
    pub tag: Option<GuildTag>,
    pub online: u32,
    pub members: u32,
    pub created_at: Timestamp,
    pub games: Vec<u64>,
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
            created_at: super::super::mapping::timestamp_from_nonce(guild_id),
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
                .and_then(super::super::mapping::local_timestamp),
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
    Custom(String),
    Banner(String),
    GuildBanner(String),
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
