pub(super) mod member_list;
pub(super) mod member_menu;

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
            if message.webhook.is_none() {
                ids.push(message.author_id);
            }
            if let Some(ReplyState::Loaded(reply)) = &message.reply
                && reply.webhook.is_none()
            {
                ids.push(reply.author_id);
            }
            collect_mentions(&self.parsed(message.id, &message.content).blocks, &mut ids);
        }
        let missing = self.store.missing_members(guild_id, ids);
        if !missing.is_empty() {
            session.request_members(guild_id, missing);
        }
    }

    pub(super) fn request_voice_members(&mut self) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let guild_id = self.selected_guild_id;
        let Some(guild) = self.guild() else {
            return;
        };
        let ids: Vec<u64> = guild
            .categories
            .iter()
            .flat_map(|category| &category.channels)
            .filter(|channel| channel.kind == crate::store::ChannelKind::Voice)
            .flat_map(|channel| self.store.voice_members(guild_id, channel.id))
            .map(|member| member.user_id)
            .collect();
        let missing = self.store.missing_members(guild_id, ids);
        if !missing.is_empty() {
            session.request_members(guild_id, missing);
        }
    }

    pub(super) fn sync_member_rows(&mut self) {
        let lead = usize::from(!self.activity_cards().is_empty());
        self.member_layout.sync(
            &self.member_rows,
            self.store.member_list(self.selected_guild_id),
            lead,
        );
    }

    pub(super) fn arm_status_expiry_timer(&mut self, cx: &mut Context<Self>) {
        let custom_expiry = self
            .store
            .user(self.store.current_user_id)
            .and_then(|user| user.custom_status.as_ref())
            .and_then(|status| status.expires_at);
        let manual_expiry = self
            .session
            .as_ref()
            .and_then(|session| session.next_status_expiry());
        let now = SystemTime::now();
        let (expired, expiry) =
            status_expiry_plan(self.status_expiry, custom_expiry, manual_expiry, now);
        if expired {
            self.refresh_expired_self_status(now);
        }
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
                if app.status_expiry != Some(expires_at) {
                    return;
                }
                app.status_expiry_timer = None;
                if SystemTime::now() < expires_at {
                    app.status_expiry = None;
                }
                app.arm_status_expiry_timer(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn refresh_expired_self_status(&mut self, now: SystemTime) {
        if let Some(user) = self.store.users.get_mut(&self.store.current_user_id)
            && user
                .custom_status
                .as_ref()
                .is_some_and(|status| status.is_expired(now))
        {
            user.custom_status = None;
            user.activity = None;
            user.activity_kind = None;
            user.playing = false;
        }
        // Project the current settings, not the choice captured by an earlier timer.
        if let Some(status) = self.selected_self_status() {
            self.store.set_self_status(status);
        }
        if let Some(session) = self.session.as_ref().filter(|_| self.live)
            && let Err(error) = session.refresh_presence()
        {
            log::debug!("Could not refresh expired status presence: {error}");
        }
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
                app.load_activity_assets(cx);
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
        let channel_id = self.thread_parent().unwrap_or(self.selected_channel_id);
        let key = (self.selected_guild_id, channel_id, ranges);
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

fn status_expiry_plan(
    scheduled: Option<SystemTime>,
    custom: Option<SystemTime>,
    manual: Option<SystemTime>,
    now: SystemTime,
) -> (bool, Option<SystemTime>) {
    // The Session exposes only future manual deadlines. An unrelated event must
    // still process a due timer before replacing it with the next deadline.
    let expired = [scheduled, custom, manual]
        .into_iter()
        .flatten()
        .any(|expiry| expiry <= now);
    let next = [custom, manual]
        .into_iter()
        .flatten()
        .filter(|expiry| *expiry > now)
        .min();
    (expired, next)
}

#[cfg(test)]
mod status_expiry_tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::status_expiry_plan;

    #[test]
    fn both_deadlines_rearm_in_order_and_due_manual_timers_cannot_be_lost() {
        let before = UNIX_EPOCH + Duration::from_secs(100);
        let manual = before + Duration::from_secs(10);
        let custom = before + Duration::from_secs(20);
        assert_eq!(
            status_expiry_plan(None, Some(custom), Some(manual), before),
            (false, Some(manual))
        );
        assert_eq!(
            status_expiry_plan(Some(manual), Some(custom), None, manual),
            (true, Some(custom)),
            "a concurrent event still processes the exact manual deadline"
        );
        assert_eq!(
            status_expiry_plan(Some(custom), Some(custom), None, custom),
            (true, None)
        );
        assert_eq!(
            status_expiry_plan(None, Some(manual), Some(custom), before),
            (false, Some(manual)),
            "custom expiry can come first without clearing the manual deadline"
        );
    }

    #[test]
    fn replacing_or_clearing_deadlines_uses_the_current_snapshot() {
        let now = UNIX_EPOCH + Duration::from_secs(100);
        let old = now + Duration::from_secs(10);
        let newer = now + Duration::from_secs(20);
        assert_eq!(
            status_expiry_plan(Some(old), None, Some(newer), now),
            (false, Some(newer))
        );
        assert_eq!(
            status_expiry_plan(Some(old), None, None, now),
            (false, None)
        );
        assert_eq!(status_expiry_plan(None, None, None, now), (false, None));
        assert_eq!(
            status_expiry_plan(Some(old), Some(newer), None, old + Duration::from_secs(1)),
            (true, Some(newer)),
            "late callbacks refresh current state and retain the replacement"
        );
    }
}
