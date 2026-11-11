use super::*;

fn viewing_allows_channel(manual_unread: Option<u64>, selected_channel_id: u64) -> bool {
    manual_unread != Some(selected_channel_id)
}

impl DiscordApp {
    pub(crate) fn mark_channel_read(&mut self, cx: &mut Context<Self>) {
        self.unread.since = None;
        self.unread.bar = false;
        self.unread.manual = None;
        self.ack_channel_now(self.selected_channel_id, true, cx);
        cx.notify();
    }

    pub(crate) fn mark_unread(&mut self, message_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let channel_id = self.selected_channel_id;
        let Some((cursor, mention_count)) = self.store.mark_unread(channel_id, message_id) else {
            return;
        };
        self.unread.manual = Some(channel_id);
        self.unread.since = Some(cursor);
        self.unread.bar = true;
        self.unread.ack_timer = None;
        self.messages.remeasure_items(0..self.messages.item_count());
        self.views.messages.update(cx, |_, cx| cx.notify());
        cx.notify();
        let guild_id = self.request_guild();
        let request = Tokio::spawn(cx, async move {
            session
                .ack_manual(guild_id, channel_id, cursor, mention_count)
                .await
        });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not mark channel unread: {error}"),
            Err(error) => log::warn!("Mark unread task failed: {error}"),
        })
        .detach();
    }

    pub(super) fn schedule_ack(&mut self, cx: &mut Context<Self>) {
        if self.unread.ack_timer.is_some()
            || !self.live
            || self.store.channel_ack(self.selected_channel_id).is_none()
        {
            return;
        }
        self.unread.ack_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ACK_DELAY).await;
            this.update(cx, |app, cx| {
                app.unread.ack_timer = None;
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
            && viewing_allows_channel(self.unread.manual, self.selected_channel_id)
            && self
                .channel()
                .is_some_and(|channel| channel.messages_loaded)
            && self.store.at_present(self.selected_channel_id)
    }

    pub(super) fn ack_channel(&mut self, cx: &mut Context<Self>) {
        self.ack_channel_in(self.selected_channel_id, cx);
    }

    pub(crate) fn ack_channel_in(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        self.ack_channel_now(channel_id, false, cx);
    }

    fn ack_channel_now(&mut self, channel_id: u64, immediate: bool, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if !immediate && !self.store.at_present(channel_id) {
            return;
        }
        let Some(message_id) = self.store.channel_ack(channel_id) else {
            return;
        };
        let delay =
            (!immediate && self.store.mention_count(channel_id) == 0).then_some(ACK_SEND_DELAY);
        self.store.apply(Event::MessageAck {
            channel_id,
            message_id,
        });
        cx.notify();
        let guild_id = self.request_guild();
        let task = cx.spawn(async move |this, cx| {
            if let Some(delay) = delay {
                cx.background_executor().timer(delay).await;
            }
            let days = days_since_discord_epoch();
            let request = Tokio::spawn(cx, async move {
                session
                    .ack_message(guild_id, channel_id, message_id, days)
                    .await
            });
            match request.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::warn!("Could not mark channel read: {error}"),
                Err(error) => log::warn!("Mark read task failed: {error}"),
            }
            this.update(cx, |app, _| app.unread.outgoing_acks.remove(&channel_id))
                .ok();
        });
        self.unread.outgoing_acks.insert(channel_id, task);
    }

    pub(crate) fn mark_dm_read(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(message_id) = self.store.channel_ack(channel_id) else {
            return;
        };
        if channel_id == self.selected_channel_id {
            self.unread.since = None;
            self.unread.bar = false;
            self.unread.manual = None;
        }
        self.store.apply(Event::MessageAck {
            channel_id,
            message_id,
        });
        cx.notify();
        let days = days_since_discord_epoch();
        let request = Tokio::spawn(cx, async move {
            session
                .ack_message(None, channel_id, message_id, days)
                .await
        });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not mark DM read: {error}"),
            Err(error) => log::warn!("Mark read task failed: {error}"),
        })
        .detach();
    }

    pub(crate) fn mark_guild_read(&mut self, guild_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if guild_id == self.selected_guild_id {
            self.unread.since = None;
            self.unread.bar = false;
            self.unread.manual = None;
        }
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
        let channel_id = if guild_id == self.selected_guild_id {
            self.selected_channel_id
        } else {
            acks[0].0
        };
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

#[cfg(test)]
mod tests {
    use super::viewing_allows_channel;

    #[test]
    fn manual_unread_holds_viewing_for_its_channel() {
        assert!(!viewing_allows_channel(Some(7), 7));
        assert!(viewing_allows_channel(None, 7));
        assert!(viewing_allows_channel(Some(8), 7));
    }
}
