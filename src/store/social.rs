use super::*;
use crate::images::ImageSpec;
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct Social {
    pub guilds: HashMap<u64, GuildSocial>,
    pub names: HashMap<u64, String>,
    messages: HashMap<(u64, u64), MessageInfo>,
    pub revision: u64,
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
}

#[derive(Default)]
struct MessageInfo {
    guild_id: Option<u64>,
    author_id: u64,
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

pub(super) fn activity(activities: &[model::Activity]) -> Option<(String, bool)> {
    for activity in activities.iter().filter(|a| a.kind == 4) {
        let emoji = activity
            .emoji
            .as_ref()
            .filter(|e| e.id.is_none())
            .and_then(|e| e.name.as_deref());
        if let Some(text) = custom_text(activity.state.as_deref().unwrap_or_default(), emoji) {
            return Some((text, false));
        }
    }
    activities.iter().find_map(|a| {
        let verb = match a.kind {
            0 => "",
            1 => "Streaming",
            2 => "Listening to",
            3 => "Watching",
            5 => "Competing in",
            _ => return None,
        };
        let name = if a.kind == 1 {
            a.details
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(&a.name)
        } else {
            &a.name
        };
        (!name.is_empty()).then(|| {
            if verb.is_empty() {
                (name.to_owned(), true)
            } else {
                (format!("{verb} {name}"), false)
            }
        })
    })
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
    pub fn member_name(&self, guild_id: u64, user_id: u64) -> Option<&str> {
        let user = self.user(user_id).filter(|user| !user.name.is_empty())?;
        Some(
            self.social
                .guilds
                .get(&guild_id)
                .and_then(|g| g.members.get(&user_id))
                .and_then(|m| m.nick.as_deref())
                .filter(|name| !name.is_empty())
                .unwrap_or(&user.name),
        )
    }

    pub fn member_color(&self, guild_id: u64, user_id: u64) -> Option<u32> {
        self.social
            .guilds
            .get(&guild_id)?
            .members
            .get(&user_id)?
            .color
    }

    pub fn role(&self, guild_id: u64, role_id: u64) -> Option<&Role> {
        self.social.guilds.get(&guild_id)?.roles.get(&role_id)
    }

    pub fn channel_name(&self, channel_id: u64) -> Option<&str> {
        self.social.names.get(&channel_id).map(String::as_str)
    }

    pub fn member_list(&self, guild_id: u64) -> Option<&MemberList> {
        self.social.guilds.get(&guild_id)?.list.as_ref()
    }

    pub fn missing_members(
        &mut self,
        guild_id: u64,
        user_ids: impl IntoIterator<Item = u64>,
    ) -> Vec<u64> {
        let guild = self.social.guilds.entry(guild_id).or_default();
        user_ids
            .into_iter()
            .filter(|id| !guild.members.contains_key(id) && guild.requested.insert(*id))
            .collect()
    }

    pub fn member_avatar_spec(&self, guild_id: u64, user_id: u64) -> Option<ImageSpec> {
        Some(ImageSpec::GuildAvatar {
            guild_id,
            user_id,
            hash: self
                .social
                .guilds
                .get(&guild_id)?
                .members
                .get(&user_id)?
                .avatar
                .clone()?,
        })
    }

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

    fn message_guild(&self, channel_id: u64, guild_id: Option<u64>) -> Option<u64> {
        guild_id.or_else(|| match self.source_channels.get(&channel_id)? {
            SourceChannel::Guild { guild, .. } | SourceChannel::Thread { guild, .. } => {
                Some(self.source.as_ref()?.guilds.get(*guild)?.id)
            }
            SourceChannel::Private(_) => None,
        })
    }

    fn upsert_member(&mut self, guild_id: u64, id: u64, member: &model::PartialMember) -> bool {
        if let Some(user) = &member.user {
            self.register_author(user);
        }
        let guild = self.social.guilds.entry(guild_id).or_default();
        let new = !guild.members.contains_key(&id);
        let info = guild.members.entry(id).or_default();
        let own_roles_changed = id == self.current_user_id
            && member
                .roles
                .as_ref()
                .is_some_and(|roles| *roles != info.roles);
        if let Some(nick) = &member.nick {
            info.nick.clone_from(nick);
        }
        if let Some(roles) = &member.roles {
            info.roles.clone_from(roles);
        }
        if let Some(avatar) = &member.avatar {
            info.avatar.clone_from(avatar);
        }
        info.color = member_color(&guild.roles, info);
        if new {
            if let Some(index) = self.guild_indices.get(&guild_id) {
                self.guilds[*index].member_ids.push(id);
            }
        }
        if id == self.current_user_id {
            if let Some(source) = &mut self.source {
                if let Some(guild) = source.guilds.iter_mut().find(|g| g.id == guild_id) {
                    let members = guild.members.get_or_insert_with(Vec::new);
                    if let Some(own) = members.iter_mut().find(|m| m.id() == Some(id)) {
                        own.nick.clone_from(&info.nick);
                        own.roles.clone_from(&info.roles);
                        own.avatar.clone_from(&info.avatar);
                    } else {
                        members.push(model::Member {
                            user_id: Some(id),
                            user: None,
                            nick: info.nick.clone(),
                            roles: info.roles.clone(),
                            avatar: info.avatar.clone(),
                        });
                    }
                }
            }
        }
        if let Some(presence) = &member.presence {
            self.apply_presence(presence, Some(id));
        }
        if own_roles_changed {
            self.social.revision = self.social.revision.wrapping_add(1);
            self.rebuild_structure(Some(guild_id));
        }
        own_roles_changed
    }

    fn apply_presence(&mut self, presence: &model::Presence, fallback: Option<u64>) {
        if let Some(user) = &presence.user {
            self.register_author(user);
        }
        let Some(id) = presence
            .user_id
            .or_else(|| presence.user.as_ref().map(|u| u.id))
            .or(fallback)
        else {
            return;
        };
        let user = self.users.entry(id).or_insert_with(|| {
            map_user(&model::User {
                id,
                username: String::new(),
                global_name: None,
                avatar: None,
                discriminator: None,
                bot: false,
                primary_guild: None,
            })
        });
        if let Some(value) = &presence.status {
            user.status = status(value);
        }
        if let Some(activities) = &presence.activities {
            let current = activity(activities);
            user.playing = current.as_ref().is_some_and(|(_, playing)| *playing);
            user.activity = current.map(|(text, _)| text);
        }
    }

    pub(super) fn apply_own_status(
        &mut self,
        settings: Option<&settings_proto::StatusSettings>,
        partial: bool,
    ) {
        let Some(user) = self.users.get_mut(&self.current_user_id) else {
            return;
        };
        if !partial || settings.and_then(|s| s.status.as_ref()).is_some() {
            user.status = match settings.and_then(|s| s.status.as_deref()) {
                Some("idle") => Status::Idle,
                Some("dnd") => Status::DoNotDisturb,
                Some("invisible" | "offline") => Status::Offline,
                _ => Status::Online,
            };
        }
        if !partial || settings.is_some_and(|s| s.custom_status_present) {
            user.playing = false;
            user.activity = settings
                .and_then(|s| s.custom_status.as_ref())
                .and_then(|s| {
                    custom_text(&s.text, (s.emoji_id == 0).then_some(s.emoji_name.as_str()))
                });
        }
    }

    fn reply(info: &MessageInfo) -> Option<ReplyState> {
        if info.reference.as_ref()?.kind != 0 {
            return None;
        }
        Some(match &info.referenced {
            Some(Some(message)) => ReplyState::Loaded(Reply {
                message_id: message.id,
                author_id: message.author.id,
                content: message.content.clone(),
                attachments: !message.attachments.is_empty(),
                embeds: !message.embeds.is_empty(),
            }),
            Some(None) => ReplyState::Deleted,
            None => ReplyState::Unavailable,
        })
    }

    fn mentioned(&self, info: &MessageInfo) -> bool {
        info.everyone
            || info.mentions.contains(&self.current_user_id)
            || info
                .guild_id
                .and_then(|id| self.social.guilds.get(&id))
                .and_then(|g| g.members.get(&self.current_user_id))
                .is_some_and(|member| info.roles.iter().any(|id| member.roles.contains(id)))
    }

    pub(super) fn clear_message_info(&mut self, channel_id: u64) {
        self.social
            .messages
            .retain(|(channel, _), _| *channel != channel_id);
    }

    pub(super) fn forget_message_info(&mut self, channel_id: u64, message_id: u64) {
        self.social.messages.remove(&(channel_id, message_id));
    }

    pub(super) fn map_network_message(
        &mut self,
        message: &model::Message,
        remember: bool,
    ) -> Option<Message> {
        self.register_author(&message.author);
        for user in &message.mentions {
            self.register_author(user);
        }
        if let Some(Some(reply)) = &message.referenced_message {
            self.register_author(&reply.author);
        }
        let guild_id = self.message_guild(message.channel_id, message.guild_id);
        if let (Some(guild), Some(member)) = (guild_id, &message.member) {
            self.upsert_member(guild, message.author.id, member);
        }
        if !remember {
            return None;
        }
        let info = MessageInfo {
            guild_id,
            author_id: message.author.id,
            mentions: message.mentions.iter().map(|u| u.id).collect(),
            roles: message.mention_roles.clone(),
            everyone: message.mention_everyone,
            reference: message.message_reference.clone(),
            referenced: message.referenced_message.clone(),
        };
        let mut mapped = map_message(message)?;
        mapped.reply = Self::reply(&info);
        mapped.mentions_me = self.mentioned(&info);
        self.social
            .messages
            .insert((message.channel_id, message.id), info);
        Some(mapped)
    }

    pub(super) fn update_message_info(
        &mut self,
        update: &model::MessageUpdate,
    ) -> (Option<ReplyState>, bool) {
        if let Some(user) = &update.author {
            self.register_author(user);
        }
        for user in update.mentions.iter().flatten() {
            self.register_author(user);
        }
        if let Some(Some(reply)) = &update.referenced_message {
            self.register_author(&reply.author);
        }
        let key = (update.channel_id, update.id);
        let known = self.social.messages.contains_key(&key);
        let mut info = self.social.messages.remove(&key).unwrap_or_default();
        info.guild_id = self
            .message_guild(update.channel_id, update.guild_id)
            .or(info.guild_id);
        if let Some(author) = &update.author {
            info.author_id = author.id;
        }
        if let (Some(guild), Some(member)) = (info.guild_id, &update.member) {
            if info.author_id != 0 {
                self.upsert_member(guild, info.author_id, member);
            }
        }
        if let Some(mentions) = &update.mentions {
            info.mentions = mentions.iter().map(|u| u.id).collect();
        }
        if let Some(roles) = &update.mention_roles {
            info.roles.clone_from(roles);
        }
        if let Some(everyone) = update.mention_everyone {
            info.everyone = everyone;
        }
        if let Some(reference) = &update.message_reference {
            if &info.reference != reference && update.referenced_message.is_none() {
                info.referenced = None;
            }
            info.reference.clone_from(reference);
        }
        if update.referenced_message.is_some() {
            info.referenced.clone_from(&update.referenced_message);
        }
        let result = (Self::reply(&info), self.mentioned(&info));
        if known {
            self.social.messages.insert(key, info);
        }
        result
    }

    fn member_row(&mut self, guild_id: u64, item: &model::MemberItem) -> Option<MemberRow> {
        if let Some(group) = &item.group {
            let name = self
                .social
                .guilds
                .get(&guild_id)
                .map_or("Role", |guild| group_name(&guild.roles, &group.id));
            return Some(MemberRow::Group {
                name: name.to_owned(),
                count: group.count,
            });
        }
        let member = item.member.as_ref()?;
        let id = member.id()?;
        self.upsert_member(guild_id, id, member);
        Some(MemberRow::Member { user_id: id })
    }

    fn apply_member_list(&mut self, update: &model::MemberListUpdate) -> bool {
        let guild = self.social.guilds.entry(update.guild_id).or_default();
        let switching = guild.list.as_ref().is_none_or(|list| list.id != update.id);
        if switching && !update.ops.iter().any(|op| op.op == "SYNC") {
            return false;
        }
        let mut groups = if switching {
            Vec::new()
        } else {
            std::mem::take(&mut guild.list_groups)
        };
        let mut list = if switching {
            MemberList {
                id: update.id.clone(),
                ..Default::default()
            }
        } else {
            guild.list.take().unwrap()
        };
        list.member_count = update.member_count;
        list.online_count = update.online_count;
        let total = update
            .groups
            .iter()
            .filter(|g| g.count > 0)
            .fold(0usize, |total, g| {
                total.saturating_add(g.count as usize + 1)
            });
        let max_rows = list
            .rows
            .len()
            .max(total)
            .saturating_add(update.ops.iter().filter(|op| op.op == "INSERT").count());
        for op in &update.ops {
            match op.op.as_str() {
                "SYNC" | "INVALIDATE" => {
                    let Some([start, end]) = op.range else {
                        continue;
                    };
                    if start > end || start as usize >= max_rows {
                        continue;
                    }
                    let end = (end as usize).min(max_rows.saturating_sub(1));
                    list.rows.resize(list.rows.len().max(end + 1), None);
                    groups.resize(list.rows.len(), None);
                    for index in start as usize..=end {
                        groups[index] = if op.op == "SYNC" {
                            op.items
                                .get(index - start as usize)
                                .and_then(|item| item.group.as_ref())
                                .map(|g| g.id.clone())
                        } else {
                            None
                        };
                        list.rows[index] = if op.op == "SYNC" {
                            op.items
                                .get(index - start as usize)
                                .and_then(|item| self.member_row(update.guild_id, item))
                        } else {
                            None
                        };
                    }
                }
                "INSERT" | "UPDATE" => {
                    let Some(index) = op.index.map(|i| i as usize).filter(|i| *i < max_rows) else {
                        continue;
                    };
                    let row = op
                        .item
                        .as_ref()
                        .and_then(|item| self.member_row(update.guild_id, item));
                    if op.op == "INSERT" {
                        list.rows.resize(list.rows.len().max(index), None);
                        groups.resize(list.rows.len(), None);
                        list.rows.insert(index, row);
                        groups.insert(
                            index,
                            op.item
                                .as_ref()
                                .and_then(|item| item.group.as_ref())
                                .map(|g| g.id.clone()),
                        );
                    } else {
                        list.rows.resize(list.rows.len().max(index + 1), None);
                        groups.resize(list.rows.len(), None);
                        list.rows[index] = row;
                        groups[index] = op
                            .item
                            .as_ref()
                            .and_then(|item| item.group.as_ref())
                            .map(|g| g.id.clone());
                    }
                }
                "DELETE" => {
                    if let Some(index) = op
                        .index
                        .map(|i| i as usize)
                        .filter(|i| *i < list.rows.len())
                    {
                        list.rows.remove(index);
                        if index < groups.len() {
                            groups.remove(index);
                        }
                    }
                }
                _ => {}
            }
        }
        list.rows.resize(total, None);
        groups.resize(total, None);
        for (row, group) in list.rows.iter_mut().zip(&groups) {
            if let (Some(MemberRow::Group { count, .. }), Some(group)) = (row, group)
                && let Some(current) = update.groups.iter().find(|g| &g.id == group)
            {
                *count = current.count;
            }
        }
        let guild = self.social.guilds.entry(update.guild_id).or_default();
        guild.list = Some(list);
        guild.list_groups = groups;
        true
    }

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
                if let Some(id) = update.member.id() {
                    if self.upsert_member(update.guild_id, id, &update.member) {
                        self.rebuild_structure(Some(update.guild_id));
                        return Ok(Change::Structure);
                    }
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
                        if let Some(source) = &mut self.source {
                            if let Some(guild) =
                                source.guilds.iter_mut().find(|g| g.id == update.guild_id)
                            {
                                if let Some(members) = &mut guild.members {
                                    members.retain(|m| m.id() != Some(id));
                                }
                            }
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
                if let Some(source) = &mut self.source {
                    if let Some(guild) = source.guilds.iter_mut().find(|g| g.id == update.guild_id)
                    {
                        let roles = guild.roles.get_or_insert_with(Vec::new);
                        if let Some(role) = roles.iter_mut().find(|r| r.id == update.role.id) {
                            *role = update.role;
                        } else {
                            roles.push(update.role);
                        }
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
                if let Some(source) = &mut self.source {
                    if let Some(guild) = source.guilds.iter_mut().find(|g| g.id == guild_id) {
                        if let Some(roles) = &mut guild.roles {
                            roles.retain(|r| r.id != role_id);
                        }
                    }
                }
                self.rebuild_structure(Some(guild_id));
                return Ok(Change::Structure);
            }
            event => return Err(event),
        };
        Ok(Change::Members { guild_id })
    }
}
