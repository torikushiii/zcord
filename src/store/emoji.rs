use super::*;

impl Store {
    pub fn emoji_access(&self, emoji: &CustomEmoji, guild_id: u64, channel_id: u64) -> EmojiAccess {
        if !emoji.available {
            return EmojiAccess::Unavailable;
        }
        if !emoji.roles.is_empty()
            && !self
                .member_roles(emoji.guild_id, self.current_user_id)
                .iter()
                .any(|role| emoji.roles.contains(&role.id))
        {
            return EmojiAccess::NeedsRole;
        }
        if emoji.animated && !self.has_nitro() {
            return EmojiAccess::NeedsNitro;
        }
        if emoji.guild_id != guild_id {
            if !self.has_nitro() {
                return EmojiAccess::NeedsNitro;
            }
            let external = self
                .permissions(channel_id)
                .is_some_and(|permissions| permissions & discord::USE_EXTERNAL_EMOJIS != 0);
            if !external {
                return EmojiAccess::NoExternalPermission;
            }
        }
        EmojiAccess::Usable
    }

    pub fn search_emoji(
        &self,
        query: &str,
        guild_id: u64,
        channel_id: u64,
        limit: usize,
    ) -> Vec<(EmojiChoice, EmojiAccess)> {
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let needle = query.to_lowercase();
        let rank = |name: &str| {
            let name = name.to_lowercase();
            if name.starts_with(&needle) {
                Some(true)
            } else if name.contains(&needle) {
                Some(false)
            } else {
                None
            }
        };
        let mut output: Vec<(EmojiChoice, EmojiAccess)> = Vec::new();
        for prefix in [true, false] {
            let Some(guild) = self.guild_by_id(guild_id) else {
                break;
            };
            for emoji in &guild.emojis {
                if rank(&emoji.name) == Some(prefix) {
                    output.push((
                        EmojiChoice::Custom(emoji.clone()),
                        self.emoji_access(emoji, guild_id, channel_id),
                    ));
                    if output.len() >= limit {
                        return output;
                    }
                }
            }
        }
        for prefix in [true, false] {
            for guild in &self.guilds {
                if guild.id == guild_id {
                    continue;
                }
                for emoji in &guild.emojis {
                    if rank(&emoji.name) == Some(prefix) {
                        output.push((
                            EmojiChoice::Custom(emoji.clone()),
                            self.emoji_access(emoji, guild_id, channel_id),
                        ));
                        if output.len() >= limit {
                            return output;
                        }
                    }
                }
            }
        }
        for prefix in [true, false] {
            for &(shortcode, emoji) in crate::text::emoji::shortcodes() {
                if rank(shortcode) == Some(prefix) {
                    output.push((
                        EmojiChoice::Unicode {
                            emoji: emoji.to_owned(),
                            shortcode: shortcode.to_owned(),
                        },
                        EmojiAccess::Usable,
                    ));
                    if output.len() >= limit {
                        return output;
                    }
                }
            }
        }
        output
    }

    pub fn emoji_guilds(&self) -> Vec<(u64, String, Option<ImageSpec>, Vec<CustomEmoji>)> {
        self.rail
            .iter()
            .flat_map(|entry| match entry {
                RailEntry::Guild(id) => std::slice::from_ref(id),
                RailEntry::Folder(folder) => &folder.guild_ids,
            })
            .filter_map(|id| {
                let guild = self.guild_by_id(*id)?;
                if guild.emojis.is_empty() {
                    return None;
                }
                let mut emojis = guild.emojis.clone();
                emojis.sort_by(|left, right| left.name.cmp(&right.name));
                Some((guild.id, guild.name.clone(), guild.icon_spec(), emojis))
            })
            .collect()
    }
}
