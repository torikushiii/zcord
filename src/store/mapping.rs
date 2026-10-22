use super::view::*;
use crate::discord::{self, model};
use chrono::{Datelike, Timelike};
use std::collections::HashMap;

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(3)
        .collect()
}

pub(super) fn local_timestamp(value: &str) -> Option<Timestamp> {
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

pub(super) fn map_embed(embed: &model::Embed) -> Embed {
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

pub(super) fn map_message(message: &model::Message) -> Option<Message> {
    Some(Message {
        id: message.id,
        author_id: message.author.id,
        timestamp: local_timestamp(&message.timestamp)?,
        content: message.content.clone(),
        embed: message.embeds.first().map(map_embed),
        attachments: message.attachments.clone(),
        kind: message.kind,
        edited: message.edited_timestamp.is_some(),
        reply: None,
        mentions_me: false,
    })
}

fn guild_tag(primary: &Option<model::PrimaryGuild>) -> Option<GuildTag> {
    let primary = primary.as_ref()?;
    if primary.identity_enabled == Some(false) {
        return None;
    }
    Some(GuildTag {
        text: primary.tag.clone().filter(|tag| !tag.is_empty())?,
        guild_id: primary.identity_guild_id?,
        badge: primary.badge.clone(),
    })
}

pub(super) fn map_user(user: &model::User) -> User {
    let name = user.global_name.as_ref().unwrap_or(&user.username).clone();
    User {
        id: user.id,
        initials: initials(&name),
        name,
        status: Status::Offline,
        bot: user.bot,
        role_color: None,
        tag: user.primary_guild.as_ref().and_then(guild_tag),
        playing: false,
        activity: None,
        avatar: user.avatar.clone(),
        discriminator: user.discriminator.clone(),
    }
}

pub(super) fn read_state<'a>(
    source: &'a model::Ready,
    index: &HashMap<u64, usize>,
    channel_id: u64,
) -> Option<&'a model::ReadState> {
    source.read_state.entries.get(*index.get(&channel_id)?)
}

pub(super) fn channel_unread(channel: &model::Channel, state: Option<&model::ReadState>) -> bool {
    state.is_some_and(|state| {
        channel.last_message_id.flatten().unwrap_or(0) > state.last_message_id.unwrap_or(0)
    })
}

pub(super) fn map_guild(
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
        member_ids: members.iter().filter_map(|m| m.id()).collect(),
        icon: guild.icon(),
    }
}
