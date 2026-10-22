mod channel_list;
mod chat;
mod guild_rail;
mod login;
mod member_list;
mod message_body;
mod rich_text;
mod titlebar;

use crate::discord::{Event, FatalError, Session};
use login::LoginForm;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    rc::Rc,
};

use futures::StreamExt;
use gpui::{prelude::*, *};
use gpui_component::input::{InputEvent, InputState};
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
const OLDER_MESSAGES_THRESHOLD: usize = 8;

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
    pub(crate) composer: Entity<InputState>,
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
    pub(crate) entity: WeakEntity<Self>,
    parsed: RefCell<HashMap<u64, Rc<Parsed>>>,
    _events: Option<Task<()>>,
    _composer_subscription: Subscription,
}

impl DiscordApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| InputState::new(window, cx));
        let messages = ListState::new(0, ListAlignment::Bottom, px(800.));
        let subscription = cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.send_message(window, cx);
            }
        });
        let mut app = Self {
            screen: Screen::Connecting,
            store: Store::seed(),
            selected_guild_id: 0,
            selected_channel_id: 0,
            show_members: true,
            composer,
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
            entity: cx.entity().downgrade(),
            parsed: RefCell::new(HashMap::new()),
            _events: None,
            _composer_subscription: subscription,
        };
        cx.observe_window_bounds(window, |_, window, cx| {
            crate::settings::remember_window(window, cx)
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
            if event.visible_range.start < OLDER_MESSAGES_THRESHOLD {
                weak.update(cx, |app, cx| app.fetch_older(cx)).ok();
            }
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
            self.member_rows.reset(0);
            self.member_top = 0;
        }
        self.selected_guild_id = id;
        self.sync_member_rows();
        self.store.mark_guild_read(id);
        self.select_channel(channel_id, window, cx);
    }

    pub(crate) fn select_channel(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel) = self
            .guild()
            .and_then(|guild| guild.channel(id))
            .filter(|channel| channel.kind == ChannelKind::Text)
        else {
            return;
        };
        let count = channel.messages.len();
        let needs_history = self.live && !channel.messages_loaded;
        let placeholder = if self.live {
            format!(
                "Message #{} (sending comes in a later update)",
                channel.name
            )
        } else {
            format!("Message #{}", channel.name)
        };

        let previous = self.selected_channel_id;
        let draft = self.composer.read(cx).value().to_string();
        if draft.is_empty() {
            self.drafts.remove(&previous);
        } else {
            self.drafts.insert(previous, draft);
        }
        let restored = self.drafts.remove(&id).unwrap_or_default();

        self.selected_channel_id = id;
        self.store.mark_channel_read(self.selected_guild_id, id);
        self.reset_messages(count);
        self.composer.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            input.set_value(restored, window, cx);
            input.focus(window, cx);
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
            session.fetch_messages(guild_id, channel_id, None).await
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
                .fetch_messages(guild_id, channel_id, Some(oldest))
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.loading_older.remove(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        if messages.len() < MESSAGE_PAGE {
                            app.history_complete.insert(channel_id);
                        }
                        let inserted = app.store.prepend_messages(channel_id, messages);
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
            self.hovered_rail = next;
            cx.notify();
        }
    }

    pub(crate) fn toggle_members(&mut self, cx: &mut Context<Self>) {
        self.show_members = !self.show_members;
        cx.notify();
    }

    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.live {
            return;
        }
        let value = self.composer.read(cx).value();
        let content = value.trim();
        if content.is_empty() {
            return;
        }
        if let Some(index) = self.store.append_message(
            self.selected_guild_id,
            self.selected_channel_id,
            content.into(),
        ) {
            let previous = index.saturating_sub(1);
            self.messages.splice(previous..index, index - previous + 1);
            self.messages.scroll_to_end();
            self.composer
                .update(cx, |input, cx| input.set_value("", window, cx));
            cx.notify();
        }
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
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme::BG))
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
