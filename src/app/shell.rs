use gpui::{prelude::*, *};

use super::DiscordApp;

#[derive(Clone)]
pub(crate) enum Start {
    Saved,
    Login { error: Option<SharedString> },
    Token { token: String, user_id: u64 },
}

pub(crate) struct Restart(pub(crate) Start);

impl EventEmitter<Restart> for DiscordApp {}

pub struct AppShell {
    app: Entity<DiscordApp>,
    _restart: Subscription,
}

impl AppShell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (app, restart) = build(Start::Saved, window, cx);
        let mut activations = crate::notify::activations();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(activation) = activations.recv().await {
                let opened = this.update_in(cx, |shell, window, cx| {
                    shell
                        .app
                        .update(cx, |app, cx| app.open_notification(activation, window, cx))
                });
                if opened.is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            app,
            _restart: restart,
        }
    }

    fn restart(&mut self, start: Start, window: &mut Window, cx: &mut Context<Self>) {
        let (app, restart) = build(start, window, cx);
        self.app = app;
        self._restart = restart;
        cx.notify();
    }
}

fn build(
    start: Start,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) -> (Entity<DiscordApp>, Subscription) {
    let app = cx.new(|cx| DiscordApp::new(window, cx));
    app.update(cx, |app, cx| app.begin(start, window, cx));
    let restart = cx.subscribe_in(&app, window, |shell, _, Restart(start), window, cx| {
        shell.restart(start.clone(), window, cx)
    });
    (app, restart)
}

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.app.clone()
    }
}
