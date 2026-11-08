use gpui::*;
use gpui_tokio::Tokio;

use super::super::DiscordApp;
use crate::discord::settings_proto::ActivityPrivacy;

#[derive(Default)]
pub(crate) struct Saving {
    shown: Option<ActivityPrivacy>,
    in_flight: bool,
    next: Option<(ActivityPrivacy, ActivityPrivacy)>,
}

impl DiscordApp {
    pub(crate) fn activity_privacy(&self) -> ActivityPrivacy {
        self.activity_privacy.shown.clone().unwrap_or_else(|| {
            self.session
                .as_ref()
                .map(|session| session.activity_privacy())
                .unwrap_or_default()
        })
    }

    pub(crate) fn change_activity_privacy(
        &mut self,
        change: impl FnOnce(&mut ActivityPrivacy),
        cx: &mut Context<Self>,
    ) {
        if self.session.is_none() || !self.live {
            return;
        }
        let base = self.activity_privacy();
        let mut desired = base.clone();
        change(&mut desired);
        self.activity_privacy.shown = Some(desired.clone());
        if self.activity_privacy.in_flight {
            let base = match self.activity_privacy.next.take() {
                Some((queued_base, _)) => queued_base,
                None => base,
            };
            self.activity_privacy.next = Some((base, desired));
        } else {
            self.save_activity_privacy(base, desired, cx);
        }
        self.views.channel_list.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub(crate) fn toggle_activity_sharing(&mut self, cx: &mut Context<Self>) {
        self.change_activity_privacy(|privacy| privacy.share_activity ^= true, cx);
    }

    fn save_activity_privacy(
        &mut self,
        base: ActivityPrivacy,
        edited: ActivityPrivacy,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone() else {
            return;
        };
        self.activity_privacy.in_flight = true;
        let request = Tokio::spawn(cx, async move {
            session.set_activity_privacy(base, edited).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let failed = match result {
                    Ok(Ok(())) => false,
                    Ok(Err(error)) => {
                        log::warn!("Could not save Activity Privacy: {error}");
                        true
                    }
                    Err(error) => {
                        log::warn!("Activity Privacy task failed: {error}");
                        true
                    }
                };
                let saving = &mut app.activity_privacy;
                saving.in_flight = false;
                match saving.next.take().filter(|_| !failed) {
                    Some((base, edited)) => app.save_activity_privacy(base, edited, cx),
                    None => saving.shown = None,
                }
                app.update_game_activity();
                app.views.channel_list.update(cx, |_, cx| cx.notify());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
