mod member_list;
mod members;
mod message_info;
mod presence;

use super::*;
use crate::media::images::ImageSpec;
use std::{
    collections::HashSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const EMBEDDED_ACTIVITY_FLAG: u64 = 1 << 8;
const PLAY_ACTIVITY_FLAG: u64 = 1 << 5;

#[derive(Default)]
pub(super) struct Social {
    pub guilds: HashMap<u64, GuildSocial>,
    pub names: HashMap<u64, String>,
    messages: HashMap<(u64, u64), MessageInfo>,
    pub revision: u64,
    pub close_contacts: HashSet<u64>,
    own_custom_status: Option<CustomStatus>,
}

#[derive(Default)]
pub(super) struct GuildSocial {
    pub members: HashMap<u64, MemberInfo>,
    pub roles: HashMap<u64, Role>,
    pub list: Option<MemberList>,
    pub list_groups: Vec<Option<String>>,
    requested: HashSet<u64>,
}

#[derive(Default)]
pub(super) struct MemberInfo {
    pub nick: Option<String>,
    pub roles: Vec<u64>,
    pub avatar: Option<String>,
    pub color: Option<u32>,
    pub nameplate: Option<Nameplate>,
    pub decoration: Option<Decoration>,
    pub name_style: Option<NameStyle>,
}

#[derive(Default)]
struct MessageInfo {
    flags: u64,
    embeds: Vec<model::Embed>,
    guild_id: Option<u64>,
    author_id: u64,
    webhook_id: Option<u64>,
    mentions: Vec<u64>,
    roles: Vec<u64>,
    everyone: bool,
    reference: Option<model::MessageReference>,
    referenced: Option<Option<Box<model::ReferencedMessage>>>,
}

pub(super) fn status(value: &str) -> Status {
    match value {
        "online" => Status::Online,
        "idle" => Status::Idle,
        "dnd" => Status::DoNotDisturb,
        _ => Status::Offline,
    }
}

fn custom_text(text: &str, emoji: Option<&str>) -> Option<String> {
    let text = match emoji.filter(|e| !e.is_empty()) {
        Some(emoji) if !text.is_empty() => format!("{emoji} {text}"),
        Some(emoji) => emoji.to_owned(),
        None => text.to_owned(),
    };
    (!text.is_empty()).then_some(text)
}

fn status_emoji(id: u64, name: &str, animated: bool) -> Option<StatusEmoji> {
    if id != 0 {
        Some(StatusEmoji::Custom {
            id,
            name: name.to_owned(),
            animated,
        })
    } else if !name.is_empty() {
        Some(StatusEmoji::Unicode(name.to_owned()))
    } else {
        None
    }
}

fn custom_status_from_settings(status: &settings_proto::CustomStatus) -> CustomStatus {
    CustomStatus {
        text: (!status.text.is_empty()).then(|| status.text.clone()),
        emoji: status_emoji(status.emoji_id, &status.emoji_name, false),
        expires_at: (status.expires_at_ms != 0)
            .then(|| UNIX_EPOCH.checked_add(Duration::from_millis(status.expires_at_ms)))
            .flatten(),
    }
}

fn custom_status_from_presence(activities: &[model::Activity]) -> Option<CustomStatus> {
    activities
        .iter()
        .filter(|activity| activity.kind == 4)
        .find_map(|activity| {
            let text = activity
                .state
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(str::to_owned);
            let emoji = activity.emoji.as_ref().and_then(|emoji| {
                status_emoji(
                    emoji.id.unwrap_or(0),
                    emoji.name.as_deref().unwrap_or_default(),
                    emoji.animated,
                )
            });
            (text.is_some() || emoji.is_some()).then_some(CustomStatus {
                text,
                emoji,
                expires_at: None,
            })
        })
}

fn show_custom_status(user: &mut User, status: &CustomStatus) {
    let emoji = match &status.emoji {
        Some(StatusEmoji::Unicode(emoji)) => Some(emoji.as_str()),
        _ => None,
    };
    user.playing = false;
    user.activity_kind = None;
    user.activity = custom_text(status.text.as_deref().unwrap_or_default(), emoji);
    user.custom_status = Some(status.clone());
}

fn same_custom_status(a: &CustomStatus, b: &CustomStatus) -> bool {
    a.text == b.text
        && match (a.emoji.as_ref(), b.emoji.as_ref()) {
            (None, None) => true,
            (Some(StatusEmoji::Unicode(a)), Some(StatusEmoji::Unicode(b))) => a == b,
            (
                Some(StatusEmoji::Custom { id: a_id, .. }),
                Some(StatusEmoji::Custom { id: b_id, .. }),
            ) => a_id == b_id,
            _ => false,
        }
}

pub(super) fn activity(activities: &[model::Activity]) -> Option<(String, bool, Option<u8>)> {
    for activity in activities.iter().filter(|a| a.kind == 4) {
        let emoji = activity
            .emoji
            .as_ref()
            .filter(|e| e.id.is_none())
            .and_then(|e| e.name.as_deref());
        if let Some(text) = custom_text(activity.state.as_deref().unwrap_or_default(), emoji) {
            return Some((text, false, None));
        }
    }
    let activity = playing_activity(activities)?;
    let verb = activity_verb(activity.kind)?;
    let name = activity_name(activity);
    let displayed = match activity.status_display_type {
        Some(1) => activity_text(activity.state.as_deref()),
        Some(2) => activity_text(activity.details.as_deref()),
        _ => None,
    };
    Some(if let Some(text) = displayed {
        (text, verb.is_empty(), Some(activity.kind))
    } else if verb.is_empty() {
        (name.to_owned(), true, Some(activity.kind))
    } else {
        (format!("{verb} {name}"), false, Some(activity.kind))
    })
}

fn playing_activity(activities: &[model::Activity]) -> Option<&model::Activity> {
    activities.iter().find(|activity| {
        activity_verb(activity.kind).is_some() && !activity_name(activity).is_empty()
    })
}

fn live_stream(activity: &model::Activity) -> LiveStream {
    let twitch_login = activity
        .assets
        .as_ref()
        .and_then(|assets| assets.large_image.as_deref()?.strip_prefix("twitch:"))
        .filter(|login| {
            !login.is_empty()
                && login
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        .map(str::to_lowercase);
    LiveStream {
        service: activity.name.clone(),
        title: activity_text(activity.details.as_deref()),
        url: activity_text(activity.url.as_deref()),
        twitch_login,
    }
}

fn game_activity(activity: &model::Activity) -> GameActivity {
    let assets = activity.assets.as_ref();
    let platform = activity
        .platform
        .as_deref()
        .map(str::trim)
        .filter(|platform| !platform.is_empty())
        .map(str::to_lowercase);
    let embedded =
        activity.flags & EMBEDDED_ACTIVITY_FLAG != 0 || platform.as_deref() == Some("embedded");
    let playable = activity.flags & PLAY_ACTIVITY_FLAG != 0;
    GameActivity {
        name: activity.name.clone(),
        application_id: activity.application_id,
        kind: activity.kind,
        platform,
        embedded,
        playable,
        started_at: activity
            .timestamps
            .as_ref()
            .and_then(|timestamps| timestamps.start)
            .or(activity.created_at)
            .and_then(|start| UNIX_EPOCH.checked_add(Duration::from_millis(start))),
        ends_at: activity
            .timestamps
            .as_ref()
            .and_then(|timestamps| timestamps.end)
            .and_then(|end| UNIX_EPOCH.checked_add(Duration::from_millis(end))),
        details: activity_text(activity.details.as_deref()),
        state: activity_text(activity.state.as_deref()),
        large_image: activity_image(
            activity.application_id,
            assets.and_then(|assets| assets.large_image.as_deref()),
            assets.and_then(|assets| assets.large_text.as_deref()),
        ),
        small_image: activity_image(
            activity.application_id,
            assets.and_then(|assets| assets.small_image.as_deref()),
            assets.and_then(|assets| assets.small_text.as_deref()),
        ),
    }
}

fn activity_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn activity_image(
    application_id: Option<u64>,
    key: Option<&str>,
    text: Option<&str>,
) -> Option<ActivityImage> {
    let key = key?;
    if key.trim().is_empty() {
        return None;
    }
    let url = if key.bytes().all(|byte| byte.is_ascii_digit()) {
        let application_id = application_id?;
        format!("https://cdn.discordapp.com/app-assets/{application_id}/{key}.png")
    } else if let Some(path) = key.strip_prefix("mp:") {
        if path.trim().is_empty() {
            return None;
        }
        format!("https://media.discordapp.net/{path}")
    } else {
        let id = key.strip_prefix("spotify:")?;
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return None;
        }
        format!("https://i.scdn.co/image/{id}")
    };
    Some(ActivityImage {
        url,
        text: activity_text(text),
    })
}

fn activity_verb(kind: u8) -> Option<&'static str> {
    Some(match kind {
        0 => "",
        1 => "Streaming",
        2 => "Listening to",
        3 => "Watching",
        5 => "Competing in",
        _ => return None,
    })
}

fn activity_name(activity: &model::Activity) -> &str {
    if activity.kind == 1 {
        activity
            .details
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&activity.name)
    } else {
        &activity.name
    }
}

fn map_role(role: &model::Role) -> Role {
    let color = if role.colors.primary_color != 0 {
        role.colors.primary_color
    } else {
        role.color
    };
    Role {
        id: role.id,
        name: role.name.clone(),
        color: (color != 0).then_some(color),
        position: role.position,
        hoist: role.hoist,
        icon: role.icon.clone().filter(|icon| !icon.is_empty()),
        unicode_emoji: role.unicode_emoji.clone().filter(|emoji| !emoji.is_empty()),
    }
}

fn member_color(roles: &HashMap<u64, Role>, member: &MemberInfo) -> Option<u32> {
    member
        .roles
        .iter()
        .filter_map(|id| roles.get(id))
        .filter(|role| role.color.is_some())
        .max_by_key(|role| (role.position, std::cmp::Reverse(role.id)))
        .and_then(|role| role.color)
}

fn group_name<'a>(roles: &'a HashMap<u64, Role>, id: &str) -> &'a str {
    match id {
        "online" => "Online",
        "offline" => "Offline",
        id => id
            .parse()
            .ok()
            .and_then(|id| roles.get(&id))
            .map(|r| r.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("Role"),
    }
}

fn refresh_group_names(guild: &mut GuildSocial) {
    if let Some(list) = &mut guild.list {
        for (row, id) in list.rows.iter_mut().zip(&guild.list_groups) {
            if let (Some(MemberRow::Group { name, .. }), Some(id)) = (row, id) {
                *name = group_name(&guild.roles, id).to_owned();
            }
        }
    }
}

impl Store {
    pub(super) fn init_social(&mut self) {
        let Some(source) = &self.source else {
            return;
        };
        self.social.guilds = source
            .guilds
            .iter()
            .map(|g| {
                let roles = g
                    .roles
                    .iter()
                    .flatten()
                    .map(|r| (r.id, map_role(r)))
                    .collect();
                let mut social = GuildSocial {
                    roles,
                    ..Default::default()
                };
                for member in g.members.iter().flatten() {
                    if let Some(id) = member.id() {
                        let mut info = MemberInfo {
                            nick: member.nick.clone(),
                            roles: member.roles.clone(),
                            avatar: member.avatar.clone(),
                            color: None,
                            nameplate: member
                                .collectibles
                                .as_ref()
                                .and_then(Option::as_ref)
                                .and_then(map_nameplate),
                            decoration: member
                                .avatar_decoration_data
                                .as_ref()
                                .and_then(Option::as_ref)
                                .map(map_decoration),
                            name_style: member
                                .display_name_styles
                                .as_ref()
                                .and_then(Option::as_ref)
                                .map(map_name_style),
                        };
                        info.color = member_color(&social.roles, &info);
                        social.members.insert(id, info);
                    }
                }
                (g.id, social)
            })
            .collect();
    }

    pub(super) fn sync_guild_social(&mut self, guild_id: u64) {
        let Some(guild) = self
            .source
            .as_ref()
            .and_then(|s| s.guilds.iter().find(|g| g.id == guild_id))
        else {
            return;
        };
        let roles = guild
            .roles
            .as_ref()
            .map(|roles| roles.iter().map(|r| (r.id, map_role(r))).collect());
        let members: Vec<_> = guild
            .members
            .iter()
            .flatten()
            .map(model::PartialMember::from)
            .collect();
        if let Some(roles) = roles {
            let guild = self.social.guilds.entry(guild_id).or_default();
            guild.roles = roles;
            for member in guild.members.values_mut() {
                member.color = member_color(&guild.roles, member);
            }
            refresh_group_names(guild);
        }
        for member in members {
            if let Some(id) = member.id() {
                self.upsert_member(guild_id, id, &member);
            }
        }
    }

    pub(super) fn index_names(&mut self) {
        self.social.names.clear();
        if let Some(source) = &self.source {
            for channel in source
                .guilds
                .iter()
                .flat_map(|g| {
                    g.threads
                        .iter()
                        .flatten()
                        .chain(g.channels.iter().flatten())
                })
                .chain(&source.private_channels)
            {
                if let Some(name) = &channel.name {
                    self.social.names.insert(channel.id, name.clone());
                } else if matches!(channel.kind, Some(1 | 3)) {
                    let names: Vec<_> = channel
                        .recipient_ids
                        .iter()
                        .copied()
                        .chain(channel.recipients.iter().map(|user| user.id))
                        .filter(|id| *id != self.current_user_id)
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .filter_map(|id| self.users.get(&id).map(|u| (id, u.name.as_str())))
                        .collect();
                    let mut names = names;
                    names.sort_by_key(|(id, _)| *id);
                    let name = names
                        .into_iter()
                        .map(|(_, name)| name)
                        .collect::<Vec<_>>()
                        .join(", ");
                    if !name.is_empty() {
                        self.social.names.insert(channel.id, name);
                    }
                }
            }
        } else {
            for guild in &self.guilds {
                for category in &guild.categories {
                    self.social.names.insert(category.id, category.name.clone());
                    for channel in &category.channels {
                        self.social.names.insert(channel.id, channel.name.clone());
                    }
                }
            }
        }
    }

    pub fn channel_guild_id(&self, channel_id: u64) -> Option<u64> {
        self.message_guild(channel_id, None)
    }

    fn message_guild(&self, channel_id: u64, guild_id: Option<u64>) -> Option<u64> {
        guild_id.or_else(|| match self.source_channels.get(&channel_id)? {
            SourceChannel::Guild { guild, .. } | SourceChannel::Thread { guild, .. } => {
                Some(self.source.as_ref()?.guilds.get(*guild)?.id)
            }
            SourceChannel::Private(_) => None,
        })
    }

    #[allow(clippy::result_large_err)]
    pub(super) fn apply_social(&mut self, event: discord::Event) -> Result<Change, discord::Event> {
        use discord::Event;
        let guild_id = match event {
            Event::ReadySupplemental(update) => {
                let ids: Vec<_> = if update.guilds.is_empty() {
                    self.source
                        .as_ref()
                        .map(|s| s.guild_order.clone())
                        .unwrap_or_default()
                } else {
                    update
                        .guilds
                        .iter()
                        .map(|g| g.get("id").and_then(model::id_value))
                        .collect()
                };
                for (index, members) in update.merged_members.iter().enumerate() {
                    if let Some(Some(guild)) = ids.get(index) {
                        for member in members {
                            if let Some(id) = member.id() {
                                self.upsert_member(*guild, id, member);
                            }
                        }
                    }
                }
                for presence in update
                    .merged_presences
                    .friends
                    .iter()
                    .chain(update.merged_presences.guilds.iter().flatten())
                {
                    self.apply_presence(presence, None);
                }
                None
            }
            Event::PresenceUpdate(presence) => {
                self.apply_presence(&presence, None);
                presence.guild_id
            }
            Event::SessionsReplace(sessions) => {
                self.apply_sessions(&sessions);
                None
            }
            Event::MembersChunk(chunk) => {
                for member in &chunk.members {
                    if let Some(id) = member.id() {
                        self.upsert_member(chunk.guild_id, id, member);
                    }
                }
                for presence in &chunk.presences {
                    self.apply_presence(presence, None);
                }
                self.social
                    .guilds
                    .entry(chunk.guild_id)
                    .or_default()
                    .requested
                    .extend(chunk.not_found);
                Some(chunk.guild_id)
            }
            Event::MemberListUpdate(update) => {
                if !self.apply_member_list(&update) {
                    return Ok(Change::None);
                }
                Some(update.guild_id)
            }
            Event::MemberUpsert(update) => {
                if let Some(id) = update.member.id()
                    && self.upsert_member(update.guild_id, id, &update.member)
                {
                    self.rebuild_structure(Some(update.guild_id));
                    return Ok(Change::Structure);
                }
                Some(update.guild_id)
            }
            Event::MemberRemove(update) => {
                if let Some(id) = update.member.id() {
                    if let Some(guild) = self.social.guilds.get_mut(&update.guild_id) {
                        guild.members.remove(&id);
                        guild.requested.insert(id);
                    }
                    if let Some(index) = self.guild_indices.get(&update.guild_id) {
                        self.guilds[*index]
                            .member_ids
                            .retain(|member| *member != id);
                    }
                    if id == self.current_user_id {
                        if let Some(source) = &mut self.source
                            && let Some(guild) =
                                source.guilds.iter_mut().find(|g| g.id == update.guild_id)
                            && let Some(members) = &mut guild.members
                        {
                            members.retain(|m| m.id() != Some(id));
                        }
                        self.rebuild_structure(Some(update.guild_id));
                        return Ok(Change::Structure);
                    }
                }
                Some(update.guild_id)
            }
            Event::RoleUpsert(update) => {
                let guild = self.social.guilds.entry(update.guild_id).or_default();
                guild.roles.insert(update.role.id, map_role(&update.role));
                for member in guild.members.values_mut() {
                    member.color = member_color(&guild.roles, member);
                }
                refresh_group_names(guild);
                if let Some(source) = &mut self.source
                    && let Some(guild) = source.guilds.iter_mut().find(|g| g.id == update.guild_id)
                {
                    let roles = guild.roles.get_or_insert_with(Vec::new);
                    if let Some(role) = roles.iter_mut().find(|r| r.id == update.role.id) {
                        *role = update.role;
                    } else {
                        roles.push(update.role);
                    }
                }
                self.rebuild_structure(Some(update.guild_id));
                return Ok(Change::Structure);
            }
            Event::RoleDelete { guild_id, role_id } => {
                if let Some(guild) = self.social.guilds.get_mut(&guild_id) {
                    guild.roles.remove(&role_id);
                    for member in guild.members.values_mut() {
                        member.color = member_color(&guild.roles, member);
                    }
                    refresh_group_names(guild);
                }
                if let Some(source) = &mut self.source
                    && let Some(guild) = source.guilds.iter_mut().find(|g| g.id == guild_id)
                    && let Some(roles) = &mut guild.roles
                {
                    roles.retain(|r| r.id != role_id);
                }
                self.rebuild_structure(Some(guild_id));
                return Ok(Change::Structure);
            }
            event => return Err(event),
        };
        Ok(Change::Members { guild_id })
    }
}
