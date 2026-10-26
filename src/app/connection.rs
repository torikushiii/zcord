use super::*;

impl DiscordApp {
    pub(crate) fn submit_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Login(form) = &mut self.screen else {
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
        self.login_with_token(token, window, cx);
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

    pub(crate) fn log_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect();
        crate::settings::update(cx, |settings| settings.voice_channel = None);
        if let Err(error) = credentials::clear() {
            log::warn!("{error}");
        }
        self.live = false;
        self.store = Store::seed();
        self.screen = Screen::Login(LoginForm::new(window, cx));
        cx.notify();
    }

    pub(super) fn disconnect(&mut self) {
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
        self._events = None;
        self.pending_token = None;
        self.reconnecting = false;
        self.loading.clear();
        self.loading_older.clear();
        self.loading_newer.clear();
        self.pending_present.clear();
        self.pending_sends.clear();
        self.history_complete.clear();
        self.forum_pages.clear();
        self.invites.clear();
        self.member_subscription = None;
        self.members_timer = None;
        self.status_expiry = None;
        self.status_expiry_timer = None;
        self.voice.call = None;
        self.voice.speaking.clear();
    }

    pub(super) fn connect(&mut self, token: String, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect_keeping_pending();
        let (session, mut events) = Session::connect(token, &Tokio::handle(cx));
        self.session = Some(session);
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
