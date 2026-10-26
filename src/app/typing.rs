use super::*;

impl DiscordApp {
    pub(super) fn send_typing(&mut self, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let now = Instant::now();
        if self.composer.read(cx).value().trim().is_empty()
            || !self.store.can_send(channel_id)
            || self.typing_sent.is_some_and(|(channel, sent)| {
                channel == channel_id && now.duration_since(sent) < TYPING_RESEND
            })
        {
            return;
        }
        self.typing_sent = Some((channel_id, now));
        let guild_id = self.request_guild();
        let request = Tokio::spawn(cx, async move {
            session.trigger_typing(guild_id, channel_id).await
        });
        cx.spawn(async move |this, cx| match request.await {
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => {
                this.update(cx, |app, _| {
                    if app.typing_sent == Some((channel_id, now)) {
                        app.typing_sent = None;
                    }
                })
                .ok();
            }
            Ok(Err(error)) => {
                log::debug!("Could not send typing indicator: {error}");
            }
            Err(_) => {}
        })
        .detach();
    }

    pub(super) fn typing_started(&mut self, channel_id: u64, user_id: u64, cx: &mut Context<Self>) {
        if user_id == self.store.current_user_id {
            return;
        }
        let users = self.typing.entry(channel_id).or_default();
        users.retain(|(id, _)| *id != user_id);
        users.push((user_id, Instant::now() + TYPING_DURATION));
        self.arm_typing_timer(cx);
        if channel_id == self.selected_channel_id {
            cx.notify();
        }
    }

    pub(super) fn typing_stopped(&mut self, channel_id: u64, user_id: u64, cx: &mut Context<Self>) {
        if let Some(users) = self.typing.get_mut(&channel_id) {
            users.retain(|(id, _)| *id != user_id);
            if channel_id == self.selected_channel_id {
                cx.notify();
            }
        }
    }

    fn arm_typing_timer(&mut self, cx: &mut Context<Self>) {
        let Some(next) = self
            .typing
            .values()
            .flatten()
            .map(|(_, until)| *until)
            .min()
        else {
            self.typing_timer = None;
            return;
        };
        self.typing_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(next.saturating_duration_since(Instant::now()))
                .await;
            this.update(cx, |app, cx| {
                let now = Instant::now();
                let selected = app.selected_channel_id;
                let before = app.typing.get(&selected).cloned();
                for users in app.typing.values_mut() {
                    users.retain(|(_, until)| *until > now);
                }
                app.typing.retain(|_, users| !users.is_empty());
                app.arm_typing_timer(cx);
                if app.typing.get(&selected) != before.as_ref() {
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    pub(crate) fn typing_users(&self) -> &[(u64, Instant)] {
        self.typing
            .get(&self.selected_channel_id)
            .map_or(&[], Vec::as_slice)
    }
}
