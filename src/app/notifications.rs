use super::*;
use crate::discord::model;
use crate::media::images;

const MAX_ALERT_AGE_MS: u64 = 60_000;
const AVATAR_SIZE: u32 = 96;
const AVATAR_WAIT: Duration = Duration::from_secs(2);

impl DiscordApp {
    pub(super) fn notify_message(&self, message: &model::Message, cx: &mut Context<Self>) {
        if !self.live || !crate::settings::notifications() {
            return;
        }
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let sent_ms = (message.id >> 22) + DISCORD_EPOCH_MS;
        if now_ms.saturating_sub(sent_ms) > MAX_ALERT_AGE_MS {
            return;
        }
        let Some(alert) = self.store.message_alert(message, now_ms) else {
            return;
        };
        let on_screen = alert.channel_id == self.selected_channel_id
            || self.thread_pane_shows(alert.channel_id);
        if self.window_active && on_screen {
            return;
        }
        let streamer = crate::settings::streamer();
        if crate::settings::notification_sound()
            && !self.voice.self_deaf
            && !streamer.mutes_sounds()
        {
            let volume = crate::settings::get(cx).voice_output_volume.unwrap_or(1.);
            crate::notify::play_message_sound(volume);
        }
        if !self.window_active && !streamer.mutes_notifications() {
            let service = images::download_service(cx);
            let spec = alert.avatar.clone();
            let avatar = Tokio::spawn(cx, async move {
                let service = service?;
                tokio::time::timeout(AVATAR_WAIT, service.load(&spec, AVATAR_SIZE))
                    .await
                    .ok()?
            });
            let activation = crate::notify::Activation {
                user_id: self.store.current_user_id,
                channel_id: alert.channel_id,
                message_id: alert.message_id,
            };
            let retired = self.accounts.retired();
            cx.spawn(async move |this, cx| {
                let icon = avatar.await.ok().flatten();
                let icon = match icon {
                    Some(path) => {
                        cx.background_executor()
                            .spawn(async move { crate::notify::toast_icon(&path) })
                            .await
                    }
                    None => None,
                };
                // The avatar wait can outlast an account switch; the old account's toast must not appear.
                let current = this
                    .read_with(cx, |app, _| {
                        app.live && app.store.current_user_id == activation.user_id
                    })
                    .unwrap_or(false);
                if current {
                    cx.background_executor()
                        .spawn(async move {
                            if !retired.get() {
                                crate::notify::show(
                                    &alert.title,
                                    &alert.body,
                                    icon.as_deref(),
                                    activation,
                                );
                            }
                        })
                        .detach();
                }
            })
            .detach();
        }
    }

    pub(super) fn open_notification(
        &mut self,
        activation: crate::notify::Activation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(windows)]
        crate::tray::show(window);
        #[cfg(not(windows))]
        window.activate_window();
        if activation.user_id != self.store.current_user_id {
            return;
        }
        self.open_message(
            None,
            activation.channel_id,
            activation.message_id,
            window,
            cx,
        );
    }
}
