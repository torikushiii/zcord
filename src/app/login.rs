use std::{rc::Rc, time::Duration};

use futures::StreamExt;
use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_tokio::Tokio;

use crate::{
    app::{
        DiscordApp, Screen,
        accounts::{
            Accounts, ListPlace, account_list, add_account_link,
            modal::{self, Dialog},
        },
    },
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
    pub(crate) mode: LoginMode,
    qr: Option<QrLogin>,
    _submit_on_enter: Subscription,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoginMode {
    Choose,
    Token { back: bool },
}

enum QrState {
    Loading,
    Code(Rc<QrMatrix>),
    Scanned(RemoteUser),
    Captcha(SharedString),
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
        let mode = if crate::settings::accounts(cx).is_empty() {
            LoginMode::Token { back: false }
        } else {
            LoginMode::Choose
        };
        Self::with_mode(mode, window, cx)
    }

    pub fn with_error(
        error: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<DiscordApp>,
    ) -> Self {
        let mut form = match error {
            Some(_) if !crate::settings::accounts(cx).is_empty() => {
                Self::with_mode(LoginMode::Token { back: true }, window, cx)
            }
            _ => Self::new(window, cx),
        };
        form.error = error;
        form
    }

    pub(crate) fn with_mode(
        mode: LoginMode,
        window: &mut Window,
        cx: &mut Context<DiscordApp>,
    ) -> Self {
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
        let mut form = Self {
            token,
            error: None,
            busy: false,
            mode: LoginMode::Choose,
            qr: None,
            _submit_on_enter: submit,
        };
        form.set_mode(mode, window, cx);
        form
    }

    pub(crate) fn set_mode(
        &mut self,
        mode: LoginMode,
        window: &mut Window,
        cx: &mut Context<DiscordApp>,
    ) {
        self.mode = mode;
        self.busy = false;
        match mode {
            LoginMode::Choose => {
                self.qr = None;
                self.error = None;
            }
            LoginMode::Token { .. } => {
                if self.qr.is_none() {
                    self.qr = Some(QrLogin::start(window, cx));
                }
                self.token.update(cx, |input, cx| input.focus(window, cx));
            }
        }
    }
}

impl DiscordApp {
    pub(crate) fn login_form(&mut self) -> Option<&mut LoginForm> {
        if let Some(form) = self.accounts.adding.as_mut() {
            return Some(form);
        }
        match &mut self.screen {
            Screen::Login(form) => Some(form),
            _ => None,
        }
    }

    fn handle_remote_auth(
        &mut self,
        event: RemoteAuthEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let adding = self.accounts.adding.is_some();
        let Some(form) = self.login_form() else {
            return;
        };
        let Some(qr) = form.qr.as_mut() else {
            return;
        };
        let state = match event {
            RemoteAuthEvent::Qr(text) => match remote_auth::qr_matrix(&text) {
                Some(matrix) => QrState::Code(Rc::new(matrix)),
                None => QrState::Failed("Couldn't draw the QR code.".into()),
            },
            RemoteAuthEvent::Scanned(user) => QrState::Scanned(user),
            RemoteAuthEvent::Captcha(url) => {
                cx.open_url(&url);
                QrState::Captcha(url.into())
            }
            RemoteAuthEvent::Failed(message) => QrState::Failed(message.into()),
            RemoteAuthEvent::Token(token) => {
                qr._auth = None;
                let scanned = match &qr.state {
                    QrState::Scanned(user) => Some(user.clone()),
                    _ => None,
                };
                if adding {
                    form.busy = true;
                    form.error = None;
                    let form_id = form.token.entity_id();
                    match scanned {
                        Some(user) => {
                            let account = crate::settings::SavedAccount {
                                id: user.id,
                                username: user.username,
                                discriminator: Some(user.discriminator),
                                avatar: user.avatar,
                                invalid: false,
                            };
                            self.add_account(form_id, account, token.into_inner(), window, cx);
                        }
                        None => self.check_added_token(token.into_inner(), window, cx),
                    }
                } else {
                    self.login_with_token(token.into_inner(), window, cx);
                }
                return;
            }
        };
        qr.state = state;
        cx.notify();
    }

    fn restart_qr_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.login_form().is_some_and(|form| form.qr.is_some()) {
            return;
        }
        let qr = QrLogin::start(window, cx);
        if let Some(form) = self.login_form() {
            form.qr = Some(qr);
        }
        cx.notify();
    }

    pub(crate) fn show_token_form(
        &mut self,
        back: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Screen::Login(form) = &mut self.screen {
            form.set_mode(LoginMode::Token { back }, window, cx);
            cx.notify();
        }
    }

    fn login_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Login(form) = &mut self.screen else {
            return;
        };
        if form.mode != (LoginMode::Token { back: true }) {
            return;
        }
        if crate::settings::accounts(cx).is_empty() {
            form.set_mode(LoginMode::Token { back: false }, window, cx);
        } else {
            form.set_mode(LoginMode::Choose, window, cx);
            self.validate_accounts(cx);
        }
        cx.notify();
    }
}

pub fn render(
    form: &LoginForm,
    accounts: &Accounts,
    window: &Window,
    cx: &mut Context<DiscordApp>,
) -> impl IntoElement + use<> {
    let viewport = window.viewport_size().width / window.rem_size() * theme::BASE_FONT_SIZE;
    let qr = form.qr.as_ref().filter(|_| viewport >= CARD_WIDTH + 48.);
    let card = div()
        .flex()
        .items_center()
        .gap(rpx(64.))
        .w(rpx(if qr.is_some() { CARD_WIDTH } else { FORM_WIDTH }))
        .max_w_full()
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()));
    let card = match form.mode {
        LoginMode::Choose => card.child(chooser(form, accounts, cx)),
        LoginMode::Token { back } => card.p(rpx(32.)).child(token_form(form, back, cx)).children(
            qr.map(|qr| qr_panel(&qr.state, cx.entity().downgrade(), theme::muted(), cx)),
        ),
    };
    div()
        .flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .px(rpx(16.))
        .bg(rgb(theme::bg()))
        .child(card)
}

fn chooser(form: &LoginForm, accounts: &Accounts, cx: &mut Context<DiscordApp>) -> Div {
    let saved = crate::settings::accounts(cx);
    let full = saved.len() >= crate::settings::MAX_ACCOUNTS;
    let list = account_list(
        &saved,
        accounts,
        ListPlace::Chooser { busy: form.busy },
        None,
        cx.entity().downgrade(),
        cx,
    );
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .pt(rpx(24.))
        .px(rpx(16.))
        .pb(rpx(16.))
        .child(
            div()
                .flex()
                .justify_center()
                .text_size(rpx(24.))
                .line_height(rpx(30.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Choose an account"),
        )
        .child(
            div()
                .flex()
                .justify_center()
                .text_center()
                .mt(rpx(8.))
                .mb(rpx(24.))
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .text_color(rgb(theme::muted()))
                .child("Select an account to log in with or add a new one."),
        )
        .child(list)
        .child(
            div()
                .flex()
                .justify_center()
                .mt(rpx(28.))
                .mb(rpx(12.))
                .child(add_account_link(
                    "login-add-account",
                    !full && !form.busy,
                    cx.listener(|this, _, window, cx| this.show_token_form(true, window, cx)),
                )),
        )
}

fn token_form(form: &LoginForm, back: bool, cx: &mut Context<DiscordApp>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .when(back, |this| {
            this.child(
                div().flex().mb(rpx(12.)).child(
                    div()
                        .id("login-back")
                        .flex()
                        .items_center()
                        .gap(rpx(6.))
                        .cursor_pointer()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::muted()))
                        .hover(|style| style.text_color(rgb(theme::strong())))
                        .on_click(cx.listener(|this, _, window, cx| this.login_back(window, cx)))
                        .child(theme::icon("arrow-left", 16., theme::muted()))
                        .child("Back"),
                ),
            )
        })
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

fn restart_qr(
    entity: &WeakEntity<DiscordApp>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = entity.clone();
    move |_, window, cx| {
        entity
            .update(cx, |app, cx| app.restart_qr_login(window, cx))
            .ok();
    }
}

fn qr_panel(state: &QrState, entity: WeakEntity<DiscordApp>, body_color: u32, cx: &App) -> Div {
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
                        .on_click(restart_qr(&entity)),
                )
                .into_any_element(),
        ),
        QrState::Captcha(url) => {
            let url = url.clone();
            (
                "Are you human?".into(),
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child("Discord wants a captcha first. Solve it in your browser to finish logging in.")
                    .child(
                        link("qr-captcha", "Open it again")
                            .mt(rpx(8.))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                    .child(
                        link("qr-captcha-restart", "Start over")
                            .mt(rpx(4.))
                            .on_click(restart_qr(&entity)),
                    )
                    .into_any_element(),
            )
        }
        QrState::Failed(message) => (
            "Log in with QR Code".into(),
            div()
                .flex()
                .flex_col()
                .items_center()
                .child(message.clone())
                .child(
                    link("qr-retry", "Try again")
                        .mt(rpx(8.))
                        .on_click(restart_qr(&entity)),
                )
                .into_any_element(),
        ),
        QrState::Loading | QrState::Code(_) => {
            const TEXT: &str = "Scan this with the Discord mobile app to log in instantly.";
            const BOLD: &str = "Discord mobile app";
            let start = TEXT.find(BOLD).unwrap_or_default();
            (
                "Log in with QR Code".into(),
                StyledText::new(TEXT)
                    .with_highlights([(
                        start..start + BOLD.len(),
                        HighlightStyle {
                            font_weight: Some(FontWeight::SEMIBOLD),
                            color: Some(rgb(theme::strong()).into()),
                            ..Default::default()
                        },
                    )])
                    .into_any_element(),
            )
        }
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
                .line_height(rpx(32.))
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
                .text_color(rgb(body_color))
                .child(body),
        )
}

pub(crate) fn add_account_dialog(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let form = app.accounts.adding.as_ref()?;
    let entity = app.entity.clone();
    let field = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .flex()
                .gap(rpx(4.))
                .mb(rpx(8.))
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .child("Token"),
                )
                .child(div().text_color(rgb(theme::RED)).child("*")),
        )
        .child(
            div()
                .px(rpx(10.))
                .rounded(rpx(8.))
                .bg(rgba(theme::recessed()))
                .border_1()
                .border_color(if form.error.is_some() {
                    rgb(theme::RED)
                } else {
                    rgba(theme::border())
                })
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
        .children(form.error.clone().map(|error| {
            div()
                .mt(rpx(8.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::RED))
                .child(error)
        }))
        .child(
            div()
                .mt(rpx(8.))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::faint()))
                .child("Stored in your system's credential manager and sent only to Discord."),
        );
    let body = div()
        .flex()
        .items_center()
        .gap(rpx(32.))
        .child(field)
        .children(
            form.qr
                .as_ref()
                .map(|qr| qr_panel(&qr.state, entity.clone(), theme::text(), cx)),
        )
        .into_any_element();
    let back = entity.clone();
    let submit = entity.clone();
    let close = entity;
    let footer = div()
        .flex()
        .items_center()
        .w_full()
        .child(
            modal::text_button("add-account-back", "Back").on_click(move |_, window, cx| {
                cx.stop_propagation();
                back.update(cx, |app, cx| app.add_account_back(window, cx))
                    .ok();
            }),
        )
        .child(div().flex_1())
        .child(
            modal::button("add-account-continue", "Continue", true, !form.busy).when(
                !form.busy,
                |this| {
                    this.on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        submit
                            .update(cx, |app, cx| app.submit_login(window, cx))
                            .ok();
                    })
                },
            ),
        )
        .into_any_element();
    Some(modal::dialog(
        Dialog {
            id: "add-account",
            width: modal::LARGE,
            title: "Add Account",
            subtitle: Some(
                "Logging in to another account will let you easily switch between accounts on this device.",
            ),
            body,
            footer,
        },
        move |window, cx| {
            close
                .update(cx, |app, cx| {
                    app.close_add_account(window, cx);
                })
                .ok();
        },
    ))
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
        QrState::Captcha(_) | QrState::Failed(_) => frame
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
