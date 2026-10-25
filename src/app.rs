mod channel_list;
mod chat;
mod embeds;
mod emoji_picker;
mod forum;
mod guild_rail;
mod invite_card;
mod lightbox;
mod login;
mod member_list;
mod message_body;
mod profile;
mod rich_text;
mod smooth_scroll;
mod switcher;
mod titlebar;
mod views;

use crate::discord::{self, Event, FatalError, OutgoingMessage, OutgoingReply, Session};
use login::LoginForm;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

use futures::StreamExt;
use gpui::{prelude::*, *};
use gpui_component::input::{InputEvent, RopeExt, TextareaState};
use gpui_tokio::Tokio;

use crate::{
    composer_tokens, credentials, emoji, markdown,
    player::{Fullscreen, Player, VideoDrag},
    store::{
        Change, Channel, EmojiAccess, EmojiChoice, Guild, Invite, InviteState, ReactionEmoji,
        ReplyState, Store,
    },
    theme::{self, rpx},
};

pub struct Parsed {
    content: String,
    pub blocks: Vec<markdown::Block>,
    pub jumbo: bool,
}

#[derive(Default)]
struct ParseCache {
    entries: HashMap<u64, Rc<Parsed>>,
    order: VecDeque<u64>,
}

const PARSED_CACHE_LIMIT: usize = 4096;
const MESSAGE_PAGE: usize = 50;
const OLDER_PAGE: usize = 20;
const OLDER_MESSAGES_THRESHOLD: usize = 8;
const NEWER_MESSAGES_THRESHOLD: usize = 8;

pub(crate) const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
const ACK_DELAY: Duration = Duration::from_secs(1);
const OLDER_RETRY_DELAY: Duration = Duration::from_secs(5);
pub(crate) const MESSAGE_LIMIT: usize = 2000;
const TYPING_RESEND: Duration = Duration::from_secs(9);
const TYPING_DURATION: Duration = Duration::from_secs(10);
const COALESCE_DELAY: Duration = Duration::from_millis(150);
const JUMP_STEP_FRACTION: f32 = 0.2;
/// Targets further than this many viewports are first approached by an instant jump.
const JUMP_GLIDE_VIEWPORTS: f32 = 3.;
pub(crate) const JUMP_HIGHLIGHT: Duration = Duration::from_millis(2000);

struct JumpScroll {
    message_id: u64,
    nonce: u64,
}

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

actions!(
    zcord,
    [
        JumpToPresent,
        MarkGuildRead,
        ComposerUp,
        ComposerDown,
        ComposerTab,
        QuickSwitch
    ]
);

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

#[derive(Default)]
struct ForumPage {
    offset: usize,
    has_more: bool,
    loading: bool,
    loaded: bool,
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
    pub(crate) lightbox: Option<lightbox::Lightbox>,
    pub(crate) emoji_picker: Option<emoji_picker::EmojiPicker>,
    emoji_picker_closing: Option<Task<()>>,
    emoji_picker_animation: u64,
    emoji_picker_closed_at: Option<Instant>,
    pub(crate) emoji_suggest: emoji_picker::Suggest,
    pub(crate) quick_switcher: Option<switcher::QuickSwitcher>,
    quick_switcher_opens: u64,
    recent_channels: crate::quick_switcher::Recent,
    typing: HashMap<u64, Vec<(u64, Instant)>>,
    typing_timer: Option<Task<()>>,
    typing_sent: Option<(u64, Instant)>,
    last_nonce: u64,
    pub(crate) messages: ListState,
    pub(crate) channel_rows: channel_list::Rows,
    pub(crate) member_rows: ListState,
    member_layout: member_list::Layout,
    member_subscription: Option<(u64, u64, Vec<(u32, u32)>)>,
    pub(crate) live: bool,
    pub(crate) reconnecting: bool,
    session: Option<Session>,
    pending_token: Option<String>,
    loading: HashSet<u64>,
    loading_older: HashSet<u64>,
    older_retry_at: HashMap<u64, Instant>,
    loading_newer: HashSet<u64>,
    reaction_requests: HashSet<(u64, u64, String)>,
    /// Channels waiting for a latest-page fetch before jumping to the present.
    pending_present: HashSet<u64>,
    pending_sends: Vec<(u64, String, Option<ReplyTarget>)>,
    history_complete: HashSet<u64>,
    forum_pages: HashMap<u64, ForumPage>,
    pub(crate) invites: HashMap<String, InviteState>,
    member_top: usize,
    open_folders: HashSet<u64>,
    drafts: HashMap<u64, String>,
    pub(crate) revealed_spoilers: HashSet<(u64, u16)>,
    pub(crate) hovered_rail: Option<u64>,
    pub(crate) hovered_add_reaction: Option<u64>,
    pub(crate) hovered_sticker: Option<(u64, u64)>,
    pub(crate) player: Option<Player>,
    pub(crate) video_fullscreen: Option<Fullscreen>,
    jump: Option<JumpScroll>,
    /// Message flashed after a reply jump lands, with a per-jump id to replay the fade.
    pub(crate) jump_highlight: Option<(u64, u64)>,
    pub(crate) hovered_video: Option<u64>,
    video_drag: Option<(u64, VideoDrag)>,
    video_dragged_at: Option<Instant>,
    pub(crate) video_seek_bounds: Rc<RefCell<Option<(u64, Bounds<Pixels>)>>>,
    pub(crate) video_volume_bounds: Rc<RefCell<Option<(u64, Bounds<Pixels>)>>>,
    /// Hover over the active player's mute button and its volume popup.
    video_volume_hover: [bool; 2],
    pub(crate) rail_morphs: guild_rail::Morphs,
    pub(crate) entity: WeakEntity<Self>,
    pub(crate) views: views::PanelViews,
    focus_handle: FocusHandle,
    window_active: bool,
    pub(crate) unread_since: Option<u64>,
    /// Whether the top "new messages" bar is still shown; `unread_since` keeps the divider.
    pub(crate) unread_bar: bool,
    ack_timer: Option<Task<()>>,
    status_expiry: Option<SystemTime>,
    status_expiry_timer: Option<Task<()>>,
    members_timer: Option<Task<()>>,
    parsed: RefCell<ParseCache>,
    _events: Option<Task<()>>,
    _composer_subscription: Subscription,
    _composer_cursor_subscription: Subscription,
}

fn is_background_change(change: &Change, selected_channel_id: u64) -> bool {
    match change {
        Change::MessageInserted { channel_id, .. }
        | Change::MessageChanged { channel_id, .. }
        | Change::MessageRemoved { channel_id, .. }
        | Change::MessageReplaced { channel_id, .. } => *channel_id != selected_channel_id,
        _ => false,
    }
}

impl DiscordApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 15)
                .submit_on_enter(true)
        });
        let messages = ListState::new(0, ListAlignment::Bottom, px(800.));
        let last_cursor = Rc::new(Cell::new(0));
        let cursor_subscription = cx.observe(&composer, move |_this, input, cx| {
            snap_composer_cursor(&input, &last_cursor, cx);
        });
        let subscription =
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => {
                        if !this.accept_emoji_suggestion(None, window, cx) {
                            this.send_message(window, cx);
                        }
                    }
                    InputEvent::Change => {
                        this.repair_composer(window, cx);
                        this.reset_emoji_suggest(cx);
                        this.send_typing(cx);
                    }
                    _ => {}
                },
            );
        let app_entity = cx.entity();
        let views = views::PanelViews::new(&app_entity, cx);
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
            lightbox: None,
            emoji_picker: None,
            emoji_picker_closing: None,
            emoji_picker_animation: 0,
            emoji_picker_closed_at: None,
            emoji_suggest: Default::default(),
            quick_switcher: None,
            quick_switcher_opens: 0,
            recent_channels: Default::default(),
            typing: HashMap::new(),
            typing_timer: None,
            typing_sent: None,
            last_nonce: 0,
            messages,
            channel_rows: channel_list::Rows::with_collapsed(
                crate::settings::get(cx).collapsed_categories,
            ),
            member_rows: ListState::new(0, ListAlignment::Top, px(400.)),
            member_layout: member_list::Layout::default(),
            member_subscription: None,
            live: false,
            reconnecting: false,
            session: None,
            pending_token: None,
            loading: HashSet::new(),
            loading_older: HashSet::new(),
            older_retry_at: HashMap::new(),
            loading_newer: HashSet::new(),
            reaction_requests: HashSet::new(),
            pending_present: HashSet::new(),
            pending_sends: Vec::new(),
            history_complete: HashSet::new(),
            forum_pages: HashMap::new(),
            invites: HashMap::new(),
            member_top: 0,
            open_folders: HashSet::new(),
            drafts: HashMap::new(),
            revealed_spoilers: HashSet::new(),
            hovered_rail: None,
            hovered_add_reaction: None,
            hovered_sticker: None,
            player: None,
            video_fullscreen: None,
            jump: None,
            jump_highlight: None,
            hovered_video: None,
            video_drag: None,
            video_dragged_at: None,
            video_seek_bounds: Rc::new(RefCell::new(None)),
            video_volume_bounds: Rc::new(RefCell::new(None)),
            video_volume_hover: [false; 2],
            rail_morphs: guild_rail::Morphs::default(),
            entity: app_entity.downgrade(),
            views,
            focus_handle: cx.focus_handle(),
            window_active: window.is_window_active(),
            unread_since: None,
            unread_bar: false,
            ack_timer: None,
            status_expiry: None,
            status_expiry_timer: None,
            members_timer: None,
            parsed: RefCell::new(ParseCache::default()),
            _events: None,
            _composer_subscription: subscription,
            _composer_cursor_subscription: cursor_subscription,
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
                // Only user scrolling reaches this handler; it takes over from a reply jump.
                app.jump = None;
                if event.visible_range.start < OLDER_MESSAGES_THRESHOLD {
                    app.fetch_older(cx);
                }
                // GPUI runs this handler while the list's state is mutably borrowed, so any
                // `app.messages` call here panics; read the event and defer list access.
                if event.visible_range.end + NEWER_MESSAGES_THRESHOLD >= event.count {
                    app.fetch_newer(cx);
                }
                if app.live && (!event.is_following_tail || app.unread_bar) {
                    let entity = app.entity.clone();
                    let channel_id = app.selected_channel_id;
                    cx.defer(move |cx| {
                        entity
                            .update(cx, |app, cx| {
                                // `is_scrolled_to_end` is None while off-screen rows are
                                // unmeasured, so compare the logical top with the item count.
                                if app.selected_channel_id != channel_id
                                    || !app.store.at_present(channel_id)
                                    || app.messages.logical_scroll_top().item_ix
                                        < app.messages.item_count()
                                {
                                    return;
                                }
                                if !app.messages.is_following_tail() {
                                    app.messages.set_follow_mode(FollowMode::Tail);
                                }
                                if app.unread_bar {
                                    app.unread_bar = false;
                                    app.views.messages.update(cx, |_, cx| cx.notify());
                                }
                                app.schedule_ack(cx);
                            })
                            .ok();
                    });
                }
                app.schedule_ack(cx);
                if app.unread_since.is_some() {
                    app.views.messages.update(cx, |_, cx| cx.notify());
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

    pub(crate) fn selected_forum(&self) -> Option<u64> {
        self.channel()
            .filter(|channel| channel.kind.is_forum())
            .map(|channel| channel.id)
    }

    pub(crate) fn thread_parent(&self) -> Option<u64> {
        let id = self.selected_channel_id;
        self.guild()?
            .threads
            .iter()
            .find(|thread| thread.id == id)
            .and_then(|thread| thread.parent_id)
    }

    pub(crate) fn open_thread(
        &mut self,
        thread_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_channel(thread_id, window, cx);
    }

    pub(crate) fn back_to_forum(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(forum_id) = self.thread_parent() {
            self.select_channel(forum_id, window, cx);
        }
    }

    pub(crate) fn parsed(&self, id: u64, content: &str) -> Rc<Parsed> {
        let mut cache = self.parsed.borrow_mut();
        if let Some(parsed) = cache.entries.get(&id).filter(|p| p.content == content) {
            return parsed.clone();
        }
        if !cache.entries.contains_key(&id) && cache.entries.len() >= PARSED_CACHE_LIMIT {
            let keep = PARSED_CACHE_LIMIT * 3 / 4;
            while cache.entries.len() > keep {
                let Some(oldest) = cache.order.pop_front() else {
                    break;
                };
                cache.entries.remove(&oldest);
            }
        }
        let blocks = markdown::parse(content);
        let parsed = Rc::new(Parsed {
            content: content.to_owned(),
            jumbo: markdown::is_jumbo(&blocks),
            blocks,
        });
        if cache.entries.insert(id, parsed.clone()).is_none() {
            cache.order.push_back(id);
        }
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
                self.arm_status_expiry_timer(cx);
                self.sync_channel_rows();
                self.screen = Screen::Chat;
                if reconnect && self.channel().is_some() {
                    let channel = self.selected_channel_id;
                    self.select_channel(channel, window, cx);
                } else {
                    self.select_initial(window, cx);
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
        self.arm_status_expiry_timer(cx);
        // Background channels only change badges, which nobody can see until a
        // repaint; coalesce a burst into one refresh with the member sync timer.
        if is_background_change(&change, self.selected_channel_id) {
            self.arm_members_timer(cx);
            return;
        }
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
                    self.unread_bar = self.unread_since.is_some();
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
                self.stop_player(Some(window), cx);
                self.messages.splice(index..index + 1, 0);
                self.messages
                    .remeasure_items(index.saturating_sub(1)..(index + 1).min(len));
            }
            Change::Members { guild_id } => {
                if guild_id.is_none_or(|id| id == self.selected_guild_id) {
                    self.arm_members_timer(cx);
                }
                return;
            }
            Change::Structure => {
                self.sync_channel_rows();
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

    fn select_initial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = crate::settings::get(cx).last_channel;
        let valid = saved.is_some_and(|saved| {
            self.store.guilds.iter().any(|guild| {
                guild.id == saved.guild_id
                    && guild
                        .channel(saved.channel_id)
                        .is_some_and(|channel| channel.kind.is_selectable())
            })
        });
        match saved.filter(|_| valid) {
            Some(saved) => {
                self.select_guild_channel(saved.guild_id, Some(saved.channel_id), window, cx)
            }
            None => self.select_first_guild(window, cx),
        }
    }

    fn select_first_guild(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.store.guilds.first().map(|guild| guild.id) {
            Some(id) => self.select_guild(id, window, cx),
            None => {
                self.selected_guild_id = 0;
                self.selected_channel_id = 0;
                self.sync_channel_rows();
                self.messages.reset(0);
                cx.notify();
            }
        }
    }

    pub(crate) fn select_guild(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let saved = crate::settings::get(cx).guild_channels.get(&id).copied();
        self.select_guild_channel(id, saved, window, cx);
    }

    fn select_guild_channel(
        &mut self,
        id: u64,
        preferred: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(guild) = self.store.guilds.iter().find(|guild| guild.id == id) else {
            return;
        };
        let channel_id = preferred
            .and_then(|channel_id| guild.channel(channel_id))
            .filter(|channel| channel.kind.is_selectable())
            .or_else(|| guild.first_text_channel())
            .or_else(|| guild.first_selectable_channel())
            .map(|channel| channel.id);
        let Some(channel_id) = channel_id else {
            return;
        };
        if self.selected_guild_id != id {
            if self.hovered_rail != Some(self.selected_guild_id) {
                self.rail_morphs.start(self.selected_guild_id, false);
            }
            self.member_rows.reset(0);
            self.member_layout = member_list::Layout::default();
            self.member_top = 0;
        }
        self.selected_guild_id = id;
        self.sync_channel_rows();
        self.sync_member_rows();
        if !self.live {
            self.store.mark_guild_read(id);
        }
        self.select_channel(channel_id, window, cx);
    }

    pub(crate) fn select_channel(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.profile = None;
        let Some((name, kind, count, messages_loaded, parent_id, is_thread)) =
            self.guild().and_then(|guild| {
                let channel = guild.channel(id)?;
                Some((
                    channel.name.clone(),
                    channel.kind,
                    channel.messages.len(),
                    channel.messages_loaded,
                    channel.parent_id,
                    guild.threads.iter().any(|thread| thread.id == id),
                ))
            })
        else {
            return;
        };
        if !kind.is_selectable() {
            return;
        }
        let forum = kind.is_forum();
        let needs_history = self.live && !forum && !messages_loaded;
        let placeholder = if forum {
            "Select a post".to_owned()
        } else if self.store.can_send(id) {
            if is_thread {
                format!("Message {name}")
            } else {
                format!("Message #{name}")
            }
        } else {
            "You do not have permission to send messages in this channel.".to_owned()
        };
        let saved_id = if is_thread {
            parent_id.unwrap_or(id)
        } else {
            id
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
            self.stop_player(Some(window), cx);
            self.close_lightbox(cx);
        }
        self.selected_channel_id = id;
        self.recent_channels.visit(id);
        self.sync_channel_rows();
        if forum {
            self.unread_since = None;
            self.unread_bar = false;
        } else {
            self.unread_since = self
                .store
                .channel_ack(id)
                .and_then(|_| self.store.last_read(id));
            self.unread_bar = self.unread_since.is_some();
        }
        let unread_since = self.unread_since;
        if !self.live {
            self.store.mark_channel_read(self.selected_guild_id, id);
        } else {
            let guild_id = self.selected_guild_id;
            crate::settings::update(cx, |settings| {
                settings.last_channel = Some(crate::settings::SavedChannel {
                    guild_id,
                    channel_id: saved_id,
                });
                settings.guild_channels.insert(guild_id, saved_id);
            });
        }
        self.reset_messages(if forum { 0 } else { count });
        self.composer.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            set_text(input, restored, window, cx);
        });
        if forum {
            self.load_forum(id, cx);
            cx.notify();
            return;
        }
        if self.viewing() {
            self.ack_channel(cx);
        }
        if needs_history {
            self.fetch_history(id, unread_since.filter(|since| *since > 0), window, cx);
        } else if self.live
            && let Some(since) = unread_since.filter(|since| *since > 0)
        {
            let (oldest, inside) = self.channel().map_or((None, false), |channel| {
                (
                    channel.messages.first().map(|message| message.id),
                    channel.messages.iter().any(|message| message.id > since),
                )
            });
            if !inside || oldest.is_some_and(|oldest| since < oldest) {
                self.fetch_history(id, Some(since), window, cx);
            } else {
                self.messages.set_follow_mode(FollowMode::Normal);
                self.scroll_to_unread(since);
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_category(&mut self, id: u64, cx: &mut Context<Self>) {
        if id == 0 {
            return;
        }
        let collapsed = self.channel_rows.toggle_collapsed(id);
        crate::settings::update(cx, |settings| {
            if collapsed {
                settings.collapsed_categories.insert(id);
            } else {
                settings.collapsed_categories.remove(&id);
            }
        });
        self.sync_channel_rows();
        cx.notify();
        if collapsed {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(channel_list::FOLD).await;
                this.update(cx, |app, cx| {
                    app.sync_channel_rows();
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    pub(crate) fn forum_loading(&self) -> bool {
        self.selected_forum()
            .and_then(|forum_id| self.forum_pages.get(&forum_id))
            .is_some_and(|page| page.loading)
    }

    pub(crate) fn forum_has_more(&self) -> bool {
        self.selected_forum()
            .and_then(|forum_id| self.forum_pages.get(&forum_id))
            .is_some_and(|page| page.has_more)
    }

    pub(crate) fn load_forum(&mut self, forum_id: u64, cx: &mut Context<Self>) {
        self.forum_pages.entry(forum_id).or_default();
        let page = self.forum_pages.get(&forum_id).expect("forum page");
        if page.loaded || page.loading {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            self.forum_pages.entry(forum_id).or_default().loaded = true;
            return;
        };
        self.forum_pages.entry(forum_id).or_default().loading = true;
        let guild_id = self.selected_guild_id;
        let request = Tokio::spawn(cx, async move {
            session.fetch_forum_posts(Some(guild_id), forum_id, 0).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let page = app.forum_pages.entry(forum_id).or_default();
                page.loading = false;
                match result {
                    Ok(Ok(search)) => {
                        page.loaded = true;
                        page.offset = search.threads.len();
                        page.has_more = search.has_more;
                        let change = app.store.apply_forum_search(forum_id, search);
                        if matches!(change, Change::Structure) {
                            app.sync_channel_rows();
                        }
                    }
                    Ok(Err(error)) => log::warn!("Could not load forum posts: {error}"),
                    Err(error) => log::warn!("Forum task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn load_more_forum(&mut self, cx: &mut Context<Self>) {
        let Some(forum_id) = self.selected_forum() else {
            return;
        };
        let Some(page) = self.forum_pages.get(&forum_id) else {
            return;
        };
        if !page.loaded || !page.has_more || page.loading {
            return;
        }
        let offset = page.offset;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.forum_pages.entry(forum_id).or_default().loading = true;
        let guild_id = self.selected_guild_id;
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_forum_posts(Some(guild_id), forum_id, offset)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let page = app.forum_pages.entry(forum_id).or_default();
                page.loading = false;
                match result {
                    Ok(Ok(search)) => {
                        page.offset = offset + search.threads.len();
                        page.has_more = search.has_more;
                        let change = app.store.apply_forum_search(forum_id, search);
                        if matches!(change, Change::Structure) {
                            app.sync_channel_rows();
                        }
                    }
                    Ok(Err(error)) => log::warn!("Could not load more forum posts: {error}"),
                    Err(error) => log::warn!("Forum task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn invite(&self, code: &str) -> InviteState {
        self.invites
            .get(code)
            .cloned()
            .unwrap_or(InviteState::Loading)
    }

    pub(crate) fn load_invite(&mut self, code: &str, cx: &mut Context<Self>) {
        if self.invites.contains_key(code) {
            return;
        }
        // Offline: leave the code unknown so a later render retries.
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.invites.insert(code.to_owned(), InviteState::Loading);
        let code_owned = code.to_owned();
        let task_code = code_owned.clone();
        let request = Tokio::spawn(cx, async move {
            let invite = match session.invite(&task_code).await {
                Ok(invite) => invite,
                Err(error) => return Err(error),
            };
            let Some(invite) = invite else {
                return Ok(None);
            };
            let game_ids = invite
                .profile
                .as_ref()
                .map(|profile| profile.game_application_ids.as_slice())
                .unwrap_or_default();
            let applications = session
                .public_applications(game_ids)
                .await
                .unwrap_or_default();
            Ok(Invite::from_model(&invite, &applications))
        });
        cx.spawn(async move |this, cx| {
            let state = match request.await {
                Ok(Ok(Some(invite))) => InviteState::Ready(Arc::new(invite)),
                Ok(Ok(None)) => InviteState::Invalid,
                Ok(Err(error)) => {
                    log::warn!("Could not load invite {code_owned}: {error}");
                    InviteState::Invalid
                }
                Err(error) => {
                    log::warn!("Invite task failed for {code_owned}: {error}");
                    InviteState::Invalid
                }
            };
            this.update(cx, |app, cx| {
                app.invites.insert(code_owned, state);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(crate) fn prefetch_invites(&mut self, content: &str, cx: &mut Context<Self>) {
        for code in discord::model::invite_codes(content) {
            self.load_invite(&code, cx);
        }
    }

    pub(crate) fn record_emoji_use(&mut self, choice: &EmojiChoice, cx: &mut Context<Self>) {
        let key = match choice {
            EmojiChoice::Unicode { emoji, .. } => emoji.clone(),
            EmojiChoice::Custom(emoji) => format!("custom:{}", emoji.id),
        };
        crate::settings::update(cx, |settings| {
            settings.recent_emojis.retain(|recent| *recent != key);
            settings.recent_emojis.insert(0, key);
            settings.recent_emojis.truncate(40);
        });
    }

    pub(crate) fn recent_emojis(&self, cx: &App) -> Vec<EmojiChoice> {
        crate::settings::get(cx)
            .recent_emojis
            .iter()
            .filter_map(|key| {
                if let Some(id) = key.strip_prefix("custom:") {
                    let id: u64 = id.parse().ok()?;
                    let emoji = self
                        .store
                        .guilds
                        .iter()
                        .flat_map(|guild| &guild.emojis)
                        .find(|emoji| emoji.id == id)?;
                    return Some(EmojiChoice::Custom(emoji.clone()));
                }
                let shortcode = emoji::shortcode_for_emoji(key)?;
                Some(EmojiChoice::Unicode {
                    emoji: key.clone(),
                    shortcode: shortcode.to_owned(),
                })
            })
            .take(40)
            .collect()
    }

    fn emoji_insert_text(&self, choice: &EmojiChoice) -> Option<String> {
        match choice {
            EmojiChoice::Unicode { emoji, .. } => Some(emoji.clone()),
            EmojiChoice::Custom(custom) => {
                (self
                    .store
                    .emoji_access(custom, self.selected_guild_id, self.selected_channel_id)
                    == EmojiAccess::Usable)
                    .then(|| composer_tokens::token_for(custom.id, &custom.name, custom.animated))
            }
        }
    }

    fn repair_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (text, cursor) = {
            let input = self.composer.read(cx);
            if !input.text().chars().any(composer_tokens::is_token_char) {
                return;
            }
            (input.value(), input.cursor())
        };
        let Some((fixed, removed)) = composer_tokens::repair_with_removed(&text) else {
            return;
        };
        let mut removed_before = 0;
        for range in &removed {
            if range.end <= cursor {
                removed_before += range.len();
            } else if range.start < cursor {
                removed_before += cursor - range.start;
            }
        }
        let cursor = cursor.saturating_sub(removed_before).min(fixed.len());
        self.composer.update(cx, |input, cx| {
            input.set_value(fixed, window, cx);
            let position = input.text().offset_to_position(cursor);
            input.set_cursor_position(position, window, cx);
        });
    }

    pub(crate) fn insert_emoji(
        &mut self,
        choice: &EmojiChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.emoji_insert_text(choice) else {
            return;
        };
        let text = format!("{text} ");
        self.composer.update(cx, |input, cx| {
            input.insert(text, window, cx);
            input.focus(window, cx);
        });
        self.record_emoji_use(choice, cx);
        cx.notify();
    }

    pub(crate) fn replace_emoji_query(
        &mut self,
        query_len: usize,
        choice: &EmojiChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.emoji_insert_text(choice) else {
            return;
        };
        let text = format!("{text} ");
        let range = {
            let input = self.composer.read(cx);
            let value = input.text().to_string();
            let cursor = input.cursor().min(value.len());
            let start = if query_len == 0 {
                cursor
            } else {
                value[..cursor]
                    .char_indices()
                    .rev()
                    .nth(query_len - 1)
                    .map_or(0, |(offset, _)| offset)
            };
            start..cursor
        };
        self.composer.update(cx, |input, cx| {
            input.set_selected_range(range, cx);
            input.replace(text, window, cx);
            input.focus(window, cx);
        });
        self.record_emoji_use(choice, cx);
        cx.notify();
    }

    pub(crate) fn emoji_query(&self, cx: &App) -> Option<String> {
        let input = self.composer.read(cx);
        let value = input.text().to_string();
        let cursor = input.cursor().min(value.len());
        let prefix = &value[..cursor];
        let mut start = cursor;
        let mut count = 0;
        for (offset, c) in prefix.char_indices().rev() {
            if !emoji_query_char(c) {
                break;
            }
            start = offset;
            count += 1;
        }
        if count < 2 {
            return None;
        }
        let (colon, _) = prefix[..start].char_indices().next_back()?;
        if prefix[colon..].chars().next()? != ':' {
            return None;
        }
        if colon > 0 {
            let before = prefix[..colon].chars().next_back()?;
            if !before.is_whitespace() {
                return None;
            }
        }
        Some(prefix[colon + 1..].to_owned())
    }

    fn encode_emoji_text(&self, content: &str) -> (String, Vec<EmojiChoice>) {
        let guild_id = self.selected_guild_id;
        let channel_id = self.selected_channel_id;
        let used = RefCell::new(Vec::new());
        let content = composer_tokens::to_wire(content);
        let encoded = emoji::encode_emoji(&content, |name| {
            let current = self
                .store
                .guild_by_id(guild_id)
                .and_then(|guild| guild.emojis.iter().find(|emoji| emoji.name == name))
                .filter(|emoji| {
                    self.store.emoji_access(emoji, guild_id, channel_id) == EmojiAccess::Usable
                });
            let custom = current.or_else(|| {
                self.store
                    .guilds
                    .iter()
                    .flat_map(|guild| &guild.emojis)
                    .find(|emoji| {
                        emoji.name == name
                            && self.store.emoji_access(emoji, guild_id, channel_id)
                                == EmojiAccess::Usable
                    })
            });
            if let Some(custom) = custom {
                used.borrow_mut().push(EmojiChoice::Custom(custom.clone()));
                return Some(format!(
                    "<{}:{}:{}>",
                    if custom.animated { "a" } else { "" },
                    custom.name,
                    custom.id
                ));
            }
            let value = emoji::shortcode(name)?;
            used.borrow_mut().push(EmojiChoice::Unicode {
                emoji: value.to_owned(),
                shortcode: name.to_owned(),
            });
            Some(value.to_owned())
        });
        (encoded, used.into_inner())
    }

    pub(crate) fn toggle_reaction(
        &mut self,
        message_id: u64,
        emoji: ReactionEmoji,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.selected_channel_id;
        let me = self
            .store
            .reaction_state(channel_id, message_id, &emoji)
            .unwrap_or(false);
        self.set_reaction(message_id, emoji, !me, cx);
    }

    pub(crate) fn react(&mut self, message_id: u64, choice: &EmojiChoice, cx: &mut Context<Self>) {
        let emoji = match choice {
            EmojiChoice::Custom(custom) => {
                if self
                    .store
                    .emoji_access(custom, self.selected_guild_id, self.selected_channel_id)
                    != EmojiAccess::Usable
                {
                    return;
                }
                ReactionEmoji {
                    id: Some(custom.id),
                    name: custom.name.clone(),
                    animated: custom.animated,
                }
            }
            EmojiChoice::Unicode { emoji, .. } => ReactionEmoji {
                id: None,
                name: emoji.clone(),
                animated: false,
            },
        };
        let channel_id = self.selected_channel_id;
        let already = self
            .store
            .reaction_state(channel_id, message_id, &emoji)
            .unwrap_or(false);
        if !already {
            self.set_reaction(message_id, emoji, true, cx);
        }
        self.record_emoji_use(choice, cx);
    }

    fn set_reaction(
        &mut self,
        message_id: u64,
        emoji: ReactionEmoji,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        let channel_id = self.selected_channel_id;
        if self.store.set_reaction(channel_id, message_id, &emoji, add) == Change::None {
            return;
        }
        self.remeasure_message(message_id);
        cx.notify();
        if self.session.is_none() || !self.live {
            return;
        }
        if self
            .reaction_requests
            .contains(&(channel_id, message_id, reaction_emoji_text(&emoji)))
        {
            return;
        }
        let guild_id = Some(self.selected_guild_id);
        self.send_reaction(guild_id, channel_id, message_id, emoji, add, cx);
    }

    fn send_reaction(
        &mut self,
        guild_id: Option<u64>,
        channel_id: u64,
        message_id: u64,
        emoji: ReactionEmoji,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let text = reaction_emoji_text(&emoji);
        self.reaction_requests
            .insert((channel_id, message_id, text.clone()));
        let request = Tokio::spawn(cx, async move {
            if add {
                session
                    .add_reaction(guild_id, channel_id, message_id, &text)
                    .await
            } else {
                session
                    .remove_reaction(guild_id, channel_id, message_id, &text)
                    .await
            }
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.reaction_requests
                    .remove(&(channel_id, message_id, reaction_emoji_text(&emoji)));
                match result {
                    Ok(Ok(())) => {
                        let desired = app
                            .store
                            .reaction_state(channel_id, message_id, &emoji)
                            .unwrap_or(false);
                        if desired != add && app.live && app.session.is_some() {
                            app.send_reaction(
                                guild_id, channel_id, message_id, emoji, desired, cx,
                            );
                        }
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not update reaction: {error}");
                        if app.store.set_reaction(channel_id, message_id, &emoji, !add)
                            != Change::None
                        {
                            app.remeasure_message(message_id);
                            cx.notify();
                        }
                    }
                    Err(error) => log::warn!("Reaction task failed: {error}"),
                }
            })
            .ok();
        })
        .detach();
    }

    fn reset_messages(&mut self, count: usize) {
        self.messages.reset(count);
        self.messages.set_follow_mode(FollowMode::Tail);
    }

    pub(crate) fn jump_to_present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Chat) {
            return;
        }
        smooth_scroll::cancel(cx);
        self.unread_since = None;
        self.unread_bar = false;
        let channel_id = self.selected_channel_id;
        let needs_present = self.live
            && self.channel().is_some_and(|channel| {
                !channel.messages_loaded || !self.store.at_present(channel.id)
            });
        self.messages.set_follow_mode(FollowMode::Tail);
        if needs_present {
            self.pending_present.insert(channel_id);
            self.fetch_history(channel_id, None, window, cx);
        } else {
            self.mark_channel_read(cx);
        }
    }

    pub(crate) fn mark_channel_read(&mut self, cx: &mut Context<Self>) {
        self.unread_since = None;
        self.unread_bar = false;
        self.ack_channel(cx);
        cx.notify();
    }

    fn scroll_to_unread(&self, since: u64) {
        let Some(index) = self.channel().and_then(|channel| {
            channel
                .messages
                .iter()
                .position(|message| message.id > since)
        }) else {
            return;
        };
        self.messages.scroll_to(ListOffset {
            item_ix: index.saturating_sub(1),
            offset_in_item: px(0.),
        });
    }

    pub(crate) fn jump_to_unread(&mut self, cx: &mut Context<Self>) {
        let Some(since) = self.unread_since else {
            return;
        };
        smooth_scroll::cancel(cx);
        self.scroll_to_unread(since);
        cx.notify();
    }

    pub(crate) fn jump_to_message(
        &mut self,
        message_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        smooth_scroll::cancel(cx);
        if let Some(index) = self.message_index(message_id) {
            self.messages.set_follow_mode(FollowMode::Normal);
            if cx.reduce_motion() {
                self.messages.scroll_to_reveal_item(index);
                self.highlight_jump(message_id, cx);
                cx.notify();
                return;
            }
            let nonce = self.jump.as_ref().map_or(0, |jump| jump.nonce) + 1;
            self.jump = Some(JumpScroll { message_id, nonce });
            self.step_jump(nonce, window, cx);
        } else if self.live {
            self.fetch_history(self.selected_channel_id, Some(message_id), window, cx);
        }
    }

    fn list_offset(&self) -> Pixels {
        let top = self.messages.logical_scroll_top();
        self.messages.offset_for_item(top.item_ix) + top.offset_in_item
    }

    /// One frame of the eased scroll toward a jump target, centring it like Discord. The
    /// target is recomputed every frame because rows measured on the way change its offset;
    /// a far target is first approached by a jump to within a viewport.
    fn step_jump(&mut self, nonce: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(jump) = self.jump.as_ref().filter(|jump| jump.nonce == nonce) else {
            return;
        };
        let message_id = jump.message_id;
        let Some(index) = self.message_index(message_id) else {
            self.jump = None;
            return;
        };
        let current = self.list_offset();
        let viewport = self.messages.viewport_bounds().size.height;
        let top = self.messages.offset_for_item(index);
        let height = self.messages.offset_for_item(index + 1) - top;
        // Rows taller than the viewport start at its top rather than being centred off-screen.
        let target = (top - ((viewport - height) / 2.).max(px(16.).min(viewport / 4.))).max(px(0.));
        let distance = target - current;
        let step = if distance.abs() > viewport * JUMP_GLIDE_VIEWPORTS {
            distance - viewport * distance.signum()
        } else if distance.abs() < px(1.) {
            distance
        } else {
            distance * JUMP_STEP_FRACTION
        };
        self.messages.scroll_by(step);
        self.views.messages.update(cx, |_, cx| cx.notify());
        if distance.abs() < px(1.) {
            self.jump = None;
            self.highlight_jump(message_id, cx);
            cx.notify();
            return;
        }
        let entity = self.entity.clone();
        window.on_next_frame(move |window, cx| {
            entity
                .update(cx, |app, cx| app.step_jump(nonce, window, cx))
                .ok();
        });
    }

    fn highlight_jump(&mut self, message_id: u64, cx: &mut Context<Self>) {
        let nonce = self.jump_highlight.map_or(0, |(_, nonce)| nonce) + 1;
        self.jump_highlight = Some((message_id, nonce));
        // Clear it once faded, so scrolling the row back into view doesn't replay the flash.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JUMP_HIGHLIGHT).await;
            this.update(cx, |app, _| {
                if app
                    .jump_highlight
                    .is_some_and(|(_, current)| current == nonce)
                {
                    app.jump_highlight = None;
                }
            })
            .ok();
        })
        .detach();
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
            && self.store.at_present(self.selected_channel_id)
    }

    fn ack_channel(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let channel_id = self.selected_channel_id;
        if !self.store.at_present(channel_id) {
            return;
        }
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
        self.unread_bar = false;
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

    fn fetch_history(
        &mut self,
        channel_id: u64,
        around: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            match around {
                Some(message_id) => {
                    session
                        .fetch_messages_around(guild_id, channel_id, message_id, MESSAGE_PAGE)
                        .await
                }
                None => {
                    session
                        .fetch_messages(guild_id, channel_id, None, MESSAGE_PAGE)
                        .await
                }
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |app, window, cx| {
                app.loading.remove(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        app.history_complete.remove(&channel_id);
                        match around {
                            Some(_) => app.store.set_around_messages(channel_id, messages),
                            None => {
                                if messages.len() < MESSAGE_PAGE {
                                    app.history_complete.insert(channel_id);
                                }
                                app.store.set_messages(channel_id, messages);
                            }
                        }
                        app.request_missing_members(channel_id);
                        if app.selected_channel_id == channel_id {
                            let count = app.channel().map_or(0, |channel| channel.messages.len());
                            app.reset_messages(count);
                            if let Some(since) = around {
                                app.messages.set_follow_mode(FollowMode::Normal);
                                if let Some(index) = app.message_index(since) {
                                    app.messages.scroll_to(ListOffset {
                                        item_ix: index,
                                        offset_in_item: px(0.),
                                    });
                                } else {
                                    app.scroll_to_unread(since);
                                }
                            }
                            if app.viewing() {
                                app.ack_channel(cx);
                            }
                        }
                        if around.is_none() {
                            app.flush_pending_sends(channel_id, window, cx);
                        }
                    }
                    Ok(Err(error)) => log::warn!("Could not load messages: {error}"),
                    Err(error) => log::warn!("History task failed: {error}"),
                }
                if app.pending_present.remove(&channel_id) && around.is_some() {
                    app.fetch_history(channel_id, None, window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn fetch_newer(&mut self, cx: &mut Context<Self>) {
        if !self.live {
            return;
        }
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(newest) = self
            .channel()
            .filter(|channel| channel.messages_loaded)
            .and_then(|channel| channel.messages.last())
            .map(|message| message.id)
        else {
            return;
        };
        if self.store.at_present(channel_id) || !self.loading_newer.insert(channel_id) {
            return;
        }
        let guild_id = Some(self.selected_guild_id);
        let request = Tokio::spawn(cx, async move {
            session
                .fetch_messages_after(guild_id, channel_id, newest, MESSAGE_PAGE)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.loading_newer.remove(&channel_id);
                match result {
                    Ok(Ok(messages)) => {
                        let current_newest = app
                            .store
                            .guilds
                            .iter()
                            .find(|guild| guild.channel(channel_id).is_some())
                            .and_then(|guild| guild.channel(channel_id))
                            .and_then(|channel| channel.messages.last())
                            .map(|message| message.id);
                        if current_newest != Some(newest) {
                            cx.notify();
                            return;
                        }
                        let inserted =
                            app.store
                                .append_messages(channel_id, messages, MESSAGE_PAGE);
                        if inserted > 0 {
                            app.request_missing_members(channel_id);
                            if app.selected_channel_id == channel_id {
                                let len = app.channel().map_or(0, |channel| channel.messages.len());
                                let index = len - inserted;
                                app.messages.splice(index..index, inserted);
                                app.messages
                                    .remeasure_items(index.saturating_sub(1)..(index + 1).min(len));
                            }
                        }
                    }
                    Ok(Err(error)) => log::warn!("Could not load newer messages: {error}"),
                    Err(error) => log::warn!("History task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn flush_pending_sends(
        &mut self,
        channel_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut index = 0;
        while index < self.pending_sends.len() {
            if self.pending_sends[index].0 == channel_id {
                let (channel, content, reply) = self.pending_sends.remove(index);
                self.dispatch_send(channel, content, reply, window, cx);
            } else {
                index += 1;
            }
        }
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
        if self
            .older_retry_at
            .get(&channel_id)
            .is_some_and(|at| Instant::now() < *at)
        {
            return;
        }
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
                        app.older_retry_at.remove(&channel_id);
                        let current_oldest = app
                            .store
                            .guilds
                            .iter()
                            .find(|guild| guild.channel(channel_id).is_some())
                            .and_then(|guild| guild.channel(channel_id))
                            .and_then(|channel| channel.messages.first())
                            .map(|message| message.id);
                        if current_oldest != Some(oldest) {
                            cx.notify();
                            return;
                        }
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
                    Ok(Err(error)) => {
                        app.older_retry_at
                            .insert(channel_id, Instant::now() + OLDER_RETRY_DELAY);
                        log::warn!("Could not load older messages: {error}");
                    }
                    Err(error) => {
                        app.older_retry_at
                            .insert(channel_id, Instant::now() + OLDER_RETRY_DELAY);
                        log::warn!("History task failed: {error}");
                    }
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

    fn sync_channel_rows(&mut self) {
        let guild = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.id == self.selected_guild_id);
        let banner = guild.and_then(|guild| self.store.guild_banner(guild.id));
        let selected = self.thread_parent().unwrap_or(self.selected_channel_id);
        self.channel_rows.sync_with_banner(guild, banner, selected);
    }

    fn sync_member_rows(&mut self) {
        self.member_layout.sync(
            &self.member_rows,
            self.store.member_list(self.selected_guild_id),
        );
    }

    fn arm_status_expiry_timer(&mut self, cx: &mut Context<Self>) {
        let expiry = self
            .store
            .user(self.store.current_user_id)
            .and_then(|user| user.custom_status.as_ref())
            .and_then(|status| status.expires_at);
        if expiry == self.status_expiry {
            return;
        }
        self.status_expiry = expiry;
        let Some(expires_at) = expiry else {
            self.status_expiry_timer = None;
            return;
        };
        self.status_expiry_timer = Some(cx.spawn(async move |this, cx| {
            let delay = expires_at
                .duration_since(SystemTime::now())
                .unwrap_or_default();
            cx.background_executor().timer(delay).await;
            this.update(cx, |app, cx| {
                let id = app.store.current_user_id;
                let Some(user) = app.store.users.get_mut(&id) else {
                    return;
                };
                let same = user
                    .custom_status
                    .as_ref()
                    .is_some_and(|status| status.expires_at == Some(expires_at));
                if !same {
                    return;
                }
                if !user
                    .custom_status
                    .as_ref()
                    .is_some_and(|status| status.is_expired(SystemTime::now()))
                {
                    app.status_expiry = None;
                    app.arm_status_expiry_timer(cx);
                    return;
                }
                user.custom_status = None;
                user.activity = None;
                user.playing = false;
                app.status_expiry = None;
                app.status_expiry_timer = None;
                cx.notify();
            })
            .ok();
        }));
    }

    fn arm_members_timer(&mut self, cx: &mut Context<Self>) {
        if self.members_timer.is_some() {
            return;
        }
        self.members_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COALESCE_DELAY).await;
            this.update(cx, |app, cx| {
                app.members_timer = None;
                app.sync_member_rows();
                cx.notify();
            })
            .ok();
        }));
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

    pub(crate) fn set_add_reaction_hover(
        &mut self,
        message_id: u64,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let next = if hovered {
            Some(message_id)
        } else if self.hovered_add_reaction == Some(message_id) {
            None
        } else {
            return;
        };
        if self.hovered_add_reaction != next {
            self.hovered_add_reaction = next;
            cx.notify();
        }
    }

    pub(crate) fn set_sticker_hover(
        &mut self,
        message_id: u64,
        sticker_id: u64,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (message_id, sticker_id);
        let next = if hovered {
            Some(key)
        } else if self.hovered_sticker == Some(key) {
            None
        } else {
            return;
        };
        if self.hovered_sticker != next {
            self.hovered_sticker = next;
            cx.notify();
        }
    }

    pub(crate) fn set_video_hover(&mut self, key: u64, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(key)
        } else if self.hovered_video == Some(key) {
            None
        } else {
            return;
        };
        if self.hovered_video != next {
            self.hovered_video = next;
            cx.notify();
        }
    }

    fn stop_player(&mut self, mut window: Option<&mut Window>, cx: &mut App) {
        if let Some(fullscreen) = self.video_fullscreen.take()
            && fullscreen.entered_window_fullscreen
            && let Some(window) = window.as_deref_mut()
            && window.is_fullscreen()
        {
            window.toggle_fullscreen();
        }
        self.hovered_video = None;
        self.video_drag = None;
        self.video_dragged_at = None;
        self.video_volume_hover = [false; 2];
        if let Some(player) = self.player.take() {
            for image in player.take_frames() {
                cx.drop_image(image, window.as_deref_mut());
            }
        }
    }

    pub(crate) fn start_video(
        &mut self,
        key: u64,
        url: String,
        width: f32,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_player(Some(window), cx);
        let Some(mut player) = Player::start(
            key,
            url.clone(),
            theme::image_px(width),
            theme::image_px(height),
        ) else {
            cx.open_url(&url);
            return;
        };
        if let Some(audio) = crate::settings::get(cx).video_audio {
            player.restore_audio(audio.volume, audio.muted);
        }
        player.attach(cx);
        self.player = Some(player);
        self.hovered_video = Some(key);
        cx.notify();
    }

    pub(crate) fn enter_video_fullscreen(
        &mut self,
        mut fullscreen: Fullscreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        fullscreen.entered_window_fullscreen = !window.is_fullscreen();
        if fullscreen.entered_window_fullscreen {
            window.toggle_fullscreen();
        }
        self.video_fullscreen = Some(fullscreen);
        cx.notify();
    }

    pub(crate) fn exit_video_fullscreen(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(fullscreen) = self.video_fullscreen.take() else {
            return false;
        };
        if fullscreen.entered_window_fullscreen && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        self.video_volume_hover = [false; 2];
        cx.notify();
        true
    }

    pub(crate) fn video_click(&mut self, key: u64, cx: &mut Context<Self>) {
        if self
            .video_dragged_at
            .take()
            .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
        {
            return;
        }
        self.toggle_video(key, cx);
    }

    pub(crate) fn toggle_video(&mut self, key: u64, cx: &mut Context<Self>) {
        if let Some(player) = self.player.as_ref().filter(|player| player.key == key) {
            player.toggle();
        }
        cx.notify();
    }

    pub(crate) fn toggle_video_mute(&mut self, key: u64, cx: &mut Context<Self>) {
        if let Some(player) = self.player.as_ref().filter(|player| player.key == key) {
            player.toggle_muted();
            remember_video_audio(player, cx);
        }
        cx.notify();
    }

    pub(crate) fn set_video_volume_hover(
        &mut self,
        index: usize,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        if self.video_volume_hover[index] != hovered {
            self.video_volume_hover[index] = hovered;
            cx.notify();
        }
    }

    pub(crate) fn video_volume_open(&self) -> bool {
        self.video_volume_hover.contains(&true)
            || matches!(self.video_drag, Some((_, VideoDrag::Volume)))
    }

    pub(crate) fn begin_video_drag(
        &mut self,
        key: u64,
        drag: VideoDrag,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.video_drag = Some((key, drag));
        self.apply_video_drag(key, drag, position, cx);
    }

    pub(crate) fn drag_video(&mut self, key: u64, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some((drag_key, drag)) = self.video_drag
            && drag_key == key
        {
            self.video_dragged_at = Some(Instant::now());
            self.apply_video_drag(key, drag, position, cx);
        }
    }

    pub(crate) fn end_video_drag(&mut self, key: u64, cx: &mut Context<Self>) {
        if self.video_drag.is_some_and(|(drag_key, _)| drag_key == key) {
            self.video_drag = None;
            cx.notify();
        }
    }

    fn apply_video_drag(
        &mut self,
        key: u64,
        drag: VideoDrag,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let bounds = match drag {
            VideoDrag::Seek => &self.video_seek_bounds,
            VideoDrag::Volume => &self.video_volume_bounds,
        };
        let Some(bounds) = bounds
            .borrow()
            .as_ref()
            .filter(|(bound_key, _)| *bound_key == key)
            .map(|(_, bounds)| *bounds)
        else {
            return;
        };
        let Some(player) = self.player.as_ref().filter(|player| player.key == key) else {
            return;
        };
        match drag {
            VideoDrag::Seek => {
                let width = bounds.size.width.as_f32();
                if width > 0. {
                    let fraction = ((position.x - bounds.origin.x).as_f32() / width).clamp(0., 1.);
                    player.seek(fraction as f64 * player.view().duration);
                }
            }
            VideoDrag::Volume => {
                let height = bounds.size.height.as_f32();
                if height > 0. {
                    let fraction =
                        1. - ((position.y - bounds.origin.y).as_f32() / height).clamp(0., 1.);
                    player.set_volume(fraction as f64);
                    remember_video_audio(player, cx);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_members(&mut self, cx: &mut Context<Self>) {
        self.show_members = !self.show_members;
        cx.notify();
    }

    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Convert first: EM SPACE (the start of a token) is whitespace to `trim`.
        let content = composer_tokens::to_wire(&self.composer.read(cx).value());
        let content = content.trim().to_owned();
        let channel_id = self.selected_channel_id;
        if content.is_empty() || !self.store.can_send(channel_id) {
            return;
        }
        let (content, used_emoji) = self.encode_emoji_text(&content);
        if content.chars().count() > MESSAGE_LIMIT {
            return;
        }
        for choice in &used_emoji {
            self.record_emoji_use(choice, cx);
        }
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        let reply = self.reply.take();
        self.typing_sent = None;
        self.unread_since = None;
        self.unread_bar = false;
        if !self.live {
            self.messages.set_follow_mode(FollowMode::Tail);
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
        if !self.store.at_present(channel_id) {
            self.pending_sends.push((channel_id, content, reply));
            self.jump_to_present(window, cx);
            return;
        }
        self.messages.set_follow_mode(FollowMode::Tail);
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
        let guild_id = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.channel(channel_id).is_some())
            .map_or(self.selected_guild_id, |guild| guild.id);
        self.last_nonce = now_snowflake().max(self.last_nonce + 1);
        let nonce = self.last_nonce;
        let change = self.store.queue_message(
            channel_id,
            nonce,
            content.clone(),
            reply.map(|reply| reply.message_id),
        );
        self.apply_change(change, window, cx);
        let outgoing = OutgoingMessage {
            content,
            nonce,
            reply: reply.map(|reply| OutgoingReply {
                message_id: reply.message_id,
                mention: reply.mention,
            }),
        };
        let send = session.send_message(Some(guild_id), channel_id, outgoing);
        let request = Tokio::spawn(cx, send);
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
        if self.live && !self.store.at_present(channel_id) {
            self.pending_sends.push((channel_id, content, reply));
            self.jump_to_present(window, cx);
            return;
        }
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
        cx.spawn(async move |this, cx| {
            match request.await {
                Ok(Ok(true)) => {}
                Ok(Ok(false)) => {
                    this.update(cx, |app, _| {
                        if app.typing_sent == Some((channel_id, now)) {
                            app.typing_sent = None;
                        }
                    })
                    .ok();
                }
                Ok(Err(error)) => {
                    log::debug!("Could not send typing indicator: {error}");
                }
                Err(_) => {}
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
                let selected = app.selected_channel_id;
                let before = app.typing.get(&selected).cloned();
                for users in app.typing.values_mut() {
                    users.retain(|(_, until)| *until > now);
                }
                app.typing.retain(|_, users| !users.is_empty());
                app.arm_typing_timer(cx);
                if app.typing.get(&selected) != before.as_ref() {
                    cx.notify();
                }
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
            .map(|message| composer_tokens::from_wire(&message.content))
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
        // Convert first: EM SPACE (the start of a token) is whitespace to `trim`.
        let raw = editing.input.read(cx).value().to_string();
        let raw = composer_tokens::repair(&raw).unwrap_or(raw);
        let content = composer_tokens::to_wire(&raw);
        let content = content.trim().to_owned();
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
        if self.close_quick_switcher(window, cx) {
            return;
        }
        if self.exit_video_fullscreen(window, cx) {
            return;
        }
        if self.dismiss_emoji_suggestions(cx) || self.close_emoji_picker(cx) {
            return;
        }
        if self.close_lightbox(cx) {
            return;
        }
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
            self.jump_to_present(window, cx);
        }
    }

    fn composer_focused(&self, window: &Window, cx: &App) -> bool {
        self.composer.read(cx).focus_handle(cx).is_focused(window)
    }

    fn composer_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.move_quick_switcher(-1, cx) {
            return;
        }
        if self.composer_focused(window, cx) && self.move_emoji_suggestion(-1, cx) {
            return;
        }
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

fn emoji_query_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

fn reaction_emoji_text(emoji: &ReactionEmoji) -> String {
    match emoji.id {
        Some(id) => format!("{}:{}", emoji.name, id),
        None => emoji.name.clone(),
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

/// Keeps arrow-key movement from stopping between the invisible characters of a token.
fn snap_composer_cursor(input: &Entity<TextareaState>, previous: &Cell<usize>, cx: &mut App) {
    // Runs on every composer notify, cursor blink included; stay allocation-free unless the
    // cursor moved and the text actually holds tokens.
    let state = input.read(cx);
    let cursor = state.cursor();
    let previous = previous.replace(cursor);
    if previous == cursor
        || !state.selected_range().is_empty()
        || !state.text().chars().any(composer_tokens::is_token_char)
    {
        return;
    }
    let text = state.value();
    let Some((range, _)) = composer_tokens::tokens_in(&text)
        .into_iter()
        .find(|(range, _)| range.start < cursor && cursor < range.end)
    else {
        return;
    };
    let target = snap_target(&range, previous, cursor);
    input.update(cx, |input, cx| input.set_selected_range(target..target, cx));
}

fn snap_target(range: &std::ops::Range<usize>, previous: usize, cursor: usize) -> usize {
    if previous == range.start {
        range.end
    } else if previous == range.end {
        range.start
    } else if cursor - range.start <= range.end - cursor {
        range.start
    } else {
        range.end
    }
}

fn remember_video_audio(player: &Player, cx: &mut App) {
    let view = player.view();
    let audio = crate::settings::VideoAudio {
        volume: view.volume,
        muted: view.muted,
    };
    crate::settings::update(cx, |settings| settings.video_audio = Some(audio));
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
        views::count_render(views::ViewKind::Root);
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
                .child(
                    self.views
                        .guild_rail
                        .clone()
                        .cached(views::guild_rail_style()),
                )
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
                        .child(
                            self.views
                                .channel_list
                                .clone()
                                .cached(views::channel_list_style()),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .child(
                                    self.views
                                        .chat_header
                                        .clone()
                                        .cached(views::chat_header_style()),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_1()
                                        .min_h_0()
                                        .child(
                                            div()
                                                .flex()
                                                .flex_col()
                                                .flex_1()
                                                .min_w_0()
                                                .h_full()
                                                .child(
                                                    self.views
                                                        .messages
                                                        .clone()
                                                        .cached(views::messages_style()),
                                                )
                                                .child(self.views.composer.clone().cached(
                                                    views::composer_style(
                                                        self.views.composer.read(cx).height(),
                                                        self.selected_forum().is_some(),
                                                    ),
                                                )),
                                        )
                                        .when(self.show_members, |this| {
                                            this.child(
                                                self.views
                                                    .members
                                                    .clone()
                                                    .cached(views::member_list_style()),
                                            )
                                        }),
                                ),
                        ),
                )
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|app, _: &JumpToPresent, window, cx| app.escape(window, cx)))
            .on_action(cx.listener(|app, _: &MarkGuildRead, _, cx| app.mark_guild_read(cx)))
            .on_action(cx.listener(|app, _: &ComposerUp, window, cx| app.composer_up(window, cx)))
            .on_action(cx.listener(|app, _: &QuickSwitch, window, cx| {
                app.toggle_quick_switcher(window, cx);
            }))
            .on_action(cx.listener(|app, _: &ComposerDown, window, cx| {
                if !(app.move_quick_switcher(1, cx)
                    || app.composer_focused(window, cx) && app.move_emoji_suggestion(1, cx))
                {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|app, _: &ComposerTab, window, cx| {
                if !(app.composer_focused(window, cx)
                    && app.accept_emoji_suggestion(None, window, cx))
                {
                    cx.propagate();
                }
            }))
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
            .child(self.views.overlays.clone().cached(views::overlays_style()))
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

#[cfg(test)]
mod composer_cursor_tests {
    use super::{is_background_change, snap_composer_cursor, snap_target};
    use crate::composer_tokens;
    use crate::store::Change;
    use gpui::{
        AppContext, Context, Entity, IntoElement, Render, Subscription, TestAppContext, Window,
    };
    use gpui_component::input::{Textarea, TextareaState};
    use std::{cell::Cell, rc::Rc};

    struct SnapComposer {
        input: Entity<TextareaState>,
        previous: Rc<Cell<usize>>,
        _subscription: Subscription,
    }

    impl Render for SnapComposer {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Textarea::new(&self.input)
        }
    }

    #[test]
    fn background_message_changes_are_the_only_ones_coalesced() {
        let selected = 7;
        for (change, background) in [
            (
                Change::MessageInserted {
                    channel_id: 7,
                    index: 0,
                },
                false,
            ),
            (
                Change::MessageInserted {
                    channel_id: 8,
                    index: 0,
                },
                true,
            ),
            (
                Change::MessageChanged {
                    channel_id: 8,
                    index: 2,
                },
                true,
            ),
            (
                Change::MessageRemoved {
                    channel_id: 8,
                    index: 2,
                },
                true,
            ),
            (
                Change::MessageReplaced {
                    channel_id: 8,
                    removed: 1,
                    inserted: 2,
                },
                true,
            ),
            (Change::Structure, false),
            (Change::Members { guild_id: None }, false),
        ] {
            assert_eq!(
                is_background_change(&change, selected),
                background,
                "{change:?}"
            );
        }
    }
    #[gpui::test]
    fn snapping_skips_over_whole_composer_tokens(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let token = composer_tokens::token_for(901, "snap", false);
        let text = format!("a{token}b");
        let before = 1;
        let after = before + token.len();
        let (composer, cx) = cx.add_window_view(move |window, cx| {
            let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 15));
            input.update(cx, |input, cx| {
                input.set_value(text.clone(), window, cx);
                input.set_selected_range(after..after, cx);
                input.focus(window, cx);
            });
            let previous = Rc::new(Cell::new(after));
            let tracked = previous.clone();
            let subscription = cx.observe(&input, move |_this, input, cx| {
                snap_composer_cursor(&input, &tracked, cx);
            });
            SnapComposer {
                input,
                previous,
                _subscription: subscription,
            }
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        cx.simulate_keystrokes("left");
        cx.update(|_, cx| {
            assert_eq!(composer.read(cx).input.read(cx).cursor(), before);
            assert_eq!(composer.read(cx).previous.get(), before);
        });

        cx.simulate_keystrokes("right");
        cx.update(|_, cx| {
            assert_eq!(composer.read(cx).input.read(cx).cursor(), after);
            assert_eq!(composer.read(cx).previous.get(), after);
        });
    }

    #[test]
    fn snap_target_uses_travel_direction_then_nearest_edge() {
        let range = 1..12;
        assert_eq!(snap_target(&range, 1, 8), 12);
        assert_eq!(snap_target(&range, 12, 8), 1);
        assert_eq!(snap_target(&range, 0, 4), 1);
        assert_eq!(snap_target(&range, 0, 8), 12);
    }
}
