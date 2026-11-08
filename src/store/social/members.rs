use super::*;

impl Store {
    pub fn absorb_profile(&mut self, guild_id: Option<u64>, profile: &model::Profile) {
        if let (Some(guild_id), Some(member)) = (guild_id, profile.guild_member.as_ref()) {
            self.upsert_member(guild_id, profile.user.user.id, &member.member);
        }
        self.absorb_user(&profile.user.user);
        if let Some(friends) = &profile.mutual_friends {
            for friend in friends {
                self.absorb_user(friend);
            }
        }
    }
    pub fn member_decoration(&self, guild_id: u64, user_id: u64) -> Option<&Decoration> {
        self.social
            .guilds
            .get(&guild_id)
            .and_then(|guild| guild.members.get(&user_id))
            .and_then(|member| member.decoration.as_ref())
            .or_else(|| self.user(user_id)?.decoration.as_ref())
    }
    pub fn member_name_style(&self, guild_id: u64, user_id: u64) -> Option<&NameStyle> {
        self.social
            .guilds
            .get(&guild_id)
            .and_then(|guild| guild.members.get(&user_id))
            .and_then(|member| member.name_style.as_ref())
            .or_else(|| self.user(user_id)?.name_style.as_ref())
    }
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
    pub fn member_name_color(&self, guild_id: u64, user_id: u64) -> Option<u32> {
        self.member_color(guild_id, user_id)
            .filter(|_| crate::settings::role_colors() == crate::settings::RoleColors::InNames)
    }
    pub fn member_role_dot(&self, guild_id: u64, user_id: u64) -> Option<u32> {
        self.member_color(guild_id, user_id)
            .filter(|_| crate::settings::role_colors() == crate::settings::RoleColors::NextToNames)
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
    pub fn member_roles(&self, guild_id: u64, user_id: u64) -> Vec<&Role> {
        let Some(guild) = self.social.guilds.get(&guild_id) else {
            return Vec::new();
        };
        let mut roles: Vec<&Role> = guild
            .members
            .get(&user_id)
            .into_iter()
            .flat_map(|member| member.roles.iter())
            .filter_map(|id| guild.roles.get(id))
            .collect();
        roles.sort_by_key(|role| std::cmp::Reverse(role.position));
        roles
    }
    fn search_candidates(&self, guild_id: Option<u64>, channel_id: u64) -> Vec<u64> {
        match guild_id {
            Some(guild_id) => self
                .social
                .guilds
                .get(&guild_id)
                .map(|guild| guild.members.keys().copied().collect())
                .unwrap_or_default(),
            None => self
                .private_channel(channel_id)
                .map(|dm| dm.recipient_ids.clone())
                .unwrap_or_default()
                .into_iter()
                .chain([self.current_user_id])
                .collect(),
        }
    }
    fn user_names(&self, guild_id: Option<u64>, user_id: u64) -> Option<[String; 3]> {
        let user = self.user(user_id)?;
        let nick = guild_id
            .and_then(|guild_id| self.member_name(guild_id, user_id))
            .unwrap_or(&user.real_name);
        Some([&user.username, &user.real_name, &nick.to_owned()].map(|name| name.to_lowercase()))
    }
    pub fn search_users(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        query: &str,
        limit: usize,
    ) -> Vec<u64> {
        let mut matches: Vec<_> = self
            .search_candidates(guild_id, channel_id)
            .into_iter()
            .filter_map(|id| {
                let names = self.user_names(guild_id, id)?;
                let rank = if names.iter().any(|name| name.starts_with(query)) {
                    0
                } else if names.iter().any(|name| name.contains(query)) {
                    1
                } else {
                    return None;
                };
                Some((rank, names[1].clone(), id))
            })
            .collect();
        matches.sort();
        matches.truncate(limit);
        matches.into_iter().map(|(_, _, id)| id).collect()
    }
    pub fn find_search_user(
        &self,
        guild_id: Option<u64>,
        channel_id: u64,
        name: &str,
    ) -> Option<u64> {
        let name = name.to_lowercase();
        self.search_candidates(guild_id, channel_id)
            .into_iter()
            .find(|id| {
                self.user_names(guild_id, *id)
                    .is_some_and(|names| names.contains(&name))
            })
    }
    pub fn nameplate(&self, guild_id: Option<u64>, user_id: u64) -> Option<&Nameplate> {
        guild_id
            .and_then(|guild_id| self.social.guilds.get(&guild_id)?.members.get(&user_id))
            .and_then(|member| member.nameplate.as_ref())
            .or_else(|| self.users.get(&user_id)?.nameplate.as_ref())
    }
}
