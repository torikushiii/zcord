use super::*;

// Partial user objects omit these fields when unchanged; only an explicit null clears them.
fn apply_cosmetics(target: &mut User, user: &model::User) {
    if let Some(decoration) = &user.avatar_decoration_data {
        target.decoration = decoration.as_ref().map(map_decoration);
    }
    if let Some(collectibles) = &user.collectibles {
        target.nameplate = collectibles.as_ref().and_then(map_nameplate);
    }
    if let Some(styles) = &user.display_name_styles {
        target.name_style = styles.as_ref().map(map_name_style);
    }
}

impl Store {
    pub fn absorb_user(&mut self, user: &model::User) {
        self.register_author(user);
    }

    pub(super) fn apply_user_update(&mut self, update: &model::UserUpdate) -> bool {
        if update.id == self.current_user_id
            && let Some(account) = &mut self.account
        {
            if !update.username.is_empty() {
                account.username.clone_from(&update.username);
            }
            if let Some(email) = &update.email {
                account.email.clone_from(email);
            }
            if let Some(phone) = &update.phone {
                account.phone.clone_from(phone);
            }
            if let Some(mfa_enabled) = update.mfa_enabled {
                account.mfa_enabled = mfa_enabled;
            }
        }
        let Some(user) = self.users.get_mut(&update.id) else {
            return false;
        };
        mapping::apply_user_update(user, update);
        true
    }

    pub(super) fn register_author(&mut self, user: &model::User) {
        if user.username.is_empty() {
            if let Some(existing) = self.users.get_mut(&user.id) {
                if let Some(name) = &user.global_name {
                    existing.name.clone_from(name);
                }
                if let Some(avatar) = &user.avatar {
                    existing.avatar = Some(avatar.clone());
                }
                if let Some(discriminator) = &user.discriminator {
                    existing.discriminator = Some(discriminator.clone());
                }
                existing.bot |= user.bot;
                apply_cosmetics(existing, user);
                return;
            }
        }
        let mut mapped = map_user(user);
        if let Some(existing) = self.users.get(&user.id) {
            mapped.status = existing.status;
            mapped.activity.clone_from(&existing.activity);
            mapped.activity_kind = existing.activity_kind;
            mapped.custom_status.clone_from(&existing.custom_status);
            mapped.playing = existing.playing;
            mapped.game.clone_from(&existing.game);
            mapped.listening.clone_from(&existing.listening);
            mapped.live_stream.clone_from(&existing.live_stream);
            mapped.on_mobile = existing.on_mobile;
            mapped.role_color = existing.role_color;
            if user.primary_guild.is_none() {
                mapped.tag.clone_from(&existing.tag);
            }
            if user.avatar_decoration_data.is_none() {
                mapped.decoration.clone_from(&existing.decoration);
            }
            if user.collectibles.is_none() {
                mapped.nameplate.clone_from(&existing.nameplate);
            }
            if user.display_name_styles.is_none() {
                mapped.name_style.clone_from(&existing.name_style);
            }
        }
        self.users.insert(user.id, mapped);
    }

    pub(super) fn typing_start(&mut self, typing: Box<model::TypingStart>) -> Change {
        if let Some(member) = &typing.member {
            match typing.guild_id {
                Some(guild_id) => {
                    self.upsert_member(guild_id, typing.user_id, member);
                }
                None => {
                    if let Some(user) = &member.user {
                        self.register_author(user);
                    }
                }
            }
        }
        Change::None
    }

    pub(super) fn user_update(&mut self, update: Box<model::UserUpdate>) -> Change {
        if self.apply_user_update(&update) {
            Change::Members { guild_id: None }
        } else {
            Change::None
        }
    }
}
