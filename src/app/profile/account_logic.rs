use gpui::{App, ClickEvent, ClipboardItem, Context, Window};
use gpui_tokio::Tokio;

use crate::{
    app::DiscordApp,
    discord::{Error, SelfStatus, StatusDuration},
};

/// Actions the bottom-left account popout can perform. Discord's other entries (editing
/// profiles, account switching, clips and orbs) need APIs this client lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountAction {
    ViewProfile,
    Settings,
    CopyUserId,
}

/// Only one change at a time; a disconnect discards its result so it can't overwrite newer
/// settings.
#[derive(Default)]
pub(crate) struct StatusChange {
    serial: u64,
    pub pending: Option<SelfStatus>,
    pub error: Option<String>,
}

impl StatusChange {
    fn begin(&mut self, connected: bool, status: SelfStatus) -> Option<u64> {
        if !connected || self.pending.is_some() {
            return None;
        }
        self.serial = self.serial.wrapping_add(1);
        self.pending = Some(status);
        self.error = None;
        Some(self.serial)
    }

    fn finish(&mut self, serial: u64, result: Result<(), Error>) -> bool {
        if self.serial != serial || self.pending.is_none() {
            return false;
        }
        self.pending = None;
        self.error = result.err().map(|error| match error {
            Error::SettingsOutdated => "Status changed elsewhere. Choose again.".into(),
            Error::Closed => "Disconnected. Reconnect to change status.".into(),
            _ => "Couldn't finish changing status. It may have been saved; choose again.".into(),
        });
        true
    }

    pub(crate) fn disconnect(&mut self) {
        self.serial = self.serial.wrapping_add(1);
        if self.pending.take().is_some() {
            self.error = Some("Connection interrupted. Status may have been saved.".into());
        }
    }
}

impl DiscordApp {
    pub(crate) fn can_set_self_status(&self) -> bool {
        self.live
            && !self.reconnecting
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.status_connected())
            && self.self_status.pending.is_none()
    }

    pub(crate) fn selected_self_status(&self) -> Option<SelfStatus> {
        self.session
            .as_ref()
            .and_then(|session| session.self_status())
    }

    pub(crate) fn set_self_status(
        &mut self,
        status: SelfStatus,
        duration: StatusDuration,
        cx: &mut Context<Self>,
    ) {
        let connected = self.can_set_self_status();
        let Some(serial) = self.self_status.begin(connected, status) else {
            return;
        };
        let Some(session) = self.session.clone() else {
            return;
        };
        let request = Tokio::spawn(
            cx,
            async move { session.set_status_for(status, duration).await },
        );
        cx.spawn(async move |this, cx| {
            let result = request.await.unwrap_or(Err(Error::Closed));
            this.update(cx, |app, cx| {
                if app.self_status.finish(serial, result) {
                    // A settings update from another client may have overtaken the REST
                    // response. Read the latest session state, never the submitted value.
                    if let Some(status) = app.selected_self_status() {
                        app.store.set_self_status(status);
                    }
                    app.arm_status_expiry_timer(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

/// A click handler bound to the app entity, matching the popout's existing handler shape.
/// Viewing the full profile keeps global context. Status changes keep the popout open
/// for progress/errors; settings and copying the ID close it first.
pub(crate) fn account_action(
    app: &DiscordApp,
    action: AccountAction,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = app.entity.clone();
    move |_, window, cx| {
        cx.stop_propagation();
        entity
            .update(cx, |app, cx| match action {
                AccountAction::ViewProfile => app.open_full_profile(cx),
                AccountAction::Settings => {
                    app.close_profile(cx);
                    app.open_settings(window, cx);
                }
                AccountAction::CopyUserId => {
                    app.close_profile(cx);
                    let user_id = app.store.current_user_id;
                    cx.write_to_clipboard(ClipboardItem::new_string(user_id.to_string()));
                }
            })
            .ok();
    }
}

#[cfg(test)]
mod tests {
    use super::StatusChange;
    use crate::discord::{Error, SelfStatus};

    #[test]
    fn rapid_clicks_are_disabled_and_errors_allow_an_explicit_retry() {
        let mut state = StatusChange::default();
        assert!(state.begin(false, SelfStatus::Online).is_none());
        let first = state.begin(true, SelfStatus::Idle).unwrap();
        assert!(state.begin(true, SelfStatus::Invisible).is_none());
        assert!(state.finish(first, Err(Error::Network)));
        assert!(state.error.is_some());
        let second = state.begin(true, SelfStatus::Invisible).unwrap();
        assert!(state.error.is_none());
        assert!(!state.finish(first, Ok(())));
        assert_eq!(state.pending, Some(SelfStatus::Invisible));
        assert!(state.finish(second, Ok(())));
        assert!(state.pending.is_none());
    }

    #[test]
    fn disconnect_never_replays_an_old_click_or_applies_its_completion() {
        let mut state = StatusChange::default();
        let old = state.begin(true, SelfStatus::Online).unwrap();
        state.disconnect();
        assert!(state.pending.is_none());
        assert!(state.error.is_some());
        let current = state.begin(true, SelfStatus::DoNotDisturb).unwrap();
        assert!(!state.finish(old, Ok(())));
        assert_eq!(state.pending, Some(SelfStatus::DoNotDisturb));
        assert!(state.finish(current, Err(Error::SettingsOutdated)));
        assert!(state.error.as_deref().unwrap().contains("elsewhere"));
    }
}
