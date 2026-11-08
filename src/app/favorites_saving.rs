use gpui::*;
use gpui_tokio::Tokio;

use super::DiscordApp;
use crate::discord::settings_proto::FavoriteChannel;

const FAVORITE_CHANNEL: u32 = 1;

#[derive(Default)]
pub(crate) struct Saving {
    shown: Option<bool>,
    in_flight: bool,
    next: Option<bool>,
}

impl DiscordApp {
    pub(crate) fn favorites_visible(&self) -> bool {
        self.favorites_saving
            .shown
            .unwrap_or(self.store.favorites().guild_visible)
    }

    pub(crate) fn change_favorites_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.session.is_none() || !self.live || self.favorites_visible() == visible {
            return;
        }
        self.favorites_saving.shown = Some(visible);
        if self.favorites_saving.in_flight {
            self.favorites_saving.next = Some(visible);
        } else {
            self.save_favorites_visible(visible, cx);
        }
        cx.notify();
    }

    fn save_favorites_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        self.favorites_saving.in_flight = true;
        let saving_session = session.clone();
        let request = Tokio::spawn(cx, async move {
            saving_session.set_favorites_visible(visible).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let failed = match result {
                    Ok(Ok(())) => {
                        app.store.set_favorites(session.favorites());
                        false
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not save Favorites visibility: {error}");
                        true
                    }
                    Err(error) => {
                        log::warn!("Favorites visibility task failed: {error}");
                        true
                    }
                };
                let saving = &mut app.favorites_saving;
                saving.in_flight = false;
                match saving.next.take().filter(|_| !failed) {
                    Some(visible) => app.save_favorites_visible(visible, cx),
                    None => saving.shown = None,
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl DiscordApp {
    pub(crate) fn is_favorite_channel(&self, channel_id: u64) -> bool {
        self.store
            .favorites()
            .channels
            .iter()
            .any(|channel| channel.id == channel_id && channel.kind == FAVORITE_CHANNEL)
    }

    pub(crate) fn open_favorites(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let first = self
            .favorite_channels()
            .into_iter()
            .find(|&id| self.store.channel_guild_id(id).is_some());
        match first {
            Some(channel_id) => self.open_favorite_channel(channel_id, window, cx),
            None => {
                self.favorites_view = true;
                cx.notify();
            }
        }
    }

    pub(crate) fn favorite_channels(&self) -> Vec<u64> {
        let mut channels: Vec<_> = self
            .store
            .favorites()
            .channels
            .iter()
            .filter(|channel| channel.kind == FAVORITE_CHANNEL)
            .collect();
        channels.sort_by_key(|channel| channel.position);
        channels.into_iter().map(|channel| channel.id).collect()
    }

    pub(crate) fn open_favorite_channel(
        &mut self,
        channel_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(guild_id) = self.store.channel_guild_id(channel_id) else {
            return;
        };
        if self.selected_guild_id == guild_id {
            self.select_channel(channel_id, window, cx);
        } else {
            self.select_guild_channel(guild_id, Some(channel_id), window, cx);
        }
        self.favorites_view = true;
        cx.notify();
    }

    pub(crate) fn toggle_favorite_channel(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let favorite = (!self.is_favorite_channel(channel_id)).then(|| {
            let position = self
                .store
                .favorites()
                .channels
                .iter()
                .map(|channel| channel.position + 1)
                .max()
                .unwrap_or(0);
            FavoriteChannel {
                id: channel_id,
                nickname: String::new(),
                kind: FAVORITE_CHANNEL,
                position,
                parent_id: 0,
                channel_type: None,
                collapsed: false,
            }
        });
        let request = Tokio::spawn(cx, async move {
            session.set_favorite_channel(channel_id, favorite).await
        });
        cx.background_spawn(async move {
            if let Ok(Err(error)) = request.await {
                log::warn!("Could not change the favorite channel: {error}");
            }
        })
        .detach();
    }
}
