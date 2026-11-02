use gpui::*;
use gpui_tokio::Tokio;

use super::DiscordApp;
use crate::{discord::settings_proto::AccountTheme, settings, theme};

impl DiscordApp {
    /// Adopts the account theme whenever another client changes it, unless sync is off.
    pub(super) fn follow_account_theme(&mut self, cx: &mut Context<Self>) {
        let account = self.store.account_theme();
        if account == self.account_theme {
            return;
        }
        self.account_theme = account;
        if settings::get(cx).theme_sync_off {
            return;
        }
        if let Some(account) = account {
            theme::set_light(account == AccountTheme::Light, cx);
        }
    }

    pub(crate) fn choose_theme(&mut self, light: bool, cx: &mut Context<Self>) {
        theme::set_light(light, cx);
        if settings::get(cx).theme_sync_off
            || self
                .store
                .account_theme()
                .is_some_and(|account| (account == AccountTheme::Light) == light)
        {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        // zcord's dark palette is Discord's darkest one.
        let wanted = if light {
            AccountTheme::Light
        } else {
            AccountTheme::Midnight
        };
        let request = Tokio::spawn(cx, async move { session.set_theme(wanted).await });
        cx.background_spawn(async move {
            if let Ok(Err(error)) = request.await {
                log::warn!("Saving the theme to the account failed: {error:?}");
            }
        })
        .detach();
    }

    pub(crate) fn set_theme_sync(&mut self, on: bool, cx: &mut Context<Self>) {
        settings::update(cx, |settings| settings.theme_sync_off = !on);
        if on {
            self.account_theme = None;
            self.follow_account_theme(cx);
        }
        cx.notify();
    }
}
