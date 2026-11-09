use super::messages::{Delivery, Media, Message, Timestamp};
use super::users::{GuildTag, StatusEmoji};
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
    pub id: u64,
    pub name: String,
    pub color: Option<u32>,
    pub position: i64,
    pub hoist: bool,
    pub icon: Option<String>,
    pub unicode_emoji: Option<String>,
}

impl Role {
    pub fn icon_spec(&self) -> Option<ImageSpec> {
        self.icon.as_ref().map(|hash| ImageSpec::RoleIcon {
            role_id: self.id,
            hash: hash.clone(),
        })
    }
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
    pub notify: u32,
    pub archived: bool,
    pub locked: bool,
    pub private: bool,
    pub member_count: u32,
    pub member_ids: Vec<u64>,
    pub recent: Option<(u64, String, Timestamp)>,
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
    pub messages: Vec<Message>,
    pub messages_loaded: bool,
}

impl Channel {
    pub fn has_outbox(&self) -> bool {
        self.messages
            .iter()
            .any(|message| message.send_order.is_some())
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoostGoal {
    pub boosts: u32,
    pub goal: u32,
}

pub struct Guild {
    pub id: u64,
    pub name: String,
    pub initials: String,
    pub icon_color: u32,
    pub unread: bool,
    pub mentions: u32,
    pub premium_tier: u8,
    pub boost_goal: Option<BoostGoal>,
    pub server_guide: bool,
    pub events: Option<u32>,
    pub channels_and_roles: bool,
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

    pub(crate) fn channel_mut(&mut self, id: u64) -> Option<&mut Channel> {
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
