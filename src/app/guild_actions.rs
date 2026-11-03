use gpui::*;
use gpui_tokio::Tokio;
use std::future::Future;

use crate::{
    app::DiscordApp,
    discord::{
        Error, Session,
        model::{GuildNotificationFlag, NotificationLevel},
    },
};

impl DiscordApp {
    fn run_guild_action<F, Fut>(&mut self, action: &'static str, request: F, cx: &mut Context<Self>)
    where
        F: FnOnce(Session) -> Fut,
        Fut: Future<Output = Result<(), Error>> + Send + 'static,
    {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let request = Tokio::spawn(cx, request(session));
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not {action}: {error}"),
            Err(error) => log::warn!("{action} task failed: {error}"),
        })
        .detach();
    }

    pub(crate) fn set_guild_mute(
        &mut self,
        guild_id: u64,
        mute: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        self.run_guild_action(
            "set server mute",
            move |session| async move { session.set_guild_mute(guild_id, mute).await },
            cx,
        );
    }

    pub(crate) fn set_guild_notifications(
        &mut self,
        guild_id: u64,
        level: NotificationLevel,
        cx: &mut Context<Self>,
    ) {
        if level == NotificationLevel::Inherit {
            log::warn!("Server notification levels cannot inherit");
            return;
        }
        self.run_guild_action(
            "set server notifications",
            move |session| async move { session.set_guild_notifications(guild_id, level).await },
            cx,
        );
    }

    pub(crate) fn set_guild_notification_flag(
        &mut self,
        guild_id: u64,
        flag: GuildNotificationFlag,
        on: bool,
        cx: &mut Context<Self>,
    ) {
        self.run_guild_action(
            "set server notification flag",
            move |session| async move {
                session
                    .set_guild_notification_flag(guild_id, flag, on)
                    .await
            },
            cx,
        );
    }
}
