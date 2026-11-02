use std::{rc::Rc, time::Duration};

use futures::StreamExt;
use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_tokio::Tokio;

use crate::{
    app::{DiscordApp, Screen},
    discord::remote_auth::{self, QrMatrix, RemoteAuth, RemoteAuthEvent, RemoteUser},
    media::images,
    store,
    theme::{self, rpx},
};

const CARD_WIDTH: f32 = 784.;
const FORM_WIDTH: f32 = 480.;
const QR_SIZE: f32 = 176.;
const QR_PADDING: f32 = 8.;
const QR_LOGO: f32 = 40.;
const SCANNED_AVATAR: f32 = 100.;

pub struct LoginForm {
    pub token: Entity<InputState>,
    pub error: Option<SharedString>,
    pub busy: bool,
    qr: QrLogin,
    _submit_on_enter: Subscription,
}

enum QrState {
    Loading,
    Code(Rc<QrMatrix>),
    Scanned(RemoteUser),
    Failed(SharedString),
}

struct QrLogin {
    state: QrState,
    _auth: Option<RemoteAuth>,
    _events: Option<Task<()>>,
}

impl QrLogin {
    fn start(window: &mut Window, cx: &mut Context<DiscordApp>) -> Self {
        let (auth, mut events) = RemoteAuth::start(&Tokio::handle(cx));
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some(event) = events.next().await {
                let alive = this.update_in(cx, |app, window, cx| {
                    app.handle_remote_auth(event, window, cx)
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        Self {
            state: QrState::Loading,
            _auth: Some(auth),
            _events: Some(task),
        }
    }
}

impl LoginForm {
    pub fn new(window: &mut Window, cx: &mut Context<DiscordApp>) -> Self {
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Paste your account token")
        });
        let submit = cx.subscribe_in(&token, window, |app, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                app.submit_login(window, cx);
            }
        });
        token.update(cx, |input, cx| input.focus(window, cx));
        Self {
            token,
            error: None,
            busy: false,
            qr: QrLogin::start(window, cx),
            _submit_on_enter: submit,
        }
    }
}

impl DiscordApp {
    fn handle_remote_auth(
        &mut self,
        event: RemoteAuthEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Screen::Login(form) = &mut self.screen else {
            return;
        };
        let state = match event {
            RemoteAuthEvent::Qr(text) => match remote_auth::qr_matrix(&text) {
                Some(matrix) => QrState::Code(Rc::new(matrix)),
                None => QrState::Failed("Couldn't draw the QR code.".into()),
            },
            RemoteAuthEvent::Scanned(user) => QrState::Scanned(user),
            RemoteAuthEvent::Failed(message) => QrState::Failed(message.into()),
            RemoteAuthEvent::Token(token) => {
                form.qr._auth = None;
                self.login_with_token(token.into_inner(), window, cx);
                return;
            }
        };
        form.qr.state = state;
        cx.notify();
    }

    fn restart_qr_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Login(_)) {
            return;
        }
        let qr = QrLogin::start(window, cx);
        if let Screen::Login(form) = &mut self.screen {
            form.qr = qr;
        }
        cx.notify();
    }
}

pub fn render(
    form: &LoginForm,
    window: &Window,
    cx: &mut Context<DiscordApp>,
) -> impl IntoElement + use<> {
    let viewport = window.viewport_size().width / window.rem_size() * theme::BASE_FONT_SIZE;
    let show_qr = viewport >= CARD_WIDTH + 48.;
    div()
        .flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .bg(rgb(theme::bg()))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(64.))
                .w(rpx(if show_qr { CARD_WIDTH } else { FORM_WIDTH }))
                .p(rpx(32.))
                .rounded(rpx(8.))
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgba(theme::border()))
                .child(token_form(form, cx))
                .when(show_qr, |card| card.child(qr_panel(&form.qr.state, cx))),
        )
}

fn token_form(form: &LoginForm, cx: &mut Context<DiscordApp>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .flex()
                .justify_center()
                .text_size(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Welcome back!"),
        )
        .child(
            div()
                .flex()
                .justify_center()
                .mt_2()
                .text_size(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child("We're so excited to see you again!"),
        )
        .child(
            div()
                .flex()
                .gap_1()
                .mt(rpx(20.))
                .mb_2()
                .text_size(rpx(12.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(if form.error.is_some() {
                    theme::RED
                } else {
                    theme::muted()
                }))
                .child("TOKEN")
                .child(match &form.error {
                    Some(error) => div()
                        .font_weight(FontWeight::NORMAL)
                        .italic()
                        .child(format!("- {error}")),
                    None => div().text_color(rgb(theme::RED)).child("*"),
                }),
        )
        .child(
            div()
                .px_3()
                .rounded(rpx(8.))
                .bg(rgb(theme::bg()))
                .border_1()
                .border_color(rgba(theme::border()))
                .child(
                    Input::new(&form.token)
                        .appearance(false)
                        .mask_toggle()
                        .h(rpx(42.))
                        .px_0()
                        .text_size(rpx(16.))
                        .line_height(rpx(22.)),
                ),
        )
        .child(
            div()
                .id("login")
                .flex()
                .items_center()
                .justify_center()
                .mt(rpx(20.))
                .h(rpx(44.))
                .rounded(rpx(8.))
                .bg(rgb(theme::BLURPLE))
                .text_size(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(white())
                .map(|button| {
                    if form.busy {
                        button.opacity(0.6).child("Connecting…")
                    } else {
                        button
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(theme::BLURPLE)))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.submit_login(window, cx)),
                            )
                            .child("Log In")
                    }
                }),
        )
        .child(
            div()
                .mt_2()
                .text_size(rpx(12.))
                .text_color(rgb(theme::faint()))
                .child("Stored in your system's credential manager and sent only to Discord."),
        )
}

fn qr_panel(state: &QrState, cx: &mut Context<DiscordApp>) -> Div {
    let (title, body): (SharedString, AnyElement) = match state {
        QrState::Scanned(user) => (
            "Check your phone!".into(),
            div()
                .flex()
                .flex_col()
                .items_center()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .justify_center()
                        .child("Logged in as ")
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::strong()))
                                .child(user.username.clone()),
                        ),
                )
                .child(
                    link("qr-restart", "Not me, start over")
                        .mt(rpx(8.))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.restart_qr_login(window, cx)),
                        ),
                )
                .into_any_element(),
        ),
        QrState::Failed(message) => {
            (
                "Log in with QR Code".into(),
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(message.clone())
                    .child(link("qr-retry", "Try again").mt(rpx(8.)).on_click(
                        cx.listener(|this, _, window, cx| this.restart_qr_login(window, cx)),
                    ))
                    .into_any_element(),
            )
        }
        QrState::Loading | QrState::Code(_) => (
            "Log in with QR Code".into(),
            div()
                .child("Scan this with the ")
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child("Discord mobile app"),
                )
                .child(" to log in instantly.")
                .flex()
                .flex_wrap()
                .justify_center()
                .into_any_element(),
        ),
    };
    div()
        .flex()
        .flex_col()
        .flex_none()
        .items_center()
        .w(rpx(240.))
        .child(qr_visual(state, cx))
        .child(
            div()
                .mt(rpx(32.))
                .mb_2()
                .text_size(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(title),
        )
        .child(
            div()
                .w_full()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .text_center()
                .text_color(rgb(theme::muted()))
                .child(body),
        )
}

fn qr_visual(state: &QrState, cx: &App) -> AnyElement {
    let frame = div()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(QR_SIZE))
        .rounded(rpx(4.));
    match state {
        QrState::Code(matrix) => frame
            .p(rpx(QR_PADDING))
            .bg(white())
            .child(qr_modules(matrix.clone()))
            .child(
                div()
                    .absolute()
                    .top(rpx((QR_SIZE - QR_LOGO) / 2.))
                    .left(rpx((QR_SIZE - QR_LOGO) / 2.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(QR_LOGO))
                    .rounded(rpx(6.))
                    .bg(white())
                    .child(theme::logo(QR_LOGO - 12.)),
            )
            .into_any_element(),
        QrState::Loading => frame
            .bg(white())
            .child(theme::logo(QR_LOGO - 12.))
            .with_animation(
                "qr-loading",
                Animation::new(Duration::from_millis(1200))
                    .repeat()
                    .with_easing(pulsating_between(0.35, 0.8)),
                |frame, delta| frame.opacity(delta),
            )
            .into_any_element(),
        QrState::Scanned(user) => {
            let spec =
                store::avatar_spec(user.id, user.avatar.as_deref(), Some(&user.discriminator));
            let image = images::resolve(&spec, theme::image_px(SCANNED_AVATAR), cx);
            frame
                .child(match image {
                    Some(path) => img(path)
                        .size(rpx(SCANNED_AVATAR))
                        .rounded_full()
                        .into_any_element(),
                    None => div()
                        .size(rpx(SCANNED_AVATAR))
                        .rounded_full()
                        .bg(theme::user_color(user.id))
                        .into_any_element(),
                })
                .into_any_element()
        }
        QrState::Failed(_) => frame
            .bg(rgb(theme::bg()))
            .child(theme::logo(QR_LOGO).opacity(0.5))
            .into_any_element(),
    }
}

fn qr_modules(matrix: Rc<QrMatrix>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let width = matrix.width.max(1);
            let module = bounds.size.width / width as f32;
            let color = rgb(0x000000);
            // Horizontal runs keep the quad count low and avoid hairline seams between modules.
            for (y, row) in matrix.dark.chunks(width).enumerate() {
                let mut x = 0;
                while x < row.len() {
                    if !row[x] {
                        x += 1;
                        continue;
                    }
                    let start = x;
                    while x < row.len() && row[x] {
                        x += 1;
                    }
                    let origin = point(
                        bounds.origin.x + module * start as f32,
                        bounds.origin.y + module * y as f32,
                    );
                    let size = size(module * (x - start) as f32, module);
                    window.paint_quad(fill(Bounds { origin, size }, color));
                }
            }
        },
    )
    .size_full()
}

fn link(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .text_size(rpx(14.))
        .text_color(rgb(theme::link()))
        .cursor_pointer()
        .hover(|style| style.underline())
        .child(label)
}
