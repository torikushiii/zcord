use super::*;

impl DiscordApp {
    pub(super) fn request_missing_members(&mut self, channel_id: u64) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(guild) = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.channel(channel_id).is_some())
        else {
            return;
        };
        let guild_id = guild.id;
        let mut ids = Vec::new();
        for message in guild
            .channel(channel_id)
            .map_or(&[][..], |channel| &channel.messages)
        {
            ids.push(message.author_id);
            if let Some(ReplyState::Loaded(reply)) = &message.reply {
                ids.push(reply.author_id);
            }
            collect_mentions(&self.parsed(message.id, &message.content).blocks, &mut ids);
        }
        let missing = self.store.missing_members(guild_id, ids);
        if !missing.is_empty() {
            session.request_members(guild_id, missing);
        }
    }

    pub(super) fn sync_member_rows(&mut self) {
        self.member_layout.sync(
            &self.member_rows,
            self.store.member_list(self.selected_guild_id),
        );
    }

    pub(super) fn arm_status_expiry_timer(&mut self, cx: &mut Context<Self>) {
        let expiry = self
            .store
            .user(self.store.current_user_id)
            .and_then(|user| user.custom_status.as_ref())
            .and_then(|status| status.expires_at);
        if expiry == self.status_expiry {
            return;
        }
        self.status_expiry = expiry;
        let Some(expires_at) = expiry else {
            self.status_expiry_timer = None;
            return;
        };
        self.status_expiry_timer = Some(cx.spawn(async move |this, cx| {
            let delay = expires_at
                .duration_since(SystemTime::now())
                .unwrap_or_default();
            cx.background_executor().timer(delay).await;
            this.update(cx, |app, cx| {
                let id = app.store.current_user_id;
                let Some(user) = app.store.users.get_mut(&id) else {
                    return;
                };
                let same = user
                    .custom_status
                    .as_ref()
                    .is_some_and(|status| status.expires_at == Some(expires_at));
                if !same {
                    return;
                }
                if !user
                    .custom_status
                    .as_ref()
                    .is_some_and(|status| status.is_expired(SystemTime::now()))
                {
                    app.status_expiry = None;
                    app.arm_status_expiry_timer(cx);
                    return;
                }
                user.custom_status = None;
                user.activity = None;
                user.playing = false;
                app.status_expiry = None;
                app.status_expiry_timer = None;
                cx.notify();
            })
            .ok();
        }));
    }

    pub(super) fn arm_members_timer(&mut self, cx: &mut Context<Self>) {
        if self.members_timer.is_some() {
            return;
        }
        self.members_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COALESCE_DELAY).await;
            this.update(cx, |app, cx| {
                app.members_timer = None;
                app.sync_member_rows();
                cx.notify();
            })
            .ok();
        }));
    }

    pub(super) fn update_member_subscription(&mut self) {
        let Some(session) = self.session.as_ref().filter(|_| self.live) else {
            return;
        };
        if self.channel().is_none() || self.is_home() {
            return;
        }
        let top = self.member_top as u32;
        let chunk = top / 100;
        let mut ranges = vec![(0, 99)];
        for page in [chunk, chunk + 1] {
            if page > 0 {
                ranges.push((page * 100, page * 100 + 99));
            }
        }
        let key = (self.selected_guild_id, self.selected_channel_id, ranges);
        if self.member_subscription.as_ref() == Some(&key) {
            return;
        }
        session.subscribe_channel(key.0, key.1, &key.2);
        self.member_subscription = Some(key);
    }

    pub(crate) fn toggle_members(&mut self, cx: &mut Context<Self>) {
        self.show_members = !self.show_members;
        cx.notify();
    }
}
