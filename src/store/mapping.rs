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
    if !crate::media::images::allowed_url(&parsed)
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

/// Kept without a proxy URL, unlike pictures: embed players have none, and the video is only
/// sized and opened, never fetched as an image.
fn map_video(value: &model::EmbedMedia) -> Option<Media> {
    map_media(value).or_else(|| {
        let url = nonempty(&value.url)?;
        reqwest::Url::parse(&url)
            .ok()
            .filter(|parsed| matches!(parsed.scheme(), "http" | "https"))?;
        Some(Media {
            url: url.clone(),
            source: url,
            width: value.width.unwrap_or(0),
            height: value.height.unwrap_or(0),
            animated: false,
        })
    })
}

pub(super) fn map_embed(embed: &model::Embed) -> Embed {
    let url = nonempty(&embed.url);
    let kind = match embed.kind.as_deref().filter(|kind| !kind.is_empty()) {
        None | Some("rich") => EmbedKind::Rich,
        Some("image") => EmbedKind::Image,
        Some("gifv") => EmbedKind::Gifv,
        Some("video") => EmbedKind::Video,
        Some("article") => EmbedKind::Article,
        Some("components") => EmbedKind::Components,
        _ if embed.color.is_some() || !embed.fields.is_empty() => EmbedKind::Rich,
        _ => EmbedKind::Link,
    };
    let mut thumbnail = embed.thumbnail.as_ref().and_then(map_media);
    let mut image = embed.image.as_ref().and_then(map_media);
    if matches!(kind, EmbedKind::Article | EmbedKind::Image) && thumbnail.is_some() {
        let main = thumbnail.take();
        image = image.or(main);
    }
    Embed {
        kind,
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
            .filter(|_| kind == EmbedKind::Rich)
            .map(|field| EmbedField {
                name: field.name.clone(),
                value: field.value.clone(),
                inline: field.inline,
            })
            .collect(),
        components: map_components(&embed.components),
        thumbnail,
        images: image.into_iter().collect(),
        video: embed.video.as_ref().and_then(map_video),
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
                first.video = embed.video.as_ref().and_then(map_video);
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
        16 => {
            let view = component
                .content_inventory_entry
                .as_ref()
                .and_then(|value| serde_json::from_value::<model::OutboxEntry>(value.clone()).ok())
                .and_then(|entry| {
                    super::activity_feed::entry_view(&entry, chrono::Utc::now().timestamp_millis())
                })?;
            Component::ActivityEntry(view)
        }
        _ => return None,
    })
}

/// A type-16 activity card supersedes its legacy flagged embed (flags bit 5).
fn embeds_without_activity_card(
    embeds: &[model::Embed],
    components: &[model::Component],
) -> Vec<model::Embed> {
    if components.iter().any(|component| component.kind == 16) {
        embeds
            .iter()
            .filter(|embed| embed.flags & (1 << 5) == 0)
            .cloned()
            .collect()
    } else {
        embeds.to_vec()
    }
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

pub(super) fn map_webhook_author(id: Option<u64>, author: &model::User) -> Option<WebhookAuthor> {
    Some(WebhookAuthor {
        id: id?,
        name: author.username.clone(),
        avatar: author.avatar.clone(),
        verified_bot: model::verified_bot(author.bot, author.public_flags),
    })
}

pub(super) fn message_thread(thread: &model::Channel) -> MessageThread {
    MessageThread {
        id: thread.id,
        name: thread.name.clone().unwrap_or_default(),
        message_count: thread.message_count.unwrap_or(0),
        archived: thread
            .thread_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.archived),
        recent: None,
    }
}

/// A THREAD_CREATED notice names its thread in `content` and `message_reference`, with no `thread`.
fn thread_notice(message: &model::Message) -> Option<MessageThread> {
    if message.kind != 18 {
        return None;
    }
    Some(MessageThread {
        id: message.message_reference.as_ref()?.channel_id?,
        name: message.content.clone(),
        message_count: 0,
        archived: false,
        recent: None,
    })
}

pub(super) fn map_message(message: &model::Message) -> Option<Message> {
    Some(Message {
        id: message.id,
        send_order: None,
        author_id: message.author.id,
        webhook: map_webhook_author(message.webhook_id, &message.author),
        timestamp: local_timestamp(&message.timestamp)?,
        delivery: Delivery::Sent,
        content: message.content.clone(),
        embeds: map_embeds(
            &embeds_without_activity_card(&message.embeds, &message.components),
            message.flags,
        ),
        components: map_components(&message.components),
        reactions: map_reactions(&message.reactions),
        attachments: message.attachments.clone(),
        stickers: map_stickers(&message.sticker_items),
        kind: message.kind,
        edited: message.edited_timestamp.is_some(),
        reply: None,
        forward: map_forward(message),
        forwardable: is_forwardable(message),
        mentions_me: false,
        uploads: Vec::new(),
        thread: message
            .thread
            .as_ref()
            .map(message_thread)
            .or_else(|| thread_notice(message)),
        reference_id: message
            .message_reference
            .as_ref()
            .and_then(|reference| reference.message_id),
        call: message
            .call
            .as_ref()
            .map(|call| map_call(call, message.channel_id, message.id)),
    })
}

pub(super) fn map_call(call: &model::MessageCall, channel_id: u64, message_id: u64) -> CallRecord {
    const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;
    let started_ms = (message_id >> 22) as i64 + DISCORD_EPOCH_MS;
    let duration_secs = call
        .ended_timestamp
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|ended| ((ended.timestamp_millis() - started_ms).max(0) as u64 + 500) / 1000);
    CallRecord {
        channel_id,
        participants: call.participants.clone(),
        duration_secs,
    }
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
        reference.message_id,
    ))
}

pub(super) fn map_snapshot(
    snapshot: &model::SnapshotMessage,
    channel_id: Option<u64>,
    guild_id: Option<u64>,
    message_id: Option<u64>,
) -> Forward {
    Forward {
        message_id,
        content: snapshot.content.clone(),
        embeds: map_embeds(
            &embeds_without_activity_card(&snapshot.embeds, &snapshot.components),
            snapshot.flags,
        ),
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
        username: user.username.clone(),
        initials: initials(&name),
        name,
        status: Status::Offline,
        bot: user.bot,
        verified_bot: model::verified_bot(user.bot, user.public_flags),
        on_mobile: false,
        role_color: None,
        tag: user.primary_guild.as_ref().and_then(guild_tag),
        playing: false,
        activity: None,
        activity_kind: None,
        game: None,
        listening: None,
        live_stream: None,
        custom_status: None,
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

pub(super) fn apply_user_update(user: &mut User, update: &model::UserUpdate) {
    let old_username = user.username.clone();
    let username_changed = !update.username.is_empty() && update.username != user.username;
    if username_changed {
        user.username.clone_from(&update.username);
    }
    match &update.global_name {
        Some(name) => user.name = name.clone().unwrap_or_else(|| user.username.clone()),
        None if username_changed && user.name == old_username => {
            user.name.clone_from(&user.username);
        }
        None => {}
    }
    if username_changed || update.global_name.is_some() {
        user.initials = initials(&user.name);
    }
    if let Some(avatar) = &update.avatar {
        user.avatar.clone_from(avatar);
    }
    if let Some(discriminator) = &update.discriminator {
        user.discriminator.clone_from(discriminator);
    }
    if let Some(bot) = update.bot {
        user.bot = bot;
    }
    if update.public_flags.is_some() {
        user.verified_bot = model::verified_bot(user.bot, update.public_flags);
    }
    user.verified_bot &= user.bot;
    if let Some(decoration) = &update.avatar_decoration_data {
        user.decoration = decoration.as_ref().map(map_decoration);
    }
    if let Some(collectibles) = &update.collectibles {
        user.nameplate = collectibles.as_ref().and_then(map_nameplate);
    }
    if let Some(styles) = &update.display_name_styles {
        user.name_style = styles.as_ref().map(map_name_style);
    }
    if let Some(primary) = &update.primary_guild {
        user.tag = guild_tag(primary);
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

pub(super) fn forum_new_posts(
    guild: &model::Guild,
    forum_id: u64,
    state: Option<&model::ReadState>,
) -> u32 {
    let last_read = state.and_then(|state| state.last_message_id).unwrap_or(0);
    let count = guild
        .threads
        .iter()
        .flatten()
        .filter(|thread| {
            thread.parent_id.flatten() == Some(forum_id)
                && thread.id > last_read
                && !thread
                    .thread_metadata
                    .as_ref()
                    .is_some_and(|metadata| metadata.archived)
        })
        .count();
    count.min(u32::MAX as usize) as u32
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
            matches!(c.kind, Some(0 | 5 | 15 | 16 | 2 | 13))
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
    for channel in ordered {
        let kind = match channel.kind {
            Some(2 | 13) => ChannelKind::Voice,
            Some(5) => ChannelKind::Announcement,
            Some(15 | 16) => ChannelKind::Forum,
            _ => ChannelKind::Text,
        };
        let state = read_state(source, read_states, channel.id);
        let mention_count = state.map_or(0, |s| s.mention_count);
        let new_posts = if kind.is_forum() {
            forum_new_posts(guild, channel.id, state)
        } else {
            0
        };
        let index = channel
            .parent_id
            .flatten()
            .and_then(|id| category_indices.get(&id))
            .copied()
            .unwrap_or(0);
        let private = channel
            .permission_overwrites
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|overwrite| {
                overwrite.id == guild.id
                    && overwrite.kind == 0
                    && overwrite.deny & discord::VIEW_CHANNEL != 0
            });
        mapped[index].channels.push(Channel {
            id: channel.id,
            name: channel.name.clone().unwrap_or_default(),
            kind,
            parent_id: channel.parent_id.flatten(),
            private,
            unread: kind.is_unreadable() && (channel_unread(channel, state) || new_posts > 0),
            mentions: mention_count,
            new_posts,
            messages: Vec::new(),
            messages_loaded: false,
        });
    }
    let threads = guild
        .threads
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter(|thread| {
            matches!(thread.kind, Some(10..=12))
                && guild.unavailable != Some(true)
                && discord::can_view(guild, thread, source.user.id)
        })
        .map(|thread| {
            let state = read_state(source, read_states, thread.id);
            Channel {
                id: thread.id,
                name: thread.name.clone().unwrap_or_default(),
                kind: ChannelKind::Text,
                parent_id: thread.parent_id.flatten(),
                private: false,
                unread: channel_unread(thread, state),
                mentions: state.map_or(0, |s| s.mention_count),
                new_posts: 0,
                messages: Vec::new(),
                messages_loaded: false,
            }
        })
        .collect();
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
        premium_tier: guild.premium_tier,
        unread: mapped
            .iter()
            .flat_map(|c| &c.channels)
            .any(|c| c.kind.is_unreadable() && c.unread),
        mentions: mapped
            .iter()
            .flat_map(|c| &c.channels)
            .map(|c| u64::from(c.mentions))
            .sum::<u64>()
            .min(u64::from(u32::MAX)) as u32,
        categories: mapped,
        threads,
        member_ids: members.iter().filter_map(|m| m.id()).collect(),
        icon: guild.icon(),
        banner: guild.banner(),
        tag: guild.tag().map(|(tag, badge)| GuildTag {
            text: tag.to_owned(),
            guild_id: guild.id,
            badge: badge.map(str::to_owned),
        }),
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
