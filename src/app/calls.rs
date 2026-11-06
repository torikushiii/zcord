use std::sync::atomic::{AtomicU64, Ordering};

use gpui::*;
use gpui_tokio::Tokio;

use crate::{app::DiscordApp, discord::Error, store::DmCall};

#[derive(Default)]
pub(super) struct Calls {
    pub(super) pending_ring: Option<u64>,
    checking: Option<(u64, u64)>,
    ringing: Option<u64>,
    error: Option<(u64, String)>,
}

impl Calls {
    pub(super) fn cancel_pending(&mut self) {
        self.pending_ring = None;
        self.checking = None;
        self.ringing = None;
    }
}

// Tokens outlive `Calls`, which is rebuilt on every disconnect.
fn next_token() -> u64 {
    static TOKENS: AtomicU64 = AtomicU64::new(0);
    TOKENS.fetch_add(1, Ordering::Relaxed) + 1
}

impl DiscordApp {
    pub(crate) fn dm_call(&self, channel_id: u64) -> Option<&DmCall> {
        self.store.dm_call(channel_id)
    }

    pub(crate) fn incoming_call(&self) -> Option<u64> {
        self.store.incoming_call()
    }

    pub(crate) fn dm_call_error(&self) -> Option<(u64, &str)> {
        self.calls
            .error
            .as_ref()
            .map(|(id, error)| (*id, error.as_str()))
    }

    pub(crate) fn request_dm_call(&self, channel_id: u64) {
        if self.store.private_channel(channel_id).is_some()
            && let Some(session) = self.session.as_ref().filter(|_| self.live)
        {
            session.request_dm_call(channel_id);
        }
    }

    pub(crate) fn join_dm_call(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        if !self.live
            || !self.store.can_connect_voice(0, channel_id)
            || self
                .calls
                .checking
                .is_some_and(|(checking, _)| checking == channel_id)
            || self
                .voice
                .call
                .as_ref()
                .is_some_and(|call| call.channel_id == channel_id)
        {
            return;
        }
        self.calls.error = None;
        let announced = self.call_announced(channel_id);
        let recipient = self
            .store
            .private_channel(channel_id)
            .filter(|channel| !channel.group)
            .and_then(|channel| channel.recipient_ids.first().copied());
        match recipient.filter(|_| !announced) {
            Some(recipient) => self.check_ringable(channel_id, recipient, cx),
            None => self.start_dm_call(channel_id, !announced, cx),
        }
    }

    fn call_announced(&self, channel_id: u64) -> bool {
        self.store
            .dm_call(channel_id)
            .is_some_and(|call| call.message_id.is_some())
    }

    pub(super) fn start_dm_call(&mut self, channel_id: u64, ring: bool, cx: &mut Context<Self>) {
        self.join_voice(0, channel_id, cx);
        if ring
            && self
                .voice
                .call
                .as_ref()
                .is_some_and(|call| call.channel_id == channel_id)
        {
            self.calls.pending_ring = Some(channel_id);
            self.ring_pending_dm_call(channel_id, cx);
        }
    }

    fn check_ringable(&mut self, channel_id: u64, recipient: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let token = next_token();
        let user_id = self.store.current_user_id;
        self.calls.checking = Some((channel_id, token));
        let request = Tokio::spawn(cx, async move { session.call_ringable(channel_id).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                if app.calls.checking != Some((channel_id, token))
                    || app.store.current_user_id != user_id
                {
                    return;
                }
                app.calls.checking = None;
                match result {
                    Ok(Ok(ringable)) => {
                        if app.live {
                            let ring = ringable && !app.call_announced(channel_id);
                            app.start_dm_call(channel_id, ring, cx);
                        }
                    }
                    Ok(Err(Error::Rejected { .. })) => {
                        let name = app
                            .store
                            .user(recipient)
                            .map_or("this user", |user| user.username.as_str());
                        app.calls.error = Some((
                            channel_id,
                            format!("You need to be friends with {name} to start a call."),
                        ));
                    }
                    Ok(Err(error)) => app.call_action_failed(channel_id, error),
                    Err(error) => {
                        log::warn!("Call eligibility task failed: {error}");
                        app.calls.error =
                            Some((channel_id, "Could not start the call.".to_owned()));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn decline_dm_call(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        let Some(call) = self
            .store
            .dm_call(channel_id)
            .filter(|call| call.ringing.contains(&self.store.current_user_id))
        else {
            return;
        };
        let declined = call.message_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.calls.error = None;
        let user_id = self.store.current_user_id;
        let request = Tokio::spawn(cx, async move {
            session.stop_ringing_call(channel_id, user_id).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let same_call = app
                    .store
                    .dm_call(channel_id)
                    .is_some_and(|call| call.message_id == declined);
                if !same_call || app.store.current_user_id != user_id {
                    return;
                }
                match result {
                    Ok(Ok(())) => app.store.stop_ringing_locally(channel_id),
                    Ok(Err(error)) => app.call_action_failed(channel_id, error),
                    Err(error) => {
                        log::warn!("Decline call task failed: {error}");
                        app.calls.error =
                            Some((channel_id, "Could not decline the call.".to_owned()));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn ring_pending_dm_call(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        let joining = self
            .voice
            .call
            .as_ref()
            .is_some_and(|call| call.channel_id == channel_id);
        if self.calls.pending_ring != Some(channel_id)
            || !joining
            || !self.call_announced(channel_id)
        {
            return;
        }
        self.calls.pending_ring = None;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let token = next_token();
        let user_id = self.store.current_user_id;
        self.calls.ringing = Some(token);
        let request = Tokio::spawn(cx, async move { session.ring_call(channel_id).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                if app.calls.ringing != Some(token) || app.store.current_user_id != user_id {
                    return;
                }
                app.calls.ringing = None;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => app.call_action_failed(channel_id, error),
                    Err(error) => {
                        log::warn!("Ring call task failed: {error}");
                        app.calls.error = Some((channel_id, "Could not ring the call.".to_owned()));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn reconcile_dm_call(&mut self, cx: &mut Context<Self>) {
        if self.voice.call.as_ref().is_some_and(|call| {
            call.guild_id == 0 && self.store.private_channel(call.channel_id).is_none()
        }) {
            self.leave_voice(cx);
        }
    }

    fn call_action_failed(&mut self, channel_id: u64, error: Error) {
        log::warn!("Call action failed: {error}");
        self.calls.error = Some((
            channel_id,
            crate::app::friends::failure_text(&error, "Discord refused this call action.")
                .to_owned(),
        ));
    }
}
