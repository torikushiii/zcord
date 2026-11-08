use gpui::*;
use gpui_tokio::Tokio;

use super::DiscordApp;
use crate::discord::settings_proto::{SpoilerMode, TextImages};

#[derive(Default)]
pub(crate) struct Saving {
    shown: Option<TextImages>,
    in_flight: bool,
    next: Option<(TextImages, TextImages)>,
}

impl DiscordApp {
    pub(crate) fn spoilers_open(&self) -> bool {
        match crate::settings::display().render_spoilers {
            SpoilerMode::Always => true,
            SpoilerMode::IfModerator => {
                self.selected_guild_id != 0
                    && self.store.can_manage_messages(self.selected_channel_id)
            }
            SpoilerMode::OnClick => false,
        }
    }

    pub(crate) fn spoiler_shown(&self, key: &(u64, u16)) -> bool {
        self.spoilers_open() || self.revealed_spoilers.contains(key)
    }

    pub(crate) fn text_images(&self) -> TextImages {
        self.text_images
            .shown
            .clone()
            .unwrap_or_else(|| self.store.text_images().clone())
    }

    pub(crate) fn change_text_images(
        &mut self,
        change: impl FnOnce(&mut TextImages),
        cx: &mut Context<Self>,
    ) {
        if self.session.is_none() || !self.live {
            return;
        }
        let base = self.text_images();
        let mut desired = base.clone();
        change(&mut desired);
        if desired == base {
            return;
        }
        self.text_images.shown = Some(desired.clone());
        if self.text_images.in_flight {
            let base = match self.text_images.next.take() {
                Some((queued_base, _)) => queued_base,
                None => base,
            };
            self.text_images.next = Some((base, desired));
        } else {
            self.save_text_images(base, desired, cx);
        }
        crate::settings::set_display(cx, self.text_images());
        cx.notify();
    }

    fn save_text_images(&mut self, base: TextImages, edited: TextImages, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        self.text_images.in_flight = true;
        let saving_session = session.clone();
        let request = Tokio::spawn(cx, async move {
            saving_session.set_text_images(base, edited).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let failed = match result {
                    Ok(Ok(())) => {
                        app.store.set_text_images(session.text_images());
                        false
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not save Text & Images: {error}");
                        true
                    }
                    Err(error) => {
                        log::warn!("Text & Images task failed: {error}");
                        true
                    }
                };
                let saving = &mut app.text_images;
                saving.in_flight = false;
                match saving.next.take().filter(|_| !failed) {
                    Some((base, edited)) => app.save_text_images(base, edited, cx),
                    None => saving.shown = None,
                }
                crate::settings::set_display(cx, app.text_images());
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
