use super::*;

impl From<discord::SelfStatus> for Status {
    fn from(status: discord::SelfStatus) -> Self {
        match status {
            discord::SelfStatus::Online => Status::Online,
            discord::SelfStatus::Idle => Status::Idle,
            discord::SelfStatus::DoNotDisturb => Status::DoNotDisturb,
            discord::SelfStatus::Invisible => Status::Offline,
        }
    }
}

impl Store {
    /// Confirmed account status only; never replace custom status or another client's activity.
    pub fn set_self_status(&mut self, status: discord::SelfStatus) {
        if let Some(user) = self.users.get_mut(&self.current_user_id) {
            user.status = status.into();
        }
    }

    pub fn set_frecency(&mut self, proto: Vec<u8>) {
        match settings_proto::favorite_gifs(&proto) {
            Ok(gifs) => {
                self.frecency = Some(proto);
                self.favorite_gifs = gifs;
            }
            Err(_) => {
                self.frecency = None;
                self.favorite_gifs.clear();
            }
        }
    }

    pub fn frecency(&self) -> Option<&[u8]> {
        self.frecency.as_deref()
    }

    pub fn favorite_gifs(&self) -> &[settings_proto::FavoriteGif] {
        &self.favorite_gifs
    }

    pub fn account_theme(&self) -> Option<settings_proto::AccountTheme> {
        self.account_theme
    }

    pub fn is_favorite_gif(&self, url: &str) -> bool {
        self.favorite_gifs.iter().any(|gif| gif.url == url)
    }

    pub(super) fn apply_frecency(&mut self, update: &model::SettingsUpdate) -> Change {
        let previous = self.favorite_gifs.clone();
        let decoded = STANDARD.decode(&update.settings.proto);
        let proto = match (update.partial, decoded) {
            (false, Ok(proto)) => Some(proto),
            (false, Err(_)) => None,
            (true, Ok(update)) => {
                let Some(base) = self.frecency.as_deref() else {
                    return Change::None;
                };
                match settings_proto::merge_partial(base, &update) {
                    Ok(merged) => Some(merged),
                    Err(_) => return Change::None,
                }
            }
            (true, Err(_)) => return Change::None,
        };
        match proto {
            Some(proto) => self.set_frecency(proto),
            None => {
                self.frecency = None;
                self.favorite_gifs.clear();
            }
        }
        if self.favorite_gifs == previous {
            Change::None
        } else {
            Change::FavoriteGifs
        }
    }

    pub(super) fn settings_update(&mut self, update: Box<model::SettingsUpdate>) -> Change {
        // Proto type 1 is PreloadedUserSettings, type 2 is FrecencyUserSettings.
        match update.settings.kind {
            1 => {}
            2 => return self.apply_frecency(&update),
            _ => return Change::None,
        }
        match settings_proto::decode_base64(&update.settings.proto) {
            Ok(settings) => {
                let structure = settings.folders.is_some() || !update.partial;
                let status = settings.status.is_some() || !update.partial;
                let theme = if settings.theme.is_some() || !update.partial {
                    let changed = self.account_theme != settings.theme;
                    self.account_theme = settings.theme;
                    changed
                } else {
                    false
                };
                if settings.folders.is_some() || !update.partial {
                    self.folder_layout = settings.folders;
                }
                if status {
                    self.apply_own_status(settings.status.as_ref(), update.partial);
                }
                if structure {
                    self.rebuild_rail();
                    return Change::Structure;
                }
                return if status {
                    Change::Members { guild_id: None }
                } else if theme {
                    Change::Theme
                } else {
                    Change::None
                };
            }
            Err(_) => {
                log::warn!("Server folder settings unavailable; using READY order");
                self.folder_layout = None;
            }
        }
        self.rebuild_rail();
        Change::Structure
    }
}
