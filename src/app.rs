mod channel_list;
mod chat;
mod embeds;
mod guild_rail;
mod login;
mod member_list;
mod message_body;
mod profile;
mod rich_text;
mod smooth_scroll;
mod titlebar;

use crate::discord::{self, Event, FatalError, OutgoingMessage, OutgoingReply, Session};
use login::LoginForm;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    rc::Rc,
    time::{Duration, Instant},
};

use futures::StreamExt;
use gpui::{prelude::*, *};
use gpui_component::input::{InputEvent, RopeExt, TextareaState};
use gpui_tokio::Tokio;

use crate::{
    credentials, markdown,
    store::{Change, Channel, ChannelKind, Guild, ReplyState, Store},
    theme::{self, rpx},
};

pub struct Parsed {
    content_hash: u64,
    pub blocks: Vec<markdown::Block>,
    pub jumbo: bool,
}

const PARSED_CACHE_LIMIT: usize = 4096;
const MESSAGE_PAGE: usize = 50;
const OLDER_PAGE: usize = 20;
const OLDER_MESSAGES_THRESHOLD: usize = 8;

pub(crate) const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
const ACK_DELAY: Duration = Duration::from_secs(1);
pub(crate) const MESSAGE_LIMIT: usize = 2000;
const TYPING_RESEND: Duration = Duration::from_secs(9);
const TYPING_DURATION: Duration = Duration::from_secs(10);
pub(crate) const DIALOG_EXIT: Duration = Duration::from_millis(150);

fn ms_since_discord_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
        .saturating_sub(DISCORD_EPOCH_MS)
}

fn now_snowflake() -> u64 {
    ms_since_discord_epoch() << 22
}

fn days_since_discord_epoch() -> u32 {
    ms_since_discord_epoch().div_ceil(86_400_000) as u32
}

actions!(zcord, [JumpToPresent, MarkGuildRead, ComposerUp]);

#[derive(Clone, Copy)]
pub(crate) struct ReplyTarget {
    pub message_id: u64,
    pub author_id: u64,
    pub mention: bool,
}

pub(crate) struct Editing {
    pub message_id: u64,
    pub input: Entity<TextareaState>,
    _subscription: Subscription,
}

pub(crate) struct FailedSend {
    reply: Option<ReplyTarget>,
    pub error: String,
}

pub enum Screen {
    Login(LoginForm),
    Connecting,
    Chat,
}

pub struct DiscordApp {
    pub(crate) screen: Screen,
    pub(crate) store: Store,
    pub(crate) selected_guild_id: u64,
    pub(crate) selected_channel_id: u64,
    pub(crate) show_members: bool,
    pub(crate) composer: Entity<TextareaState>,
    pub(crate) reply: Option<ReplyTarget>,
    pub(crate) editing: Option<Editing>,
    pub(crate) confirm_delete: Option<u64>,
    delete_closing: Option<Task<()>>,
    pub(crate) failed_sends: HashMap<u64, FailedSend>,
    pub(crate) hovered_message: Option<u64>,
    pub(crate) hovered_actions: Option<u64>,
    pub(crate) hovered_member: Option<u64>,
    pub(crate) hovered_account: bool,
    pub(crate) profile: Option<profile::ProfileCard>,
    typing: HashMap<u64, Vec<(u64, Instant)>>,
    typing_timer: Option<Task<()>>,
    typing_sent: Option<(u64, Instant)>,
    last_nonce: u64,
    pub(crate) messages: ListState,
    pub(crate) member_rows: ListState,
    member_subscription: Option<(u64, u64, Vec<(u32, u32)>)>,
    pub(crate) live: bool,
    pub(crate) reconnecting: bool,
    session: Option<Session>,
    pending_token: Option<String>,
    loading: HashSet<u64>,
    loading_older: HashSet<u64>,
    history_complete: HashSet<u64>,
    member_top: usize,
    open_folders: HashSet<u64>,
    drafts: HashMap<u64, String>,
    pub(crate) revealed_spoilers: HashSet<(u64, u16)>,
    pub(crate) hovered_rail: Option<u64>,
    pub(crate) rail_morphs: guild_rail::Morphs,
    pub(crate) entity: WeakEntity<Self>,
    focus_handle: FocusHandle,
    window_active: bool,
    pub(crate) unread_since: Option<u64>,
    ack_timer: Option<Task<()>>,
    parsed: RefCell<HashMap<u64, Rc<Parsed>>>,
    _events: Option<Task<()>>,
    _composer_subscription: Subscription,
}

impl DiscordApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 15)
                .submit_on_enter(true)
        });
        let messages = ListState::new(0, ListAlignment::Bottom, px(800.));
        let subscription =
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => this.send_message(window, cx),
                    InputEvent::Change => this.send_typing(cx),
                    _ => {}
                },
            );
        let mut app = Self {
            screen: Screen::Connecting,
            store: Store::seed(),
            selected_guild_id: 0,
            selected_channel_id: 0,
            show_members: true,
            composer,
            reply: None,
            editing: None,
            confirm_delete: None,
            delete_closing: None,
            failed_sends: HashMap::new(),
            hovered_message: None,
            hovered_actions: None,
            hovered_member: None,
            hovered_account: false,
            profile: None,
            typing: HashMap::new(),
            typing_timer: None,
            typing_sent: None,
            last_nonce: 0,
            messages,
            member_rows: ListState::new(0, ListAlignment::Top, px(400.)),
            member_subscription: None,
            live: false,
            reconnecting: false,
            session: None,
            pending_token: None,
            loading: HashSet::new(),
            loading_older: HashSet::new(),
            history_complete: HashSet::new(),
            member_top: 0,
            open_folders: HashSet::new(),
            drafts: HashMap::new(),
            revealed_spoilers: HashSet::new(),
            hovered_rail: None,
            rail_morphs: guild_rail::Morphs::default(),
            entity: cx.entity().downgrade(),
            focus_handle: cx.focus_handle(),
            window_active: window.is_window_active(),
            unread_since: None,
            ack_timer: None,
            parsed: RefCell::new(HashMap::new()),
            _events: None,
            _composer_subscription: subscription,
        };
        cx.on_focus(&app.focus_handle, window, |app, window, cx| {
            if matches!(app.screen, Screen::Chat) {
                app.composer.update(cx, |input, cx| input.focus(window, cx));
            }
        })
        .detach();
        cx.observe_window_bounds(window, |_, window, cx| {
            crate::settings::remember_window(window, cx)
        })
        .detach();
        cx.observe_window_activation(window, |app, window, cx| {
            app.window_active = window.is_window_active();
            app.schedule_ack(cx);
        })
        .detach();
        let weak = app.entity.clone();
        app.member_rows.set_scroll_handler(move |event, _, cx| {
            let top = event.visible_range.start;
            weak.update(cx, |app, _| {
                app.member_top = top;
                app.update_member_subscription();
            })
            .ok();
        });
        let weak = app.entity.clone();
        app.messages.set_scroll_handler(move |event, _, cx| {
            weak.update(cx, |app, cx| {
                if event.visible_range.start < OLDER_MESSAGES_THRESHOLD {
                    app.fetch_older(cx);
                }
                app.schedule_ack(cx);
                if app.unread_since.is_some() {
                    cx.notify();
                }
            })
            .ok();
        });
        match credentials::load() {
            Some(token) => app.connect(token, window, cx),
            None => app.screen = Screen::Login(LoginForm::new(window, cx)),
        }
        app
    }

    pub(crate) fn guild(&self) -> Option<&Guild> {
        self.store
            .guilds
            .iter()
            .find(|guild| guild.id == self.selected_guild_id)
    }

    pub(crate) fn channel(&self) -> Option<&Channel> {
        self.guild()?.channel(self.selected_channel_id)
    }

    pub(crate) fn parsed(&self, id: u64, content: &str) -> Rc<Parsed> {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let content_hash = hasher.finish();
        let mut cache = self.parsed.borrow_mut();
        if let Some(parsed) = cache.get(&id).filter(|p| p.content_hash == content_hash) {
            return parsed.clone();
        }
        if cache.len() >= PARSED_CACHE_LIMIT {
            cache.clear();
        }
        let blocks = markdown::parse(content);
        let parsed = Rc::new(Parsed {
            content_hash,
            jumbo: markdown::is_jumbo(&blocks),
            blocks,
        });
        cache.insert(id, parsed.clone());
        parsed
    }

    pub(crate) fn reveal_spoiler(&mut self, message_id: u64, index: u16, cx: &mut Context<Self>) {
        if self.revealed_spoilers.insert((message_id, index)) {
            cx.notify();
        }
    }

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
        form.busy = true;
        form.error = None;
        self.pending_token = Some(token.clone());
        self.connect(token, window, cx);
        cx.notify();
    }

    pub(crate) fn open_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect();
        self.live = false;
        self.store = Store::seed();
        self.screen = Screen::Chat;
        self.select_first_guild(window, cx);
    }

    pub(crate) fn log_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect();
        if let Err(error) = credentials::clear() {
            log::warn!("{error}");
        }
        self.live = false;
        self.store = Store::seed();
        self.screen = Screen::Login(LoginForm::new(window, cx));
        cx.notify();
    }

    fn disconnect(&mut self) {
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
        self._events = None;
        self.pending_token = None;
        self.reconnecting = false;
        self.loading.clear();
        self.loading_older.clear();
        self.history_complete.clear();
        self.member_subscription = None;
    }

    fn connect(&mut self, token: String, window: &mut Window, cx: &mut Context<Self>) {
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

    fn handle_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Fatal(error) => {
                let message: SharedString = match error {
                    FatalError::InvalidToken => {
                        if let Err(error) = credentials::clear() {
                            log::warn!("{error}");
                        }
                        "Invalid token".into()
                    }
                    FatalError::Other(message) => message.into(),
                };
                self.disconnect();
                self.live = false;
                let mut form = LoginForm::new(window, cx);
                form.error = Some(message);
                self.screen = Screen::Login(form);
                cx.notify();
            }
            Event::Reconnecting => {
                self.reconnecting = true;
                cx.notify();
            }
            Event::Resumed => {
                self.reconnecting = false;
                self.member_subscription = None;
                self.typing.clear();
                cx.notify();
            }
            Event::Ready(_) => {
                if let Some(token) = self.pending_token.take()
                    && let Err(error) = credentials::save(&token)
                {
                    log::warn!("{error}");
                }
                self.live = true;
                self.reconnecting = false;
                self.typing.clear();
                let reconnect = matches!(self.screen, Screen::Chat);
                self.member_subscription = None;
                self.store.apply(event);
                self.screen = Screen::Chat;
                if reconnect && self.channel().is_some() {
                    let channel = self.selected_channel_id;
                    self.select_channel(channel, window, cx);
                } else {
                    self.select_first_guild(window, cx);
                }
            }
            event => {
                match &event {
                    Event::TypingStart(typing) => {
                        self.typing_started(typing.channel_id, typing.user_id, cx)
                    }
                    Event::MessageCreate(message) => {
                        self.typing_stopped(message.channel_id, message.author.id, cx)
                    }
                    Event::MessageDelete { id, .. } => self.forget_message(*id, cx),
                    _ => {}
                }
                let change = self.store.apply(event);
                self.apply_change(change, window, cx);
            }
        }
    }

    fn apply_change(&mut self, change: Change, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.channel().map_or(0, |channel| channel.messages.len());
        match change {
            Change::None => return,
            Change::MessageInserted { channel_id, index }
                if channel_id == self.selected_channel_id =>
            {
                self.messages.splice(index..index, 1);
                self.messages
                    .remeasure_items(index.saturating_sub(1)..(index + 2).min(len));
                let own = self
                    .channel()
                    .and_then(|channel| channel.messages.get(index))
                    .is_some_and(|message| message.author_id == self.store.current_user_id);
                if self.unread_since.is_none() && !own && !self.viewing() {
                    self.unread_since = self
                        .channel()
                        .and_then(|channel| channel.messages.get(index.checked_sub(1)?))
                        .map(|message| message.id);
                }
                self.schedule_ack(cx);
            }
            Change::MessageReplaced {
                channel_id,
                removed,
                inserted,
            } if channel_id == self.selected_channel_id => {
                self.messages.splice(removed..removed + 1, 0);
                self.messages.splice(inserted..inserted, 1);
                let start = removed.min(inserted).saturating_sub(1);
                self.messages
                    .remeasure_items(start..(removed.max(inserted) + 2).min(len));
            }
            Change::MessageChanged { channel_id, index }
                if channel_id == self.selected_channel_id =>
            {
                self.messages.remeasure_items(index..index + 1);
            }
            Change::MessageRemoved { channel_id, index }
                if channel_id == self.selected_channel_id =>
            {
                self.messages.splice(index..index + 1, 0);
                self.messages
                    .remeasure_items(index.saturating_sub(1)..(index + 1).min(len));
            }
            Change::Members { guild_id } => {
                if guild_id.is_none_or(|id| id == self.selected_guild_id) {
                    self.sync_member_rows();
                }
            }
            Change::Structure => {
                self.sync_member_rows();
                if self.channel().is_none() {
                    self.select_first_guild(window, cx);
                    return;
                }
                if self.messages.item_count() != len {
                    self.messages.reset(len);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn select_first_guild(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.store.guilds.first().map(|guild| guild.id) {
            Some(id) => self.select_guild(id, window, cx),
            None => {
                self.selected_guild_id = 0;
                self.selected_channel_id = 0;
                self.messages.reset(0);
                cx.notify();
            }
        }
    }

    pub(crate) fn select_guild(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.id == id)
            .and_then(Guild::first_text_channel)
            .map(|channel| channel.id)
        else {
            return;
        };
        if self.selected_guild_id != id {
            if self.hovered_rail != Some(self.selected_guild_id) {
                self.rail_morphs.start(self.selected_guild_id, false);
            }
            self.member_rows.reset(0);
            self.member_top = 0;
        }
        self.selected_guild_id = id;
        self.sync_member_rows();
        if !self.live {
            self.store.mark_guild_read(id);
        }
        self.select_channel(channel_id, window, cx);
    }

    pub(crate) fn select_channel(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.profile = None;
        let Some(channel) = self
            .guild()
            .and_then(|guild| guild.channel(id))
            .filter(|channel| channel.kind == ChannelKind::Text)
        else {
            return;
        };
        let count = channel.messages.len();
        let needs_history = self.live && !channel.messages_loaded;
        let placeholder = if self.store.can_send(id) {
            format!("Message #{}", channel.name)
        } else {
            "You do not have permission to send messages in this channel.".to_owned()
        };

        let previous = self.selected_channel_id;
        let draft = self.composer.read(cx).value().to_string();
        if draft.is_empty() {
            self.drafts.remove(&previous);
        } else {
            self.drafts.insert(previous, draft);
        }
        let restored = self.drafts.remove(&id).unwrap_or_default();

        if previous != id {
            self.reply = None;
            self.editing = None;
            self.confirm_delete = None;
            self.typing_sent = None;
        }
        self.selected_channel_id = id;
        self.unread_since = self
            .store
            .channel_ack(id)
            .and_then(|_| self.store.last_read(id));
        if !self.live {
            self.store.mark_channel_read(self.selected_guild_id, id);
        }
        self.reset_messages(count);
        if self.viewing() {
            self.ack_channel(cx);
        }
        self.composer.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            set_text(input, restored, window, cx);
        });
        if needs_history {
            self.fetch_history(id, cx);
        }
        cx.notify();
    }

    fn reset_messages(&mut self, count: usize) {
        self.messages.reset(count);
        self.messages.set_follow_mode(FollowMode::Tail);
    }

    fn jump_to_present(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Chat) {
            return;
        }
        smooth_scroll::cancel(cx);
        self.messages.set_follow_mode(FollowMode::Tail);
        self.mark_channel_read(cx);
    }

    pub(crate) fn mark_channel_read(&mut self, cx: &mut Context<Self>) {
        self.unread_since = None;
        self.ack_channel(cx);
        cx.notify();
    }

    pub(crate) fn jump_to_unread(&mut self, cx: &mut Context<Self>) {
        let Some(since) = self.unread_since else {
            return;
        };
        let index = self.channel().map_or(0, |channel| {
            channel
                .messages
                .iter()
                .position(|message| message.id > since)
                .unwrap_or(0)
        });
        smooth_scroll::cancel(cx);
        self.messages.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: px(0.),
        });
        cx.notify();
    }

    fn schedule_ack(&mut self, cx: &mut Context<Self>) {
        if self.ack_timer.is_some()
            || !self.live
            || self.store.channel_ack(self.selected_channel_id).is_none()
        {
            return;
        }
        self.ack_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ACK_DELAY).await;
            this.update(cx, |app, cx| {
                app.ack_timer = None;
                if app.viewing() {
                    app.ack_channel(cx);
                }
            })
            .ok();
        }));
    }

    fn viewing(&self) -> bool {
        self.live
            && self.window_active
            && matches!(self.screen, Screen::Chat)
            && self.messages.is_following_tail()
            && self
                .channel()
                .is_some_and(|channel| channel.messages_loaded)
    }

    fn ack_channel(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let channel_id = self.selected_channel_id;
        let Some(message_id) = self.store.channel_ack(channel_id) else {
            return;
        };
        self.store.apply(Event::MessageAck {
            channel_id,
            message_id,
        });
        cx.notify();
        let guild_id = self.selected_guild_id;
        let days = days_since_discord_epoch();
        let request = Tokio::spawn(cx, async move {
            session
                .ack_message(Some(guild_id), channel_id, message_id, days)
                .await
        });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not mark channel read: {error}"),
            Err(error) => log::warn!("Mark read task failed: {error}"),
        })
        .detach();
    }

    fn mark_guild_read(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        if !matches!(self.screen, Screen::Chat) {
            return;
        }
        let guild_id = self.selected_guild_id;
        self.unread_since = None;
        cx.notify();
        let acks = self.store.unread_acks(guild_id, now_snowflake());
        if acks.is_empty() {
            return;
        }
        for &(channel_id, message_id) in &acks {
            self.store.apply(Event::MessageAck {
                channel_id,
                message_id,
            });
        }
        let channel_id = self.selected_channel_id;
        let request = Tokio::spawn(cx, async move {
            session.ack_bulk(Some(guild_id), channel_id, acks).await
        });
        cx.spawn(async move |_, _| match request.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Could not mark server read: {error}"),
            Err(error) => log::warn!("Mark read task failed: {error}"),
        })
        .detach();
    }

    fn fetch_history(&mut self, channel_id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if !self.loading.insert(channel_id) {
            return;
        }
        let guild_id = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.channel(channel_id).is_some())
            .map(|guild| guild.id);
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_messages(guild_id, channel_id, None, MESSAGE_PAGE)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.loading.remove(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        if messages.len() < MESSAGE_PAGE {
                            app.history_complete.insert(channel_id);
                        }
                        app.store.set_messages(channel_id, messages);
                        app.request_missing_members(channel_id);
                        if app.selected_channel_id == channel_id {
                            let count = app.channel().map_or(0, |channel| channel.messages.len());
                            app.reset_messages(count);
                            if app.viewing() {
                                app.ack_channel(cx);
                            }
                        }
                    }
                    Ok(Err(error)) => log::warn!("Could not load messages: {error}"),
                    Err(error) => log::warn!("History task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn fetch_older(&mut self, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(oldest) = self
            .channel()
            .filter(|channel| channel.messages_loaded)
            .and_then(|channel| channel.messages.first())
            .map(|message| message.id)
        else {
            return;
        };
        if self.history_complete.contains(&channel_id) || !self.loading_older.insert(channel_id) {
            return;
        }
        let guild_id = Some(self.selected_guild_id);
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_messages(guild_id, channel_id, Some(oldest), OLDER_PAGE)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.loading_older.remove(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        let complete = messages.len() < OLDER_PAGE;
                        if complete {
                            app.history_complete.insert(channel_id);
                        }
                        let inserted = app.store.prepend_messages(channel_id, messages);
                        if complete && inserted == 0 && app.selected_channel_id == channel_id {
                            app.messages.remeasure_items(0..1);
                        }
                        if inserted > 0 {
                            app.request_missing_members(channel_id);
                            if app.selected_channel_id == channel_id {
                                app.messages.splice(0..0, inserted);
                                app.messages.remeasure_items(inserted..inserted + 1);
                            }
                        }
                    }
                    Ok(Err(error)) => log::warn!("Could not load older messages: {error}"),
                    Err(error) => log::warn!("History task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn request_missing_members(&mut self, channel_id: u64) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(guild) = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.channel(channel_id).is_some())
        else {
            return;
        };
        let guild_id = guild.id;
        let mut ids = Vec::new();
        for message in guild
            .channel(channel_id)
            .map_or(&[][..], |channel| &channel.messages)
        {
            ids.push(message.author_id);
            if let Some(ReplyState::Loaded(reply)) = &message.reply {
                ids.push(reply.author_id);
            }
            collect_mentions(&self.parsed(message.id, &message.content).blocks, &mut ids);
        }
        let missing = self.store.missing_members(guild_id, ids);
        if !missing.is_empty() {
            session.request_members(guild_id, missing);
        }
    }

    fn sync_member_rows(&mut self) {
        let count = self
            .store
            .member_list(self.selected_guild_id)
            .map_or(0, |list| list.rows.len());
        let old = self.member_rows.item_count();
        self.member_rows.splice(0..old, count);
    }

    fn update_member_subscription(&mut self) {
        let Some(session) = self.session.as_ref().filter(|_| self.live) else {
            return;
        };
        if self.channel().is_none() {
            return;
        }
        let top = self.member_top as u32;
        let chunk = top / 100;
        let mut ranges = vec![(0, 99)];
        for page in [chunk, chunk + 1] {
            if page > 0 {
                ranges.push((page * 100, page * 100 + 99));
            }
        }
        let key = (self.selected_guild_id, self.selected_channel_id, ranges);
        if self.member_subscription.as_ref() == Some(&key) {
            return;
        }
        session.subscribe_channel(key.0, key.1, &key.2);
        self.member_subscription = Some(key);
    }

    pub(crate) fn is_loading(&self, channel_id: u64) -> bool {
        self.loading.contains(&channel_id)
    }

    pub(crate) fn is_folder_open(&self, id: u64) -> bool {
        self.open_folders.contains(&id)
    }

    pub(crate) fn toggle_folder(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.open_folders.remove(&id) {
            self.open_folders.insert(id);
        }
        cx.notify();
    }

    pub(crate) fn set_rail_hover(&mut self, key: u64, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(key)
        } else if self.hovered_rail == Some(key) {
            None
        } else {
            return;
        };
        if self.hovered_rail != next {
            let previous = std::mem::replace(&mut self.hovered_rail, next);
            for key in [previous, next].into_iter().flatten() {
                if key != self.selected_guild_id {
                    self.rail_morphs.start(key, next == Some(key));
                }
            }
            cx.notify();
        }
    }

    pub(crate) fn toggle_members(&mut self, cx: &mut Context<Self>) {
        self.show_members = !self.show_members;
        cx.notify();
    }

    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content = self.composer.read(cx).value().trim().to_owned();
        let channel_id = self.selected_channel_id;
        if content.is_empty()
            || content.chars().count() > MESSAGE_LIMIT
            || !self.store.can_send(channel_id)
        {
            return;
        }
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        let reply = self.reply.take();
        self.typing_sent = None;
        self.unread_since = None;
        self.messages.set_follow_mode(FollowMode::Tail);
        if !self.live {
            if let Some(index) =
                self.store
                    .append_message(self.selected_guild_id, channel_id, content)
            {
                let previous = index.saturating_sub(1);
                self.messages.splice(previous..index, index - previous + 1);
                self.messages.scroll_to_end();
            }
            cx.notify();
            return;
        }
        self.dispatch_send(channel_id, content, reply, window, cx);
    }

    fn dispatch_send(
        &mut self,
        channel_id: u64,
        content: String,
        reply: Option<ReplyTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone() else {
            return;
        };
        self.last_nonce = now_snowflake().max(self.last_nonce + 1);
        let nonce = self.last_nonce;
        let change = self.store.queue_message(
            channel_id,
            nonce,
            content.clone(),
            reply.map(|reply| reply.message_id),
        );
        self.apply_change(change, window, cx);
        let guild_id = self.selected_guild_id;
        let outgoing = OutgoingMessage {
            content,
            nonce,
            reply: reply.map(|reply| OutgoingReply {
                message_id: reply.message_id,
                mention: reply.mention,
            }),
        };
        let request = Tokio::spawn(cx, async move {
            session
                .send_message(Some(guild_id), channel_id, outgoing)
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                let error = match result {
                    Ok(Ok(message)) => {
                        let change = app.store.apply(Event::MessageCreate(Box::new(message)));
                        app.apply_change(change, window, cx);
                        return;
                    }
                    Ok(Err(error)) => error,
                    Err(_) => discord::Error::Closed,
                };
                log::warn!("Could not send message: {error}");
                app.failed_sends.insert(
                    nonce,
                    FailedSend {
                        reply,
                        error: send_error_text(&error),
                    },
                );
                let change = app.store.fail_message(channel_id, nonce);
                app.apply_change(change, window, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn retry_message(
        &mut self,
        nonce: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.selected_channel_id;
        let Some(content) = self
            .channel()
            .and_then(|channel| channel.messages.iter().find(|message| message.id == nonce))
            .map(|message| message.content.clone())
        else {
            return;
        };
        let reply = self
            .failed_sends
            .remove(&nonce)
            .and_then(|failed| failed.reply);
        let change = self.store.discard_message(channel_id, nonce);
        self.apply_change(change, window, cx);
        self.dispatch_send(channel_id, content, reply, window, cx);
    }

    pub(crate) fn discard_message(
        &mut self,
        nonce: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.failed_sends.remove(&nonce);
        let change = self.store.discard_message(self.selected_channel_id, nonce);
        self.apply_change(change, window, cx);
    }

    fn send_typing(&mut self, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let now = Instant::now();
        if self.composer.read(cx).value().trim().is_empty()
            || !self.store.can_send(channel_id)
            || self.typing_sent.is_some_and(|(channel, sent)| {
                channel == channel_id && now.duration_since(sent) < TYPING_RESEND
            })
        {
            return;
        }
        self.typing_sent = Some((channel_id, now));
        let guild_id = self.selected_guild_id;
        let request = Tokio::spawn(cx, async move {
            session.trigger_typing(Some(guild_id), channel_id).await
        });
        cx.spawn(async move |_, _| {
            if let Ok(Err(error)) = request.await {
                log::debug!("Could not send typing indicator: {error}");
            }
        })
        .detach();
    }

    fn typing_started(&mut self, channel_id: u64, user_id: u64, cx: &mut Context<Self>) {
        if user_id == self.store.current_user_id {
            return;
        }
        let users = self.typing.entry(channel_id).or_default();
        users.retain(|(id, _)| *id != user_id);
        users.push((user_id, Instant::now() + TYPING_DURATION));
        self.arm_typing_timer(cx);
        if channel_id == self.selected_channel_id {
            cx.notify();
        }
    }

    fn typing_stopped(&mut self, channel_id: u64, user_id: u64, cx: &mut Context<Self>) {
        if let Some(users) = self.typing.get_mut(&channel_id) {
            users.retain(|(id, _)| *id != user_id);
            if channel_id == self.selected_channel_id {
                cx.notify();
            }
        }
    }

    fn arm_typing_timer(&mut self, cx: &mut Context<Self>) {
        let Some(next) = self
            .typing
            .values()
            .flatten()
            .map(|(_, until)| *until)
            .min()
        else {
            self.typing_timer = None;
            return;
        };
        self.typing_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(next.saturating_duration_since(Instant::now()))
                .await;
            this.update(cx, |app, cx| {
                let now = Instant::now();
                for users in app.typing.values_mut() {
                    users.retain(|(_, until)| *until > now);
                }
                app.typing.retain(|_, users| !users.is_empty());
                app.arm_typing_timer(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    pub(crate) fn typing_users(&self) -> &[(u64, Instant)] {
        self.typing
            .get(&self.selected_channel_id)
            .map_or(&[], Vec::as_slice)
    }

    fn message_index(&self, message_id: u64) -> Option<usize> {
        self.channel()?
            .messages
            .binary_search_by_key(&message_id, |message| message.id)
            .ok()
    }

    fn remeasure_message(&self, message_id: u64) {
        if let Some(index) = self.message_index(message_id) {
            self.messages.remeasure_items(index..index + 1);
        }
    }

    fn focus_composer(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(crate) fn start_reply(
        &mut self,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(author_id) = self
            .message_index(message_id)
            .and_then(|index| self.channel()?.messages.get(index))
            .map(|message| message.author_id)
        else {
            return;
        };
        self.reply = Some(ReplyTarget {
            message_id,
            author_id,
            mention: true,
        });
        self.focus_composer(window, cx);
        cx.notify();
    }

    pub(crate) fn toggle_reply_mention(&mut self, cx: &mut Context<Self>) {
        if let Some(reply) = &mut self.reply {
            reply.mention = !reply.mention;
            cx.notify();
        }
    }

    pub(crate) fn cancel_reply(&mut self, cx: &mut Context<Self>) {
        if self.reply.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn start_edit(
        &mut self,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(content) = self
            .message_index(message_id)
            .and_then(|index| self.channel()?.messages.get(index))
            .filter(|message| message.author_id == self.store.current_user_id)
            .map(|message| message.content.clone())
        else {
            return;
        };
        if let Some(previous) = self.editing.take() {
            self.remeasure_message(previous.message_id);
        }
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 20)
                .submit_on_enter(true)
        });
        input.update(cx, |input, cx| set_text(input, content, window, cx));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                move |app, _, event, window, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => app.save_edit(window, cx),
                    InputEvent::Change => app.remeasure_message(message_id),
                    _ => {}
                },
            );
        self.editing = Some(Editing {
            message_id,
            input,
            _subscription: subscription,
        });
        self.remeasure_message(message_id);
        cx.notify();
    }

    pub(crate) fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editing) = self.editing.take() {
            self.remeasure_message(editing.message_id);
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    pub(crate) fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        let message_id = editing.message_id;
        let content = editing.input.read(cx).value().trim().to_owned();
        if content.chars().count() > MESSAGE_LIMIT {
            self.editing = Some(editing);
            return;
        }
        self.remeasure_message(message_id);
        self.focus_composer(window, cx);
        cx.notify();
        if content.is_empty() {
            self.confirm_delete = Some(message_id);
            return;
        }
        let channel_id = self.selected_channel_id;
        let Some(original) = self
            .message_index(message_id)
            .and_then(|index| self.channel()?.messages.get(index))
            .map(|message| message.content.clone())
            .filter(|original| *original != content)
        else {
            return;
        };
        let change = self
            .store
            .edit_message_locally(channel_id, message_id, content.clone());
        self.apply_change(change, window, cx);
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let guild_id = self.selected_guild_id;
        let request = Tokio::spawn(cx, async move {
            session
                .edit_message(Some(guild_id), channel_id, message_id, content)
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                let change = match result {
                    Ok(Ok(update)) => app.store.apply(Event::MessageUpdate(Box::new(update))),
                    Ok(Err(error)) => {
                        log::warn!("Could not edit message: {error}");
                        app.store
                            .edit_message_locally(channel_id, message_id, original)
                    }
                    Err(_) => return,
                };
                app.apply_change(change, window, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn request_delete(
        &mut self,
        message_id: u64,
        confirmed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if confirmed {
            self.delete_message(message_id, window, cx);
        } else {
            self.confirm_delete = Some(message_id);
            self.delete_closing = None;
            cx.notify();
        }
    }

    pub(crate) fn cancel_delete(&mut self, cx: &mut Context<Self>) {
        if self.confirm_delete.is_none() || self.delete_closing.is_some() {
            return;
        }
        self.delete_closing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DIALOG_EXIT).await;
            this.update(cx, |app, cx| {
                app.confirm_delete = None;
                app.delete_closing = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub(crate) fn delete_dialog_closing(&self) -> bool {
        self.delete_closing.is_some()
    }

    fn delete_message(&mut self, message_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_delete(cx);
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            let change = self.store.apply(Event::MessageDelete {
                channel_id,
                id: message_id,
            });
            self.forget_message(message_id, cx);
            self.apply_change(change, window, cx);
            return;
        };
        let guild_id = self.selected_guild_id;
        let request = Tokio::spawn(cx, async move {
            session
                .delete_message(Some(guild_id), channel_id, message_id)
                .await
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| match result {
                Ok(Ok(())) => {
                    app.forget_message(message_id, cx);
                    let change = app.store.apply(Event::MessageDelete {
                        channel_id,
                        id: message_id,
                    });
                    app.apply_change(change, window, cx);
                }
                Ok(Err(error)) => log::warn!("Could not delete message: {error}"),
                Err(_) => {}
            })
            .ok();
        })
        .detach();
    }

    fn forget_message(&mut self, message_id: u64, cx: &mut Context<Self>) {
        if self
            .reply
            .is_some_and(|reply| reply.message_id == message_id)
        {
            self.reply = None;
            cx.notify();
        }
        if self
            .editing
            .as_ref()
            .is_some_and(|editing| editing.message_id == message_id)
        {
            self.editing = None;
            cx.notify();
        }
        if self.confirm_delete == Some(message_id) {
            self.confirm_delete = None;
            cx.notify();
        }
    }

    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_profile(cx) {
            return;
        }
        if self.confirm_delete.is_some() {
            self.cancel_delete(cx);
        } else if self.editing.is_some() {
            self.cancel_edit(window, cx);
        } else if self.reply.is_some() {
            self.cancel_reply(cx);
        } else {
            self.jump_to_present(cx);
        }
    }

    fn composer_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composer = self.composer.read(cx);
        let last_own = self.channel().and_then(|channel| {
            channel
                .messages
                .iter()
                .rev()
                .find(|message| {
                    message.author_id == self.store.current_user_id
                        && message.delivery == crate::store::Delivery::Sent
                        && matches!(message.kind, 0 | 19)
                })
                .map(|message| message.id)
        });
        match last_own {
            Some(message_id)
                if self.editing.is_none()
                    && composer.value().is_empty()
                    && composer.focus_handle(cx).is_focused(window) =>
            {
                self.start_edit(message_id, window, cx);
                self.messages
                    .scroll_to_reveal_item(self.message_index(message_id).unwrap_or_default());
            }
            _ => cx.propagate(),
        }
    }

    pub(crate) fn set_message_hover(
        &mut self,
        message_id: u64,
        hovered: bool,
        actions: bool,
        cx: &mut Context<Self>,
    ) {
        let slot = if actions {
            &mut self.hovered_actions
        } else {
            &mut self.hovered_message
        };
        let next = if hovered {
            Some(message_id)
        } else if *slot == Some(message_id) {
            None
        } else {
            return;
        };
        if *slot != next {
            *slot = next;
            cx.notify();
        }
    }
}

fn set_text(
    input: &mut TextareaState,
    text: String,
    window: &mut Window,
    cx: &mut Context<TextareaState>,
) {
    input.set_value(text, window, cx);
    let end = input.text().offset_to_position(input.text().len());
    input.set_cursor_position(end, window, cx);
}

fn send_error_text(error: &discord::Error) -> String {
    match error {
        discord::Error::Captcha => {
            "Discord asked for a captcha. Send this one from the official client.".into()
        }
        discord::Error::Rejected { message, .. } => message.clone(),
        discord::Error::RateLimited { .. } => "You are sending messages too quickly.".into(),
        error => error.to_string(),
    }
}

fn collect_mentions(blocks: &[markdown::Block], ids: &mut Vec<u64>) {
    use markdown::{Block, SpanKind};
    for block in blocks {
        match block {
            Block::Paragraph(spans) | Block::Subtext(spans) | Block::Heading { spans, .. } => {
                ids.extend(spans.iter().filter_map(|span| match span.kind {
                    SpanKind::UserMention(id) => Some(id),
                    _ => None,
                }));
            }
            Block::Quote(inner) => collect_mentions(inner, ids),
            Block::List { items, .. } => {
                for item in items {
                    collect_mentions(item, ids);
                }
            }
            Block::CodeBlock { .. } => {}
        }
    }
}

impl Render for DiscordApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if matches!(self.screen, Screen::Chat) {
            self.update_member_subscription();
        }
        let body = match &self.screen {
            Screen::Login(form) => login::render(form, cx).into_any_element(),
            Screen::Connecting => connecting().into_any_element(),
            Screen::Chat => div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(guild_rail::render(self, cx))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .rounded_tl(rpx(8.))
                        .border_t_1()
                        .border_l_1()
                        .border_color(rgb(theme::BORDER))
                        .child(channel_list::render(self, cx))
                        .child(chat::render(self, cx)),
                )
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|app, _: &JumpToPresent, window, cx| app.escape(window, cx)))
            .on_action(cx.listener(|app, _: &MarkGuildRead, _, cx| app.mark_guild_read(cx)))
            .on_action(cx.listener(|app, _: &ComposerUp, window, cx| app.composer_up(window, cx)))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme::BG))
            .child(smooth_scroll::interceptor())
            .text_color(rgb(theme::TEXT))
            .text_size(rpx(16.))
            .child(titlebar::render(self, window, cx))
            .when(self.reconnecting, |this| {
                this.child(
                    div()
                        .flex()
                        .justify_center()
                        .py_1()
                        .bg(rgb(theme::YELLOW))
                        .text_size(rpx(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::BG))
                        .child("Connection lost. Reconnecting…"),
                )
            })
            .child(body)
            .children(chat::delete_dialog(self, cx))
            .children(profile::render(self, window, cx))
    }
}

fn connecting() -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .child(theme::icon("discord", 64., theme::BLURPLE))
        .child(
            div()
                .text_size(rpx(14.))
                .text_color(rgb(theme::MUTED))
                .child("Connecting to Discord…"),
        )
}
