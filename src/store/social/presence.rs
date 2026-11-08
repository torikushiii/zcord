use super::*;

impl Store {
    pub(crate) fn upsert_member(
        &mut self,
        guild_id: u64,
        id: u64,
        member: &model::PartialMember,
    ) -> bool {
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
        if let Some(collectibles) = &member.collectibles {
            info.nameplate = collectibles.as_ref().and_then(map_nameplate);
        }
        if let Some(decoration) = &member.avatar_decoration_data {
            info.decoration = decoration.as_ref().map(map_decoration);
        }
        if let Some(styles) = &member.display_name_styles {
            info.name_style = styles.as_ref().map(map_name_style);
        }
        info.color = member_color(&guild.roles, info);
        if new && let Some(index) = self.guild_indices.get(&guild_id) {
            self.guilds[*index].member_ids.push(id);
        }
        if id == self.current_user_id
            && let Some(source) = &mut self.source
            && let Some(guild) = source.guilds.iter_mut().find(|g| g.id == guild_id)
        {
            let members = guild.members.get_or_insert_with(Vec::new);
            if let Some(own) = members.iter_mut().find(|m| m.id() == Some(id)) {
                own.nick.clone_from(&info.nick);
                own.roles.clone_from(&info.roles);
                own.avatar.clone_from(&info.avatar);
                if member.avatar_decoration_data.is_some() {
                    own.avatar_decoration_data
                        .clone_from(&member.avatar_decoration_data);
                }
                if member.display_name_styles.is_some() {
                    own.display_name_styles
                        .clone_from(&member.display_name_styles);
                }
            } else {
                members.push(model::Member {
                    user_id: Some(id),
                    user: None,
                    nick: info.nick.clone(),
                    roles: info.roles.clone(),
                    avatar: info.avatar.clone(),
                    avatar_decoration_data: member.avatar_decoration_data.clone(),
                    display_name_styles: member.display_name_styles.clone(),
                    collectibles: None,
                });
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
    pub(super) fn apply_presence(&mut self, presence: &model::Presence, fallback: Option<u64>) {
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
        let own_custom = if id == self.current_user_id {
            self.social.own_custom_status.clone()
        } else {
            None
        };
        let user = self.users.entry(id).or_insert_with(|| {
            map_user(&model::User {
                id,
                ..Default::default()
            })
        });
        if let Some(value) = &presence.status {
            user.status = status(value);
        }
        if let Some(client_status) = &presence.client_status {
            user.on_mobile = client_status.mobile_only();
        }
        if let Some(activities) = &presence.activities {
            let current = activity(activities);
            let playing = current.as_ref().is_some_and(|(_, playing, _)| *playing);
            user.playing = playing;
            user.activity_kind = current.as_ref().and_then(|(_, _, kind)| *kind);
            user.activity = current.map(|(text, _, _)| text);
            user.game = playing_activity(activities)
                .filter(|activity| activity.kind == 0)
                .map(game_activity);
            user.listening = activities
                .iter()
                .find(|activity| activity.kind == 2)
                .map(game_activity);
            user.live_stream = activities
                .iter()
                .find(|activity| activity.kind == 1)
                .map(live_stream);
            if let Some(mut custom) = custom_status_from_presence(activities) {
                let same_as_settings = own_custom
                    .as_ref()
                    .is_some_and(|configured| same_custom_status(&custom, configured));
                if same_as_settings {
                    custom.expires_at = own_custom.as_ref().and_then(|status| status.expires_at);
                    self.social.own_custom_status = Some(custom.clone());
                } else if own_custom.is_some() {
                    self.social.own_custom_status = None;
                }
                user.custom_status = Some(custom);
            } else if let Some(configured) = own_custom {
                if !configured.is_expired(SystemTime::now()) {
                    show_custom_status(user, &configured);
                } else {
                    user.custom_status = None;
                }
            } else {
                user.custom_status = None;
            }
        }
    }
    pub(crate) fn apply_sessions(&mut self, sessions: &[model::Session]) {
        let Some(session) = sessions
            .iter()
            .find(|session| session.session_id == "all")
            .or_else(|| sessions.iter().find(|session| session.active))
            .or_else(|| sessions.first())
        else {
            return;
        };
        let presence = model::Presence {
            user: None,
            user_id: Some(self.current_user_id),
            guild_id: None,
            status: session.status.clone(),
            activities: Some(session.activities.clone()),
            client_status: None,
        };
        self.apply_presence(&presence, None);
    }
    pub(crate) fn apply_own_status(
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
            let custom = settings
                .and_then(|s| s.custom_status.as_ref())
                .map(custom_status_from_settings);
            match &custom {
                Some(custom) => show_custom_status(user, custom),
                None => {
                    user.playing = false;
                    user.activity_kind = None;
                    user.activity = None;
                    user.custom_status = None;
                }
            }
            self.social.own_custom_status = custom;
        }
    }
}
