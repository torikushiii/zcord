use gpui::*;
use gpui_tokio::Tokio;
use std::future::Future;

use crate::{
    app::{DiscordApp, direct::friends::failure_text},
    discord::{Error, Event, Session},
};

impl DiscordApp {
    fn run_dm_action<F, Fut>(&mut self, action: &'static str, request: F, cx: &mut Context<Self>)
    where
        F: FnOnce(Session) -> Fut,
        Fut: Future<Output = Result<(), (u64, Error)>> + Send + 'static,
    {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.dm_error = None;
        cx.notify();
        let request = Tokio::spawn(cx, request(session));
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err((target_id, error))) => {
                        log::warn!("Could not {action}: {error}");
                        app.dm_error = Some((
                            target_id,
                            failure_text(&error, "Discord refused this.").to_owned(),
                        ));
                    }
                    Err(error) => log::warn!("{action} task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn run_dm_action_in_window<F, Fut>(
        &mut self,
        channel_id: u64,
        action: &'static str,
        request: F,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(Session) -> Fut,
        Fut: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.dm_error = None;
        cx.notify();
        let request = Tokio::spawn(cx, request(session));
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                match result {
                    Ok(Ok(())) => {
                        if app.selected_channel_id == channel_id {
                            app.open_friends(window, cx);
                        }
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not {action}: {error}");
                        app.dm_error = Some((
                            channel_id,
                            failure_text(&error, "Discord refused this.").to_owned(),
                        ));
                    }
                    Err(error) => log::warn!("{action} task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn close_dm(
        &mut self,
        channel_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_dm_action_in_window(
            channel_id,
            "close DM",
            move |session| async move { session.close_dm(channel_id).await },
            window,
            cx,
        );
    }

    pub(crate) fn leave_group(
        &mut self,
        channel_id: u64,
        silent: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_dm_action_in_window(
            channel_id,
            "leave group",
            move |session| async move { session.leave_group(channel_id, silent).await },
            window,
            cx,
        );
    }

    pub(crate) fn block_user(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.run_dm_action(
            "block user",
            move |session| async move {
                session
                    .block_user(user_id)
                    .await
                    .map_err(|error| (user_id, error))
            },
            cx,
        );
    }

    pub(crate) fn unblock_user(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.run_dm_action(
            "unblock user",
            move |session| async move {
                session
                    .unblock_user(user_id)
                    .await
                    .map_err(|error| (user_id, error))
            },
            cx,
        );
    }

    pub(crate) fn ignore_user(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.run_dm_action(
            "ignore user",
            move |session| async move {
                session
                    .ignore_user(user_id)
                    .await
                    .map_err(|error| (user_id, error))
            },
            cx,
        );
    }

    pub(crate) fn unignore_user(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.run_dm_action(
            "unignore user",
            move |session| async move {
                session
                    .unignore_user(user_id)
                    .await
                    .map_err(|error| (user_id, error))
            },
            cx,
        );
    }

    pub(crate) fn set_friend_nickname(
        &mut self,
        user_id: u64,
        nickname: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.run_dm_action(
            "set friend nickname",
            move |session| async move {
                session
                    .set_friend_nickname(user_id, nickname.as_deref())
                    .await
                    .map_err(|error| (user_id, error))
            },
            cx,
        );
    }

    pub(crate) fn set_dm_mute(
        &mut self,
        channel_id: u64,
        mute: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        self.run_dm_action(
            "set DM mute",
            move |session| async move {
                session
                    .set_dm_mute(channel_id, mute)
                    .await
                    .map_err(|error| (channel_id, error))
            },
            cx,
        );
    }

    pub(crate) fn add_group_recipients(
        &mut self,
        channel_id: u64,
        recipients: Vec<u64>,
        cx: &mut Context<Self>,
    ) {
        self.run_dm_action(
            "add group recipient",
            move |session| async move {
                for user_id in recipients {
                    if let Err(error) = session.add_group_recipient(channel_id, user_id).await {
                        return Err((user_id, error));
                    }
                }
                Ok(())
            },
            cx,
        );
    }

    pub(crate) fn remove_group_recipient(
        &mut self,
        channel_id: u64,
        user_id: u64,
        cx: &mut Context<Self>,
    ) {
        self.run_dm_action(
            "remove group recipient",
            move |session| async move {
                session
                    .remove_group_recipient(channel_id, user_id)
                    .await
                    .map_err(|error| (user_id, error))
            },
            cx,
        );
    }

    pub(crate) fn create_group_dm(
        &mut self,
        recipients: Vec<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.dm_error = None;
        cx.notify();
        let target_id = recipients.first().copied().unwrap_or(0);
        let request = Tokio::spawn(
            cx,
            async move { session.create_group_dm(&recipients).await },
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                match result {
                    Ok(Ok(channel)) => {
                        let channel_id = channel.id;
                        let change = app.store.apply(Event::ChannelUpsert(Box::new(channel)));
                        app.apply_change(change, window, cx);
                        app.select_dm(channel_id, window, cx);
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not create group DM: {error}");
                        app.dm_error = Some((
                            target_id,
                            failure_text(&error, "Discord refused this.").to_owned(),
                        ));
                    }
                    Err(error) => log::warn!("Create group DM task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
