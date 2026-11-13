mod list;
mod manage;
pub(crate) mod modal;
mod toast;

pub(crate) use list::{ListPlace, account_list, add_account_link};
pub(crate) use manage::manage_dialog;
pub(crate) use toast::toast;

use super::*;
use crate::{
    credentials::CredentialError,
    discord::{Error, model},
    settings::{self, SavedAccount},
};
use login::LoginMode;
use std::sync::atomic::{AtomicBool, Ordering};

const TOAST_DURATION: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(crate) struct Accounts {
    manage_open: bool,
    pub(crate) adding: Option<LoginForm>,
    menu: Option<u64>,
    validating: HashSet<u64>,
    busy: Option<u64>,
    expected: Option<u64>,
    switched: bool,
    toast: Option<(SharedString, bool)>,
    toast_serial: u64,
    toast_timer: Option<Task<()>>,
    retired: Arc<AtomicBool>,
}

impl Accounts {
    pub(crate) fn validating(&self, user_id: u64) -> bool {
        self.validating.contains(&user_id)
    }

    pub(crate) fn busy(&self) -> Option<u64> {
        self.busy
    }

    pub(crate) fn retired(&self) -> SessionRetired {
        SessionRetired(self.retired.clone())
    }
}

#[derive(Clone)]
pub(crate) struct SessionRetired(Arc<AtomicBool>);

impl SessionRetired {
    pub(crate) fn get(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

enum Validation {
    Valid(Option<SavedAccount>),
    Invalid,
}

fn rejects_token(error: &Error) -> bool {
    matches!(
        error,
        Error::Http(401 | 403)
            | Error::InvalidCredential
            | Error::Rejected {
                status: 401 | 403,
                ..
            }
    )
}

fn saved_account(user: &model::User) -> SavedAccount {
    SavedAccount {
        id: user.id,
        username: user.username.clone(),
        discriminator: user.discriminator.clone(),
        avatar: user.avatar.clone(),
        invalid: false,
    }
}

fn clear_credentials(user_ids: impl IntoIterator<Item = u64>) {
    for user_id in user_ids {
        if let Err(error) = credentials::clear(user_id) {
            log::warn!("{error}");
        }
    }
}

impl DiscordApp {
    pub(super) fn begin(&mut self, start: Start, window: &mut Window, cx: &mut Context<Self>) {
        match start {
            Start::Saved => self.load_saved_account(window, cx),
            Start::Login { error } => {
                let form = LoginForm::with_error(error, window, cx);
                self.show_login(form, cx);
            }
            Start::Token { token, user_id } => {
                self.accounts.expected = Some(user_id);
                self.accounts.switched = true;
                self.pending_token = Some(token.clone());
                self.connect(token, window, cx);
            }
        }
    }

    fn load_saved_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = settings::active_account(cx);
        let load = cx.background_executor().spawn(async move {
            if let Some(user_id) = active
                && let Some(token) = credentials::load(user_id)?
            {
                return Ok::<_, CredentialError>(Some((token, Some(user_id))));
            }
            Ok(credentials::load_legacy()?.map(|token| (token, None)))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = load.await;
            this.update_in(cx, |app, window, cx| {
                if !matches!(app.screen, Screen::Connecting)
                    || app.session.is_some()
                    || app.pending_token.is_some()
                {
                    return;
                }
                match result {
                    Ok(Some((token, owner))) => {
                        // Re-save after Ready to persist keyutils-only tokens from older builds.
                        app.accounts.expected = owner;
                        app.pending_token = Some(token.clone());
                        app.connect(token, window, cx);
                    }
                    result => {
                        let error = result.err().map(|error| {
                            log::warn!("{error}");
                            format!("{error}. Your token may not be saved.").into()
                        });
                        let form = LoginForm::with_error(error, window, cx);
                        app.show_login(form, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn show_login(&mut self, form: LoginForm, cx: &mut Context<Self>) {
        let choosing = form.mode == LoginMode::Choose;
        self.screen = Screen::Login(form);
        if choosing {
            self.validate_accounts(cx);
        }
        cx.notify();
    }

    pub(crate) fn current_account_id(&self) -> Option<u64> {
        if self.live {
            Some(self.store.current_user_id)
        } else {
            self.accounts.expected
        }
    }

    pub(crate) fn switchable_accounts(&self, cx: &App) -> Vec<SavedAccount> {
        let mut accounts = settings::accounts(cx);
        let current = self.store.current_user_id;
        if self.live
            && !accounts.iter().any(|account| account.id == current)
            && let Some(user) = self.store.user(current)
        {
            accounts.insert(
                0,
                SavedAccount {
                    id: current,
                    username: user.username.clone(),
                    discriminator: user.discriminator.clone(),
                    avatar: user.avatar.clone(),
                    invalid: false,
                },
            );
        }
        accounts
    }

    pub(super) fn account_ready(&mut self, user: &model::User, cx: &mut Context<Self>) -> bool {
        if let Some(token) = self.pending_token.take()
            && let Err(error) = credentials::save(user.id, &token)
        {
            log::warn!("{error}");
        }
        let account = saved_account(user);
        let mut changed = false;
        let mut dropped = Vec::new();
        settings::update(cx, |settings| {
            changed = settings.activate_account(user.id);
            dropped = settings.remember_account(account);
        });
        clear_credentials(dropped);
        if changed {
            self.channel_rows = guilds::channel_list::Rows::from_settings(&settings::get(cx));
        }
        self.accounts.expected = Some(user.id);
        self.accounts.busy = None;
        let switched = std::mem::take(&mut self.accounts.switched);
        if switched {
            self.show_toast(format!("Switched to {}", user.username), true, cx);
        }
        switched
    }

    pub(super) fn forget_current_account(&mut self, cx: &mut Context<Self>) {
        match self.current_account_id() {
            Some(user_id) => self.forget_account(user_id, cx),
            None => {
                if let Err(error) = credentials::clear_legacy() {
                    log::warn!("{error}");
                }
            }
        }
    }

    pub(crate) fn restart(&mut self, start: Start, window: &mut Window, cx: &mut Context<Self>) {
        self.accounts.retired.store(true, Ordering::Release);
        self.close_stream(Some(window), cx);
        self.leave_voice(cx);
        self.disconnect();
        self.live = false;
        appearance::forget_account_appearance(cx);
        settings::update(cx, |settings| {
            settings.voice_channel = None;
            if let Start::Token { user_id, .. } = &start {
                settings.activate_account(*user_id);
            }
        });
        cx.emit(Restart(start));
    }

    pub(crate) fn log_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.forget_current_account(cx);
        self.restart(Start::Login { error: None }, window, cx);
    }

    fn forget_account(&mut self, user_id: u64, cx: &mut Context<Self>) {
        clear_credentials([user_id]);
        self.accounts.validating.remove(&user_id);
        settings::update(cx, |settings| settings.forget_account(user_id));
    }

    pub(crate) fn log_out_account(
        &mut self,
        user_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.accounts.menu = None;
        if self.live && user_id == self.store.current_user_id {
            self.accounts.manage_open = false;
            self.log_out(window, cx);
            return;
        }
        self.forget_account(user_id, cx);
        if let Screen::Login(form) = &mut self.screen
            && form.mode == LoginMode::Choose
            && settings::accounts(cx).is_empty()
        {
            form.set_mode(LoginMode::Token { back: false }, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn validate_accounts(&mut self, cx: &mut Context<Self>) {
        let runtime = Tokio::handle(cx);
        for account in settings::accounts(cx) {
            let user_id = account.id;
            if !self.accounts.validating.insert(user_id) {
                continue;
            }
            let token = cx
                .background_executor()
                .spawn(async move { credentials::load(user_id) });
            let runtime = runtime.clone();
            cx.spawn(async move |this, cx| {
                let validation = match token.await {
                    Ok(Some(token)) => {
                        Some(match runtime.spawn(discord::token_owner(token)).await {
                            Ok(Ok(user)) if user.id == user_id => {
                                Validation::Valid(Some(saved_account(&user)))
                            }
                            Ok(Ok(_)) => Validation::Invalid,
                            Ok(Err(error)) if rejects_token(&error) => Validation::Invalid,
                            _ => Validation::Valid(None),
                        })
                    }
                    Ok(None) => Some(Validation::Invalid),
                    Err(error) => {
                        log::warn!("{error}");
                        None
                    }
                };
                this.update(cx, |app, cx| app.finish_validation(user_id, validation, cx))
                    .ok();
            })
            .detach();
        }
        cx.notify();
    }

    fn finish_validation(
        &mut self,
        user_id: u64,
        validation: Option<Validation>,
        cx: &mut Context<Self>,
    ) {
        if !self.accounts.validating.remove(&user_id) {
            return;
        }
        match validation {
            Some(Validation::Valid(user)) => settings::update(cx, |settings| {
                if let Some(saved) = settings
                    .accounts
                    .iter_mut()
                    .find(|account| account.id == user_id)
                {
                    *saved = match user {
                        Some(user) => user,
                        None => SavedAccount {
                            invalid: false,
                            ..saved.clone()
                        },
                    };
                }
            }),
            Some(Validation::Invalid) => {
                settings::update(cx, |settings| settings.set_account_invalid(user_id, true))
            }
            None => {}
        }
        cx.notify();
    }

    pub(crate) fn switch_account(
        &mut self,
        user_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.accounts.busy.is_some() || (self.live && user_id == self.store.current_user_id) {
            return;
        }
        let Some(account) = settings::accounts(cx)
            .into_iter()
            .find(|account| account.id == user_id)
        else {
            return;
        };
        self.accounts.menu = None;
        if account.invalid {
            self.reauthenticate(window, cx);
            return;
        }
        self.accounts.busy = Some(user_id);
        if let Screen::Login(form) = &mut self.screen {
            form.busy = true;
            form.error = None;
        }
        cx.notify();
        let load = cx
            .background_executor()
            .spawn(async move { credentials::load(user_id) });
        cx.spawn_in(window, async move |this, cx| {
            let result = load.await;
            this.update_in(cx, |app, window, cx| {
                app.finish_switch(user_id, result, window, cx)
            })
            .ok();
        })
        .detach();
    }

    fn finish_switch(
        &mut self,
        user_id: u64,
        result: Result<Option<String>, CredentialError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.accounts.busy != Some(user_id) {
            return;
        }
        match result {
            Ok(Some(token)) if self.live => {
                self.restart(Start::Token { token, user_id }, window, cx);
            }
            Ok(Some(token)) => {
                self.accounts.expected = Some(user_id);
                self.pending_token = Some(token.clone());
                self.connect(token, window, cx);
            }
            Ok(None) => {
                self.accounts.busy = None;
                settings::update(cx, |settings| settings.set_account_invalid(user_id, true));
                self.reauthenticate(window, cx);
            }
            Err(error) => {
                log::warn!("{error}");
                self.accounts.busy = None;
                match &mut self.screen {
                    Screen::Login(form) => {
                        form.busy = false;
                        form.error = Some(format!("{error}").into());
                        form.set_mode(LoginMode::Token { back: true }, window, cx);
                    }
                    _ => self.show_toast("Switch failed, sorry :(", false, cx),
                }
            }
        }
        cx.notify();
    }

    fn reauthenticate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.live {
            self.close_profile(cx);
            if self.accounts.manage_open {
                self.open_add_account(window, cx);
            } else {
                self.open_manage_accounts(cx);
            }
        } else {
            self.show_token_form(true, window, cx);
        }
    }

    pub(crate) fn open_add_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.accounts.manage_open = false;
        self.accounts.menu = None;
        self.close_profile(cx);
        if self.live {
            self.accounts.adding = Some(LoginForm::with_mode(
                LoginMode::Token { back: false },
                window,
                cx,
            ));
        } else {
            self.show_token_form(true, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn add_account_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.accounts.adding.take().is_some() {
            self.open_manage_accounts(cx);
            self.focus_composer(window, cx);
        }
    }

    pub(crate) fn close_add_account(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.accounts.adding.take().is_none() {
            return false;
        }
        self.focus_composer(window, cx);
        cx.notify();
        true
    }

    pub(crate) fn adding_form_id(&self) -> Option<EntityId> {
        self.accounts
            .adding
            .as_ref()
            .map(|form| form.token.entity_id())
    }

    pub(crate) fn check_added_token(
        &mut self,
        token: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form_id) = self.adding_form_id() else {
            return;
        };
        if let Some(form) = self.accounts.adding.as_mut() {
            form.busy = true;
            form.error = None;
        }
        cx.notify();
        let runtime = Tokio::handle(cx);
        cx.spawn_in(window, async move |this, cx| {
            let owner = runtime.spawn(discord::token_owner(token.clone())).await;
            this.update_in(cx, |app, window, cx| match owner {
                Ok(Ok(user)) => app.add_account(form_id, saved_account(&user), token, window, cx),
                Ok(Err(error)) if rejects_token(&error) => {
                    app.added_token_failed(form_id, "Invalid token".into(), cx)
                }
                Ok(Err(error)) => app.added_token_failed(form_id, error.to_string().into(), cx),
                Err(_) => app.added_token_failed(form_id, "Couldn't reach Discord".into(), cx),
            })
            .ok();
        })
        .detach();
    }

    fn added_token_failed(
        &mut self,
        form_id: EntityId,
        error: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.adding_form_id() != Some(form_id) {
            return;
        }
        if let Some(form) = self.accounts.adding.as_mut() {
            form.busy = false;
            form.error = Some(error);
            cx.notify();
        }
    }

    pub(crate) fn add_account(
        &mut self,
        form_id: EntityId,
        account: SavedAccount,
        token: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.adding_form_id() != Some(form_id) {
            return;
        }
        if let Err(error) = credentials::save(account.id, &token) {
            log::warn!("{error}");
        }
        let user_id = account.id;
        let mut dropped = Vec::new();
        settings::update(cx, |settings| dropped = settings.remember_account(account));
        clear_credentials(dropped);
        if self.live && user_id == self.store.current_user_id {
            self.close_add_account(window, cx);
            return;
        }
        self.restart(Start::Token { token, user_id }, window, cx);
    }

    pub(crate) fn open_manage_accounts(&mut self, cx: &mut Context<Self>) {
        self.close_profile(cx);
        self.accounts.manage_open = true;
        self.accounts.menu = None;
        self.validate_accounts(cx);
        cx.notify();
    }

    pub(crate) fn close_manage_accounts(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.accounts.manage_open {
            return false;
        }
        self.accounts.manage_open = false;
        self.accounts.menu = None;
        cx.notify();
        true
    }

    pub(crate) fn close_account_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.accounts.menu.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn toggle_account_menu(&mut self, user_id: u64, cx: &mut Context<Self>) {
        self.accounts.menu = (self.accounts.menu != Some(user_id)).then_some(user_id);
        cx.notify();
    }

    pub(crate) fn show_toast(
        &mut self,
        text: impl Into<SharedString>,
        success: bool,
        cx: &mut Context<Self>,
    ) {
        self.accounts.toast = Some((text.into(), success));
        self.accounts.toast_serial += 1;
        self.accounts.toast_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_DURATION).await;
            this.update(cx, |app, cx| {
                app.accounts.toast = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}
