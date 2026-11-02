use gpui::*;
use gpui_tokio::Tokio;

use crate::{app::DiscordApp, discord::Error, store::DmCall};

#[derive(Default)]
pub(super) struct Calls {
    pub(super) pending_ring: Option<u64>,
    error: Option<(u64, String)>,
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

    /// Call when opening a DM to discover an existing call without joining it.
    pub(crate) fn request_dm_call(&self, channel_id: u64) {
        if self.store.private_channel(channel_id).is_some()
            && let Some(session) = self.session.as_ref().filter(|_| self.live)
        {
            session.request_dm_call(channel_id);
        }
    }

    /// Starts a new call or joins a known one. Only a new call rings, after connection.
    pub(crate) fn join_dm_call(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        if !self.live
            || !self.store.can_connect_voice(0, channel_id)
            || self
                .voice
                .call
                .as_ref()
                .is_some_and(|call| call.channel_id == channel_id)
        {
            return;
        }
        let ring = self.store.dm_call(channel_id).is_none();
        self.calls.error = None;
        self.join_voice(0, channel_id, cx);
        if ring
            && self
                .voice
                .call
                .as_ref()
                .is_some_and(|call| call.channel_id == channel_id)
        {
            self.calls.pending_ring = Some(channel_id);
        }
    }

    pub(crate) fn decline_dm_call(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        if !self
            .store
            .dm_call(channel_id)
            .is_some_and(|call| call.ringing.contains(&self.store.current_user_id))
        {
            return;
        }
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
        if self.calls.pending_ring != Some(channel_id) {
            return;
        }
        self.calls.pending_ring = None;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let request = Tokio::spawn(cx, async move { session.ring_call(channel_id).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
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
