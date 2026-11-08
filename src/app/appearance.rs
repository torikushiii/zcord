use gpui::*;
use gpui_tokio::Tokio;

use super::DiscordApp;
use crate::{
    discord::settings_proto::{AccountTheme, ClientTheme, CustomTheme},
    settings::{self, SavedClientTheme},
    theme::{self, BaseTheme, Gradient, gradient},
};

type AccountAppearance = (Option<AccountTheme>, Option<ClientTheme>);

#[derive(Default)]
struct SeenAccount(Option<AccountAppearance>);

impl Global for SeenAccount {}

/// The store keeps the old account theme until the gateway echoes our save, so
/// unrelated store changes in between must not re-apply it.
#[derive(Default)]
struct PendingSave {
    target: Option<AccountAppearance>,
    generation: u64,
}

impl Global for PendingSave {}

const PENDING_ECHO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

struct DeviceLight(bool);

impl Global for DeviceLight {}

fn base_for(account: AccountTheme) -> BaseTheme {
    match account {
        AccountTheme::Light => BaseTheme::Light,
        AccountTheme::Dark => BaseTheme::Ash,
        AccountTheme::Darker => BaseTheme::Dark,
        AccountTheme::Midnight => BaseTheme::Onyx,
    }
}

fn account_for(base: BaseTheme) -> AccountTheme {
    match base {
        BaseTheme::Light => AccountTheme::Light,
        BaseTheme::Ash => AccountTheme::Dark,
        BaseTheme::Dark => AccountTheme::Darker,
        BaseTheme::Onyx => AccountTheme::Midnight,
    }
}

fn parse_hex(color: &str) -> Option<u32> {
    let digits = color.strip_prefix('#').unwrap_or(color);
    (digits.len() == 6)
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()
}

fn saved_from_account(theme: &ClientTheme) -> Option<SavedClientTheme> {
    match theme {
        ClientTheme::Preset(id) => Some(SavedClientTheme::Preset(*id)),
        ClientTheme::Custom(custom) => {
            let colors: Vec<u32> = custom.colors.iter().filter_map(|c| parse_hex(c)).collect();
            (!colors.is_empty()).then_some(SavedClientTheme::Custom {
                colors,
                angle: custom.angle,
                base_mix: custom.base_mix,
            })
        }
    }
}

fn account_from_saved(theme: &SavedClientTheme) -> ClientTheme {
    match theme {
        SavedClientTheme::Preset(id) => ClientTheme::Preset(*id),
        SavedClientTheme::Custom {
            colors,
            angle,
            base_mix,
        } => ClientTheme::Custom(CustomTheme {
            colors: colors.iter().map(|color| format!("#{color:06x}")).collect(),
            stops: Vec::new(),
            angle: *angle,
            base_mix: *base_mix,
        }),
    }
}

pub(crate) fn chosen_base(saved: &settings::Settings) -> BaseTheme {
    saved.base_theme.unwrap_or(if saved.light_theme {
        BaseTheme::Light
    } else {
        BaseTheme::Onyx
    })
}

fn device_light(cx: &App) -> bool {
    cx.try_global::<DeviceLight>().map_or_else(
        || {
            matches!(
                cx.window_appearance(),
                WindowAppearance::Light | WindowAppearance::VibrantLight
            )
        },
        |device| device.0,
    )
}

pub(crate) fn resolve(saved: &settings::Settings, cx: &App) -> (BaseTheme, Option<Gradient>) {
    let mut base = chosen_base(saved);
    if saved.theme_follows_device {
        base = match (device_light(cx), base) {
            (true, _) => BaseTheme::Light,
            (false, BaseTheme::Light) => BaseTheme::Onyx,
            (false, dark) => dark,
        };
    }
    match &saved.client_theme {
        Some(SavedClientTheme::Preset(id)) => match gradient::preset(*id) {
            Some(preset) => (
                if preset.light {
                    BaseTheme::Light
                } else {
                    BaseTheme::Dark
                },
                Some(Gradient::from_preset(preset)),
            ),
            None => (base, None),
        },
        Some(SavedClientTheme::Custom {
            colors,
            angle,
            base_mix,
        }) => (
            base,
            Gradient::custom(
                colors,
                *angle as f32,
                *base_mix as f32,
                base == BaseTheme::Light,
            ),
        ),
        None => (base, None),
    }
}

pub(crate) fn device_appearance_changed(window: &Window, cx: &mut App) {
    let light = matches!(
        window.appearance(),
        WindowAppearance::Light | WindowAppearance::VibrantLight
    );
    cx.set_global(DeviceLight(light));
    if settings::get(cx).theme_follows_device {
        apply_saved_theme(cx);
    }
}

pub(crate) fn apply_saved_theme(cx: &mut App) {
    let (base, gradient) = resolve(&settings::get(cx), cx);
    theme::set_theme(base, gradient, cx);
}

impl DiscordApp {
    pub(super) fn follow_account_theme(&mut self, cx: &mut Context<Self>) {
        let account = (
            self.store.account_theme(),
            self.store.client_theme().cloned(),
        );
        let pending = cx.default_global::<PendingSave>();
        if let Some(target) = &pending.target {
            if *target != account {
                return;
            }
            pending.target = None;
        }
        let seen = cx.default_global::<SeenAccount>();
        if seen.0.as_ref() == Some(&account) {
            return;
        }
        seen.0 = Some(account.clone());
        self.account_theme = account.0;
        if settings::get(cx).theme_sync_off {
            return;
        }
        let Some(theme) = account.0 else {
            return;
        };
        settings::update(cx, |settings| {
            settings.base_theme = Some(base_for(theme));
            settings.light_theme = theme == AccountTheme::Light;
            settings.client_theme = account.1.as_ref().and_then(saved_from_account);
        });
        apply_saved_theme(cx);
    }

    pub(crate) fn can_sync_client_theme(&self) -> bool {
        matches!(self.store.premium_type(), Some(1 | 2))
    }

    fn save_appearance(&mut self, cx: &mut Context<Self>) {
        let saved = settings::get(cx);
        apply_saved_theme(cx);
        if saved.theme_sync_off {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        let synced_client = saved
            .client_theme
            .as_ref()
            .filter(|_| self.can_sync_client_theme())
            .map(account_from_saved);
        if saved.client_theme.is_some() && synced_client.is_none() {
            return;
        }
        let (base, _) = resolve(
            &settings::Settings {
                theme_follows_device: false,
                ..saved
            },
            cx,
        );
        let theme = account_for(base);
        let target = (Some(theme), synced_client.clone());
        cx.default_global::<SeenAccount>().0 = Some(target.clone());
        let pending = cx.default_global::<PendingSave>();
        pending.generation += 1;
        pending.target = Some(target);
        let generation = pending.generation;
        let request = Tokio::spawn(cx, async move {
            session.set_appearance(theme, synced_client).await
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Err(error)) = request.await {
                log::warn!("Saving the theme to the account failed: {error:?}");
            } else {
                cx.background_executor().timer(PENDING_ECHO_TIMEOUT).await;
            }
            this.update(cx, |app, cx| {
                let pending = cx.default_global::<PendingSave>();
                if pending.generation != generation || pending.target.is_none() {
                    return;
                }
                pending.target = None;
                app.follow_account_theme(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn choose_theme(&mut self, base: BaseTheme, cx: &mut Context<Self>) {
        settings::update(cx, |settings| {
            settings.base_theme = Some(base);
            settings.light_theme = base == BaseTheme::Light;
            settings.client_theme = None;
            settings.theme_follows_device = false;
        });
        self.save_appearance(cx);
    }

    pub(crate) fn choose_client_theme(&mut self, theme: SavedClientTheme, cx: &mut Context<Self>) {
        settings::update(cx, |settings| {
            if let SavedClientTheme::Preset(id) = theme
                && let Some(preset) = gradient::preset(id)
            {
                let base = if preset.light {
                    BaseTheme::Light
                } else {
                    BaseTheme::Dark
                };
                settings.base_theme = Some(base);
                settings.light_theme = preset.light;
            }
            settings.client_theme = Some(theme);
            settings.theme_follows_device = false;
        });
        self.save_appearance(cx);
    }

    pub(crate) fn set_theme_follows_device(&mut self, on: bool, cx: &mut Context<Self>) {
        settings::update(cx, |settings| {
            settings.theme_follows_device = on;
            if on {
                settings.client_theme = None;
            }
        });
        apply_saved_theme(cx);
        cx.notify();
    }

    pub(crate) fn set_theme_sync(&mut self, on: bool, cx: &mut Context<Self>) {
        settings::update(cx, |settings| settings.theme_sync_off = !on);
        if on {
            cx.default_global::<SeenAccount>().0 = None;
            cx.default_global::<PendingSave>().target = None;
            self.follow_account_theme(cx);
        }
        cx.notify();
    }
}
