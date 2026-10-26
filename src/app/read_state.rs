use super::*;

impl DiscordApp {
    pub(crate) fn mark_channel_read(&mut self, cx: &mut Context<Self>) {
        self.unread_since = None;
        self.unread_bar = false;
        self.ack_channel(cx);
        cx.notify();
    }

    pub(super) fn schedule_ack(&mut self, cx: &mut Context<Self>) {
        if self.ack_timer.is_some()
            || !self.live
            || self.store.channel_ack(self.selected_channel_id).is_none()
        {
            return;
        }
        self.ack_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ACK_DELAY).await;
            this.update(cx, |app, cx| {
                app.ack_timer = None;
                if app.viewing() {
                    app.ack_channel(cx);
                }
            })
            .ok();
        }));
    }

    pub(super) fn viewing(&self) -> bool {
        self.live
            && self.window_active
            && matches!(self.screen, Screen::Chat)
            && self.messages.is_following_tail()
            && self
                .channel()
                .is_some_and(|channel| channel.messages_loaded)
            && self.store.at_present(self.selected_channel_id)
    }

    pub(super) fn ack_channel(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let channel_id = self.selected_channel_id;
        if !self.store.at_present(channel_id) {
            return;
        }
        let Some(message_id) = self.store.channel_ack(channel_id) else {
            return;
        };
        self.store.apply(Event::MessageAck {
            channel_id,
            message_id,
        });
        cx.notify();
        let guild_id = self.request_guild();
        let days = days_since_discord_epoch();
        let request = Tokio::spawn(cx, async move {
            session
                .ack_message(guild_id, channel_id, message_id, days)
                .await
        });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not mark channel read: {error}"),
            Err(error) => log::warn!("Mark read task failed: {error}"),
        })
        .detach();
    }

    pub(super) fn mark_guild_read(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if !matches!(self.screen, Screen::Chat) {
            return;
        }
        let guild_id = self.selected_guild_id;
        self.unread_since = None;
        self.unread_bar = false;
        cx.notify();
        let acks = self.store.unread_acks(guild_id, now_snowflake());
        if acks.is_empty() {
            return;
        }
        for &(channel_id, message_id) in &acks {
            self.store.apply(Event::MessageAck {
                channel_id,
                message_id,
            });
        }
        let channel_id = self.selected_channel_id;
        let request = Tokio::spawn(cx, async move {
            session.ack_bulk(Some(guild_id), channel_id, acks).await
        });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not mark server read: {error}"),
            Err(error) => log::warn!("Mark read task failed: {error}"),
        })
        .detach();
    }
}
