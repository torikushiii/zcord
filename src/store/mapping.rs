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

pub(super) fn timestamp_from_nonce(nonce: u64) -> Timestamp {
    const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
    let local = (nonce >> 22)
        .checked_add(DISCORD_EPOCH_MS)
        .and_then(|ms| i64::try_from(ms).ok())
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|utc| utc.with_timezone(&chrono::Local))
        .unwrap_or_else(chrono::Local::now);
    Timestamp {
        year: local.year().try_into().unwrap_or(2026),
        month: local.month() as u8,
        day: local.day() as u8,
        hour: local.hour() as u8,
        minute: local.minute() as u8,
    }
}

fn nonempty(value: &Option<String>) -> Option<String> {
    value.clone().filter(|value| !value.is_empty())
}

fn media(
    url: &Option<String>,
    proxy: &Option<String>,
    width: u32,
    height: u32,
    animated: bool,
) -> Option<Media> {
    let proxied = nonempty(proxy);
    let raw = proxied.as_ref().or(url.as_ref())?;
    let parsed = reqwest::Url::parse(raw).ok()?;
    if !crate::images::allowed_url(&parsed)
        || (proxied.is_none()
            && !matches!(
                parsed.host_str(),
                Some("cdn.discordapp.com" | "media.discordapp.net")
            ))
    {
        return None;
    }

    let source = nonempty(url)
        .filter(|url| {
            reqwest::Url::parse(url).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
        })
        .unwrap_or_else(|| raw.clone());
    Some(Media {
        url: raw.clone(),
        source,
        width,
        height,
        animated,
    })
}

const EMBED_MEDIA_IS_ANIMATED: u64 = 1 << 5;

fn map_media(value: &model::EmbedMedia) -> Option<Media> {
    media(
        &value.url,
        &value.proxy_url,
        value.width.unwrap_or(0),
        value.height.unwrap_or(0),
        value.flags & EMBED_MEDIA_IS_ANIMATED != 0,
    )
}

pub(super) fn map_embed(embed: &model::Embed) -> Embed {
    let url = nonempty(&embed.url);
    Embed {
        kind: match embed.kind.as_deref().filter(|kind| !kind.is_empty()) {
            None | Some("rich") => EmbedKind::Rich,
            Some("image") => EmbedKind::Image,
            Some("gifv") => EmbedKind::Gifv,
            Some("video") => EmbedKind::Video,
            Some("components") => EmbedKind::Components,
            _ if embed.color.is_some() || !embed.fields.is_empty() => EmbedKind::Rich,
            _ => EmbedKind::Link,
        },
        url: url.clone(),
        color: embed.color,
        provider: embed
            .provider
            .as_ref()
            .filter(|p| !p.name.is_empty())
            .map(|p| EmbedText {
                text: p.name.clone(),
                url: nonempty(&p.url),
                icon: None,
            }),
        author: embed
            .author
            .as_ref()
            .filter(|a| !a.name.is_empty())
            .map(|a| EmbedText {
                text: a.name.clone(),
                url: nonempty(&a.url),
                icon: media(&a.icon_url, &a.proxy_icon_url, 0, 0, false),
            }),
        title: nonempty(&embed.title).map(|text| EmbedText {
            text,
            url,
            icon: None,
        }),
        description: nonempty(&embed.description),
        fields: embed
            .fields
            .iter()
            .map(|field| EmbedField {
                name: field.name.clone(),
                value: field.value.clone(),
                inline: field.inline,
            })
            .collect(),
        components: map_components(&embed.components),
        thumbnail: embed.thumbnail.as_ref().and_then(map_media),
        images: embed
            .image
            .as_ref()
            .and_then(map_media)
            .into_iter()
            .collect(),
        video: embed.video.as_ref().and_then(map_media),
        footer: embed
            .footer
            .as_ref()
            .filter(|f| !f.text.is_empty())
            .map(|f| EmbedText {
                text: f.text.clone(),
                url: None,
                icon: media(&f.icon_url, &f.proxy_icon_url, 0, 0, false),
            }),
        timestamp: embed.timestamp.as_deref().and_then(local_timestamp),
    }
}

pub(super) fn map_embeds(embeds: &[model::Embed], flags: u64) -> Vec<Embed> {
    if flags & (1 << 2) != 0 {
        return Vec::new();
    }
    let mut mapped: Vec<Embed> = Vec::new();
    let mut group_url = None;
    for embed in embeds {
        let url = nonempty(&embed.url);
        if url.is_some()
            && url == group_url
            && embed.kind.as_deref() != Some("components")
            && mapped
                .last()
                .is_some_and(|first| first.kind != EmbedKind::Components)
        {
            let Some(first) = mapped.last_mut() else {
                continue;
            };
            // Keep a follower's video when the group has none, so it stays playable.
            if first.video.is_none() {
                first.video = embed.video.as_ref().and_then(map_media);
            }
            let media = embed
                .image
                .as_ref()
                .or(embed.thumbnail.as_ref())
                .and_then(map_media);
            if let Some(media) = media
                && first.images.len() < 4
                && !first.images.iter().any(|image| image.url == media.url)
                && !(first.images.is_empty()
                    && first
                        .thumbnail
                        .as_ref()
                        .is_some_and(|thumbnail| thumbnail.url == media.url))
            {
                first.images.push(media);
            }
        } else {
            mapped.push(map_embed(embed));
            group_url = url;
        }
    }
    mapped
}

pub(super) fn map_emoji(emoji: &model::Emoji) -> ReactionEmoji {
    ReactionEmoji {
        id: emoji.id,
        name: emoji.name.clone().unwrap_or_default(),
        animated: emoji.animated,
    }
}

pub(super) fn map_reactions(reactions: &[model::Reaction]) -> Vec<Reaction> {
    reactions
        .iter()
        .map(|reaction| Reaction {
            emoji: map_emoji(&reaction.emoji),
            count: reaction.count,
            me: reaction.me || reaction.me_burst || reaction.burst_me,
        })
        .collect()
}

pub(super) fn map_components(components: &[model::Component]) -> Vec<Component> {
    components.iter().filter_map(map_component).collect()
}

fn map_component(component: &model::Component) -> Option<Component> {
    Some(match component.kind {
        1 => Component::Row(map_components(&component.components)),
        2 => Component::Button {
            style: component.style,
            label: nonempty(&component.label),
            emoji: component.emoji.as_ref().map(map_emoji),
            url: nonempty(&component.url),
            disabled: component.disabled,
        },
        3 | 5..=8 => Component::Select {
            placeholder: nonempty(&component.placeholder),
            disabled: component.disabled,
        },
        9 => Component::Section {
            components: map_components(&component.components),
            accessory: component
                .accessory
                .as_deref()
                .and_then(map_component)
                .map(Box::new),
        },
        10 => Component::Text(component.content.clone()),
        11 => Component::Thumbnail {
            media: component.media.as_ref().and_then(map_media)?,
            description: nonempty(&component.description),
            spoiler: component.spoiler,
        },
        12 => Component::Gallery(
            component
                .items
                .iter()
                .filter_map(|item| {
                    Some(GalleryItem {
                        media: map_media(&item.media)?,
                        description: nonempty(&item.description),
                        spoiler: item.spoiler,
                    })
                })
                .collect(),
        ),
        13 => {
            let _ = &component.file;
            Component::File {
                name: component.name.clone(),
                size: component.size,
                spoiler: component.spoiler,
            }
        }
        14 => Component::Separator {
            divider: component.divider,
            large: component.spacing == 2,
        },
        17 => Component::Container {
            accent: component.accent_color,
            spoiler: component.spoiler,
            components: map_components(&component.components),
        },
        _ => return None,
    })
}

pub(super) fn map_stickers(stickers: &[model::Sticker]) -> Vec<Sticker> {
    stickers
        .iter()
        .map(|sticker| Sticker {
            id: sticker.id,
            name: sticker.name.clone(),
            format: sticker.format_type,
        })
        .collect()
}

pub(super) fn map_message(message: &model::Message) -> Option<Message> {
    Some(Message {
        id: message.id,
        author_id: message.author.id,
        timestamp: local_timestamp(&message.timestamp)?,
        delivery: Delivery::Sent,
        content: message.content.clone(),
        embeds: map_embeds(&message.embeds, message.flags),
        components: map_components(&message.components),
        reactions: map_reactions(&message.reactions),
        attachments: message.attachments.clone(),
        stickers: map_stickers(&message.sticker_items),
        kind: message.kind,
        edited: message.edited_timestamp.is_some(),
        reply: None,
        forward: map_forward(message),
        mentions_me: false,
    })
}

fn map_forward(message: &model::Message) -> Option<Forward> {
    let reference = message
        .message_reference
        .as_ref()
        .filter(|reference| reference.kind == 1)?;
    let snapshot = message.message_snapshots.first()?;
    Some(map_snapshot(
        &snapshot.message,
        reference.channel_id,
        reference.guild_id,
    ))
}

pub(super) fn map_snapshot(
    snapshot: &model::SnapshotMessage,
    channel_id: Option<u64>,
    guild_id: Option<u64>,
) -> Forward {
    Forward {
        content: snapshot.content.clone(),
        embeds: map_embeds(&snapshot.embeds, snapshot.flags),
        components: map_components(&snapshot.components),
        attachments: snapshot.attachments.clone(),
        stickers: map_stickers(&snapshot.sticker_items),
        timestamp: snapshot.timestamp.as_deref().and_then(local_timestamp),
        edited: snapshot.edited_timestamp.is_some(),
        channel_id,
        guild_id,
    }
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

pub(super) fn map_decoration(decoration: &model::AvatarDecoration) -> Decoration {
    Decoration {
        asset: decoration.asset.clone(),
        sku_id: decoration.sku_id,
    }
}

pub(super) fn map_nameplate(collectibles: &model::Collectibles) -> Option<Nameplate> {
    let nameplate = collectibles.nameplate.as_ref()?;
    Some(Nameplate {
        asset: nameplate.asset.clone(),
        palette: nameplate.palette.clone(),
        label: nameplate.label.clone(),
    })
}

pub(super) fn map_name_style(styles: &model::DisplayNameStyles) -> NameStyle {
    NameStyle {
        font_id: styles.font_id,
        effect_id: styles.effect_id,
        colors: styles.colors.clone(),
    }
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
        decoration: user
            .avatar_decoration_data
            .as_ref()
            .and_then(Option::as_ref)
            .map(map_decoration),
        nameplate: user
            .collectibles
            .as_ref()
            .and_then(Option::as_ref)
            .and_then(map_nameplate),
        name_style: user
            .display_name_styles
            .as_ref()
            .and_then(Option::as_ref)
            .map(map_name_style),
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
    // Discord lists voice and stage channels after text-like ones within each category,
    // regardless of position.
    ordered.sort_by_key(|c| {
        (
            matches!(c.kind, Some(2 | 13)),
            c.position.unwrap_or(0),
            c.id,
        )
    });
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
        banner: guild.banner(),
        emojis: guild
            .emojis
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|emoji| CustomEmoji {
                id: emoji.id,
                name: emoji.name.clone(),
                animated: emoji.animated,
                guild_id: guild.id,
                available: emoji.available,
                roles: emoji.roles.clone(),
            })
            .collect(),
    }
}
