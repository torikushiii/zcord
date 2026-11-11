use super::*;

impl DiscordApp {
    pub(crate) fn submit_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let adding = self.accounts.adding.is_some();
        let Some(form) = self.login_form() else {
            return;
        };
        if form.busy {
            return;
        }
        let token = form.token.read(cx).value().trim().to_owned();
        if token.is_empty() {
            form.error = Some("This field is required".into());
            cx.notify();
            return;
        }
        if adding {
            self.check_added_token(token, window, cx);
        } else {
            self.login_with_token(token, window, cx);
        }
    }

    pub(crate) fn login_with_token(
        &mut self,
        token: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Screen::Login(form) = &mut self.screen {
            form.busy = true;
            form.error = None;
        }
        self.pending_token = Some(token.clone());
        self.connect(token, window, cx);
        cx.notify();
    }

    pub(super) fn disconnect(&mut self) {
        self.calls = calls::Calls::default();
        self.self_status.disconnect();
        self.self_status.error = None;
        self.expression_info.reset();
        self.settings_page = None;
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
        self._events = None;
        self.pending_token = None;
        self.reconnecting = false;
        self.history.loading.clear();
        self.history.older.clear();
        self.history.newer.clear();
        self.history.pending_present.clear();
        self.history.present_limit.clear();
        self.forget_trimmed();
        self.history.failed.clear();
        self.history.jump_return = None;
        self.compose.pending_sends.clear();
        self.history.complete.clear();
        self.forum_pages.clear();
        self.invites.clear();
        self.member_subscription = None;
        self.members_timer = None;
        self.status_expiry = None;
        self.status_expiry_timer = None;
        self.voice.call = None;
        self.voice.call_view = None;
        self.voice_lobby = None;
        self.affinity_refresh = None;
        self.voice.speaking.clear();
        self.game.application = None;
        self.game.current = None;
        self.thread_previews.clear();
        self.menus.reactions = None;
        self.dismiss_reaction_tooltip();
        self.update_game_activity();
    }

    pub(super) fn connect(&mut self, token: String, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect_keeping_pending();
        let (session, mut events) = Session::connect(token, &Tokio::handle(cx));
        self.session = Some(session);
        self.update_game_activity();
        if !matches!(self.screen, Screen::Login(_)) {
            self.screen = Screen::Connecting;
        }
        self._events = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events.next().await {
                let alive =
                    this.update_in(cx, |app, window, cx| app.handle_event(event, window, cx));
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    fn disconnect_keeping_pending(&mut self) {
        let pending = self.pending_token.take();
        self.disconnect();
        self.pending_token = pending;
    }
}
