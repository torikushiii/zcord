mod activity_feed;
mod activity_popout;
mod appearance;
pub(crate) mod attachments;
mod call_view;
mod calls;
mod channel_list;
mod chat;
mod composer;
mod connection;
mod display_name;
mod dm_actions;
mod dm_menu;
mod dm_panel;
mod embeds;
mod emoji_picker;
mod expression_info;
mod forum;
mod forum_actions;
mod forward_dialog;
mod friends;
mod gif_picker;
mod guild_actions;
mod guild_menu;
mod guild_rail;
mod history;
mod home;
mod hover;
mod invite_card;
mod invites;
mod lightbox;
mod login;
mod member_list;
mod members;
pub(crate) mod mentions;
mod menu;
mod message_body;
mod notifications;
mod profile;
mod reaction_tooltip;
mod reactions_dialog;
mod read_state;
mod rich_text;
mod search;
mod selection;
mod sending;
mod settings_page;
mod smooth_scroll;
mod sticker_picker;
mod stream_view;
mod switcher;
mod titlebar;
mod typing;
mod video;
mod views;
mod voice;

use crate::discord::{
    self, Event, FatalError, OutgoingMessage, OutgoingReply, Session, UploadProgress,
};
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
use gpui_component::slider::SliderState;
use gpui_tokio::Tokio;
use tokio::sync::watch;

use crate::{
    credentials,
    media::player::{Fullscreen, Player, VideoDrag},
    store::{
        Change, Channel, EmojiAccess, EmojiChoice, Guild, Invite, InviteState, ReactionEmoji,
        ReplyState, Store,
    },
    text::composer_tokens,
    text::emoji,
    text::markdown,
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
pub(crate) const JUMP_MISSING: Duration = Duration::from_secs(4);

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
        QuickSwitch,
        FocusSearch
    ]
);

#[derive(Clone)]
pub(crate) struct ReplyTarget {
    pub message_id: u64,
    pub author_id: u64,
    pub webhook_name: Option<SharedString>,
    pub mention: bool,
}

#[derive(Default)]
struct ComposerDraft {
    content: String,
    mentions: HashMap<String, u64>,
}

pub(crate) struct Editing {
    pub message_id: u64,
    pub input: Entity<TextareaState>,
    mentions: HashMap<String, u64>,
    _subscriptions: [Subscription; 2],
}

pub(crate) struct FailedSend {
    reply: Option<ReplyTarget>,
    files: Vec<attachments::StagedFile>,
    /// Activity reply/reaction entry to re-attach when the send is retried.
    entry: Option<Arc<serde_json::Value>>,
    pub error: String,
}

#[derive(Default)]
struct ForumPage {
    offset: usize,
    has_more: bool,
    loading: bool,
    loaded: bool,
    /// Bumped whenever the query changes so a reply to an older query is dropped.
    generation: u64,
    /// The viewer's Sort & View choices; `None` follows the forum's defaults.
    sort: Option<crate::discord::ForumSort>,
    gallery: Option<bool>,
    tags: Vec<u64>,
    match_all: bool,
    menu: bool,
    menu_closed: Option<Instant>,
    /// Posts whose starter message or owner was already requested, so each is asked for once.
    requested: HashSet<u64>,
}

pub enum Screen {
    Login(LoginForm),
    Connecting,
    Chat,
}

pub struct DiscordApp {
    pub(crate) screen: Screen,
    pub(crate) store: Store,
    account_theme: Option<crate::discord::settings_proto::AccountTheme>,
    activity_feed: activity_feed::ActivityFeed,
    activity_ticker: Option<Task<()>>,
    pub(crate) activity_popout: Option<activity_popout::ActivityPopout>,
    pub(crate) activity_card_tops: std::rc::Rc<std::cell::RefCell<HashMap<u64, Pixels>>>,
    pub(crate) selected_guild_id: u64,
    pub(crate) selected_channel_id: u64,
    pub(crate) dm_error: Option<(u64, String)>,
    dm_menu: Option<dm_menu::DmMenu>,
    dm_dialog: Option<dm_menu::DmDialog>,
    guild_menu: Option<guild_menu::GuildMenu>,
    calls: calls::Calls,
    pub(crate) show_members: bool,
    pub(crate) composer: Entity<TextareaState>,
    pub(crate) reply: Option<ReplyTarget>,
    pub(crate) editing: Option<Editing>,
    pub(crate) confirm_delete: Option<u64>,
    delete_closing: Option<Task<()>>,
    pub(crate) failed_sends: HashMap<u64, FailedSend>,
    staged: HashMap<u64, Vec<attachments::StagedFile>>,
    upload_refusal: Option<attachments::Refusal>,
    upload_progress: HashMap<u64, Arc<UploadProgress>>,
    next_staged_id: u64,
    upload_ticking: bool,
    pub(crate) hovered_message: Option<u64>,
    pub(crate) hovered_actions: Option<u64>,
    pub(in crate::app) message_menu: Option<chat::MessageMenu>,
    pub(in crate::app) composer_menu: Option<chat::ComposerMenu>,
    pub(crate) shift_held: bool,
    pub(in crate::app) channel_popout: Option<chat::ChannelPopout>,
    pub(in crate::app) popout_closed: Option<(chat::PopoutKind, Instant)>,
    pub(in crate::app) create_thread: Option<chat::CreateThread>,
    pub(in crate::app) thread_menu: Option<chat::ThreadMenu>,
    pub(in crate::app) thread_pane: Option<chat::ThreadPane>,
    pub(crate) thread_previews: HashSet<u64>,
    pub(crate) sharing_paused: Option<Point<Pixels>>,
    pub(crate) hovered_member: Option<u64>,
    pub(crate) hovered_account: bool,
    pub(crate) profile: Option<profile::ProfileCard>,
    pub(crate) profile_styles: Option<profile::styles::StylesEditor>,
    pub(crate) self_status: profile::StatusChange,
    pub(crate) settings_page: Option<settings_page::SettingsPage>,
    settings_animation: u64,
    pub(crate) lightbox: Option<lightbox::Lightbox>,
    pub(crate) stream_view: Option<stream_view::StreamView>,
    /// `(guild, channel)` to go back to when a stage's chat closes.
    stage_chat: Option<(u64, u64)>,
    affinity_refresh: Option<Task<()>>,
    dm_profile: Option<profile::ProfileCard>,
    message_search: Option<search::MessageSearch>,
    forward_dialog: Option<forward_dialog::ForwardDialog>,
    reactions_dialog: Option<reactions_dialog::ReactionsDialog>,
    reactions_openings: u64,
    stream_nonce: u64,
    pub(crate) stream_members_shown: bool,
    pub(crate) stream_members_toggles: u64,
    pub(crate) stream_volume: Option<(Entity<SliderState>, Subscription)>,
    pub(crate) hovered_stage_control: Option<&'static str>,
    pub(crate) emoji_picker: Option<emoji_picker::EmojiPicker>,
    emoji_picker_closing: Option<Task<()>>,
    emoji_picker_animation: u64,
    emoji_picker_closed_at: Option<Instant>,
    pub(crate) emoji_suggest: emoji_picker::Suggest,
    mention_suggest: mentions::Suggest,
    suggest_scroll: ScrollHandle,
    mention_names: HashMap<String, u64>,
    pub(crate) gifs: gif_picker::Gifs,
    pub(crate) stickers: sticker_picker::Stickers,
    pub(crate) home: home::Home,
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
    pub(crate) voice: voice::Voice,
    pub(crate) reconnecting: bool,
    session: Option<Session>,
    game_enabled: watch::Sender<bool>,
    game_generation: u64,
    pub(crate) current_game_application: Option<u64>,
    pub(crate) current_game: Option<String>,
    pending_token: Option<String>,
    loading: HashSet<u64>,
    loading_older: HashSet<u64>,
    older_retry_at: HashMap<u64, Instant>,
    loading_newer: HashSet<u64>,
    reaction_requests: HashSet<(u64, u64, String)>,
    /// Channels waiting for a latest-page fetch before jumping to the present.
    pending_present: HashSet<u64>,
    pending_jump: Option<u64>,
    pending_sends: sending::PendingSends,
    history_complete: HashSet<u64>,
    forum_pages: HashMap<u64, ForumPage>,
    /// Gallery columns that fit the forum's last laid-out width.
    forum_columns: Rc<Cell<usize>>,
    forum_header: Option<forum_actions::ForumHeader>,
    pub(crate) invites: HashMap<String, InviteState>,
    member_top: usize,
    open_folders: HashSet<u64>,
    drafts: HashMap<u64, ComposerDraft>,
    pub(crate) revealed_spoilers: HashSet<(u64, u16)>,
    pub(crate) hovered_rail: Option<u64>,
    /// The window's height in `rpx` units, as of the last frame, for popups that must fit it.
    pub(crate) window_height: f32,
    /// `rpx` per pixel as of the last frame, to measure laid-out elements in `rpx`.
    pub(crate) rpx_per_pixel: f32,
    pub(crate) hovered_add_reaction: Option<u64>,
    reaction_tips: reaction_tooltip::ReactionTips,
    pub(crate) hovered_upload_button: Option<attachments::UploadButton>,
    pub(crate) expression_info: expression_info::State,
    pub(crate) player: Option<Player>,
    pub(crate) video_fullscreen: Option<Fullscreen>,
    jump: Option<JumpScroll>,
    /// Message flashed after a reply jump lands, with a per-jump id to replay the fade.
    pub(crate) jump_highlight: Option<(u64, u64)>,
    pub(crate) jump_missing: bool,
    jump_missing_nonce: u64,
    pub(crate) hovered_video: Option<u64>,
    video_drag: Option<(u64, VideoDrag)>,
    video_dragged_at: Option<Instant>,
    pub(crate) video_seek_bounds: Rc<RefCell<Option<(u64, Bounds<Pixels>)>>>,
    pub(crate) video_volume_bounds: Rc<RefCell<Option<(u64, Bounds<Pixels>)>>>,
    /// Hover over the active player's mute button and its volume popup.
    video_volume_hover: [bool; 2],
    pub(crate) rail_morphs: guild_rail::Morphs,
    pub(crate) folder_folds: guild_rail::FolderFolds,
    pub(crate) entity: WeakEntity<Self>,
    pub(crate) views: views::PanelViews,
    focus_handle: FocusHandle,
    delete_focus: FocusHandle,
    window_active: bool,
    pub(crate) unread_since: Option<u64>,
    /// Whether the top "new messages" bar is still shown; `unread_since` keeps the divider.
    pub(crate) unread_bar: bool,
    pub(crate) manual_unread: Option<u64>,
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
        attachments::clear_pasted(cx);
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 15)
                .submit_on_enter(true)
                .context_menu(false)
        });
        let messages = ListState::new(0, ListAlignment::Bottom, px(800.));
        let last_cursor = Rc::new(Cell::new(0));
        let cursor_subscription = cx.observe(&composer, move |this, input, cx| {
            snap_composer_cursor(&input, &last_cursor, cx);
            this.reset_mention_suggest(cx);
        });
        let subscription =
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => {
                        if !(this.accept_mention_suggestion(None, window, cx)
                            || this.accept_emoji_suggestion(None, window, cx))
                        {
                            this.send_message(window, cx);
                        }
                    }
                    InputEvent::Change => {
                        this.repair_composer(window, cx);
                        this.reset_emoji_suggest(cx);
                        this.reset_mention_suggest(cx);
                        this.send_typing(cx);
                    }
                    _ => {}
                },
            );
        let (game_enabled, game_enabled_rx) = watch::channel(false);
        let app_entity = cx.entity();
        let views = views::PanelViews::new(&app_entity, cx);
        let app = Self {
            screen: Screen::Connecting,
            store: Store::seed(),
            account_theme: None,
            activity_feed: activity_feed::ActivityFeed::default(),
            activity_ticker: None,
            activity_popout: None,
            activity_card_tops: Default::default(),
            selected_guild_id: 0,
            selected_channel_id: 0,
            dm_error: None,
            dm_menu: None,
            dm_dialog: None,
            guild_menu: None,
            calls: calls::Calls::default(),
            show_members: true,
            composer,
            reply: None,
            editing: None,
            confirm_delete: None,
            delete_closing: None,
            failed_sends: HashMap::new(),
            staged: HashMap::new(),
            upload_refusal: None,
            upload_progress: HashMap::new(),
            next_staged_id: 1,
            upload_ticking: false,
            hovered_message: None,
            hovered_actions: None,
            message_menu: None,
            composer_menu: None,
            shift_held: false,
            channel_popout: None,
            popout_closed: None,
            create_thread: None,
            thread_menu: None,
            thread_pane: None,
            thread_previews: HashSet::new(),
            sharing_paused: None,
            hovered_member: None,
            hovered_account: false,
            profile: None,
            profile_styles: None,
            settings_page: None,
            settings_animation: 0,
            lightbox: None,
            emoji_picker: None,
            emoji_picker_closing: None,
            emoji_picker_animation: 0,
            emoji_picker_closed_at: None,
            emoji_suggest: Default::default(),
            mention_suggest: Default::default(),
            suggest_scroll: ScrollHandle::new(),
            mention_names: HashMap::new(),
            gifs: Default::default(),
            stickers: Default::default(),
            home: Default::default(),
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
            voice: voice::Voice::from_settings(cx),
            reconnecting: false,
            session: None,
            game_enabled,
            game_generation: 0,
            current_game_application: None,
            current_game: None,
            self_status: profile::StatusChange::default(),
            pending_token: None,
            loading: HashSet::new(),
            loading_older: HashSet::new(),
            older_retry_at: HashMap::new(),
            loading_newer: HashSet::new(),
            reaction_requests: HashSet::new(),
            pending_present: HashSet::new(),
            pending_jump: None,
            pending_sends: sending::PendingSends::default(),
            history_complete: HashSet::new(),
            forum_pages: HashMap::new(),
            forum_columns: Rc::new(Cell::new(3)),
            forum_header: None,
            invites: HashMap::new(),
            member_top: 0,
            open_folders: HashSet::new(),
            drafts: HashMap::new(),
            revealed_spoilers: HashSet::new(),
            hovered_rail: None,
            window_height: f32::MAX,
            rpx_per_pixel: 1.,
            hovered_add_reaction: None,
            reaction_tips: Default::default(),
            hovered_upload_button: None,
            stream_view: None,
            stage_chat: None,
            affinity_refresh: None,
            dm_profile: None,
            message_search: None,
            forward_dialog: None,
            reactions_dialog: None,
            reactions_openings: 0,
            stream_nonce: 0,
            stream_members_shown: true,
            stream_members_toggles: 0,
            stream_volume: None,
            hovered_stage_control: None,
            expression_info: expression_info::State::default(),
            player: None,
            video_fullscreen: None,
            jump: None,
            jump_highlight: None,
            jump_missing: false,
            jump_missing_nonce: 0,
            hovered_video: None,
            video_drag: None,
            video_dragged_at: None,
            video_seek_bounds: Rc::new(RefCell::new(None)),
            video_volume_bounds: Rc::new(RefCell::new(None)),
            video_volume_hover: [false; 2],
            rail_morphs: guild_rail::Morphs::default(),
            folder_folds: guild_rail::FolderFolds::default(),
            entity: app_entity.downgrade(),
            views,
            focus_handle: cx.focus_handle(),
            delete_focus: cx.focus_handle(),
            window_active: window.is_window_active(),
            unread_since: None,
            unread_bar: false,
            manual_unread: None,
            ack_timer: None,
            status_expiry: None,
            status_expiry_timer: None,
            members_timer: None,
            parsed: RefCell::new(ParseCache::default()),
            _events: None,
            _composer_subscription: subscription,
            _composer_cursor_subscription: cursor_subscription,
        };
        let mut activations = crate::notify::activations();
        cx.spawn_in(window, async move |this, cx| {
            while let Some((channel_id, message_id)) = activations.recv().await {
                let opened = this.update_in(cx, |app, window, cx| {
                    app.open_notification(channel_id, message_id, window, cx)
                });
                if opened.is_err() {
                    break;
                }
            }
        })
        .detach();
        let games = Tokio::spawn(cx, async move { crate::games::start(game_enabled_rx) });
        let runtime = Tokio::handle(cx);
        cx.spawn(async move |this, cx| {
            let Ok(mut output) = games.await else {
                return;
            };
            while output.changed().await.is_ok() {
                let value = output.borrow().clone();
                let (generation, session) =
                    match this.update(cx, |app, _| (app.game_generation, app.session.clone())) {
                        Ok(state) => state,
                        Err(_) => break,
                    };
                let resolved = match (value, session) {
                    (Some(game), Some(session)) => {
                        let activity = game.activity.clone();
                        let activity = runtime
                            .spawn(async move { session.resolve_activity_images(activity).await })
                            .await
                            .unwrap_or(game.activity);
                        Some(crate::games::GameActivity { activity, ..game })
                    }
                    (value, _) => value,
                };
                let applied = this.update(cx, |app, cx| {
                    if app.game_generation != generation {
                        return;
                    }
                    if resolved.is_some() && !*app.game_enabled.borrow() {
                        return;
                    }
                    app.apply_game_activity(resolved, cx);
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.on_focus(&app.focus_handle, window, |app, window, cx| {
            if matches!(app.screen, Screen::Chat) {
                app.focus_composer(window, cx);
            }
        })
        .detach();
        // A focused field that unmounts, like the profile editor's name field or the tag
        // search, leaves nothing focused, and Escape and the other shortcuts would go nowhere.
        cx.on_focus_lost(window, |app, window, cx| {
            if matches!(app.screen, Screen::Chat) {
                app.focus_composer(window, cx);
            }
        })
        .detach();
        // The dialog unmounting (cancel, confirm or external deletion) drops its focus;
        // hand it back to the composer once the dialog is really gone.
        cx.on_blur(&app.delete_focus, window, |app, window, cx| {
            if app.confirm_delete.is_none() {
                app.focus_composer(window, cx);
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
            app.refresh_activity_feed(false, cx);
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
        let credential_load = cx
            .background_executor()
            .spawn(async { credentials::load() });
        cx.spawn_in(window, async move |this, cx| {
            let result = credential_load.await;
            this.update_in(cx, |app, window, cx| {
                if !matches!(app.screen, Screen::Connecting)
                    || app.session.is_some()
                    || app.pending_token.is_some()
                {
                    return;
                }
                match result {
                    Ok(Some(token)) => {
                        // Re-save after Ready to persist keyutils-only tokens from older builds.
                        app.pending_token = Some(token.clone());
                        app.connect(token, window, cx);
                    }
                    result => {
                        let mut form = LoginForm::new(window, cx);
                        if let Err(error) = result {
                            log::warn!("{error}");
                            form.error =
                                Some(format!("{error}. Your token may not be saved.").into());
                        }
                        app.screen = Screen::Login(form);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        app
    }

    fn apply_game_activity(
        &mut self,
        game: Option<crate::games::GameActivity>,
        cx: &mut Context<Self>,
    ) {
        let (activity, running) =
            game.map_or((None, None), |game| (Some(game.activity), game.running));
        self.current_game_application = running.as_ref().map(|game| game.application_id);
        self.current_game = running.map(|game| game.name);
        if let Some(session) = &self.session {
            session.set_local_activity(activity);
        }
        if let Some(application_id) = self.current_game_application {
            self.load_game_apps(vec![application_id], cx);
        }
        self.clear_stale_sharing_card();
        self.views.channel_list.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub(crate) fn update_game_activity(&mut self) {
        let enabled = crate::settings::game_activity()
            && self.live
            && self
                .session
                .as_ref()
                .is_some_and(|session| session.shows_current_game());
        let was_enabled = *self.game_enabled.borrow();
        let _ = self.game_enabled.send_if_modified(|current| {
            if *current == enabled {
                false
            } else {
                *current = enabled;
                true
            }
        });
        if was_enabled && !enabled {
            self.game_generation = self.game_generation.wrapping_add(1);
            self.current_game_application = None;
            self.current_game = None;
            if let Some(session) = &self.session {
                session.set_local_activity(None);
            }
        }
        self.clear_stale_sharing_card();
    }

    fn clear_stale_sharing_card(&mut self) {
        if self.current_game.is_none() || !channel_list::self_invisible(self) {
            self.sharing_paused = None;
        }
    }

    pub(crate) fn guild(&self) -> Option<&Guild> {
        self.store
            .guilds
            .iter()
            .find(|guild| guild.id == self.selected_guild_id)
    }

    pub(crate) fn channel(&self) -> Option<&Channel> {
        if self.is_home() {
            return self
                .store
                .private_channel(self.selected_channel_id)
                .map(|dm| &dm.channel);
        }
        self.guild()?.channel(self.selected_channel_id)
    }

    /// The Home screen (DMs and friends) is the selection without a guild.
    pub(crate) fn is_home(&self) -> bool {
        self.selected_guild_id == 0
    }

    pub(crate) fn showing_home_page(&self) -> bool {
        self.is_home() && self.selected_channel_id == 0
    }

    fn request_guild(&self) -> Option<u64> {
        (!self.is_home()).then_some(self.selected_guild_id)
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
            Event::Voice(event) => self.handle_voice_event(event, cx),
            Event::Reconnecting => {
                self.self_status.disconnect();
                self.reconnecting = true;
                cx.notify();
            }
            Event::Resumed => {
                self.reconnecting = false;
                if let Some(status) = self.selected_self_status() {
                    self.store.set_self_status(status);
                }
                self.arm_status_expiry_timer(cx);
                self.refresh_voice_start_times();
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
                self.thread_previews.clear();
                let reconnect = matches!(self.screen, Screen::Chat);
                self.member_subscription = None;
                self.store.apply(event);
                if let Some(status) = self.selected_self_status() {
                    self.store.set_self_status(status);
                }
                self.update_game_activity();
                self.refresh_voice_start_times();
                self.arm_status_expiry_timer(cx);
                self.load_user_affinities(cx);
                self.refresh_activity_feed(true, cx);
                self.sync_channel_rows();
                self.screen = Screen::Chat;
                if reconnect && self.channel().is_some() {
                    let channel = self.selected_channel_id;
                    let thread_pane = self.thread_pane.take();
                    self.select_channel(channel, window, cx);
                    self.restore_thread_pane(thread_pane, window, cx);
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
                        self.notify_message(message, cx);
                        self.typing_stopped(message.channel_id, message.author.id, cx)
                    }
                    Event::MessageDelete { id, .. } => self.forget_message(*id, cx),
                    Event::CallDelete { channel_id }
                        if self.voice.joined_call() == Some((0, *channel_id)) =>
                    {
                        self.leave_voice(cx);
                    }
                    _ => {}
                }
                let supplemental = matches!(event, Event::ReadySupplemental(_));
                let status_update = match &event {
                    Event::SettingsUpdate(update) => update.settings.kind == 1,
                    Event::SessionsReplace(_) | Event::ReadySupplemental(_) => true,
                    Event::PresenceUpdate(presence) => {
                        presence
                            .user_id
                            .or_else(|| presence.user.as_ref().map(|user| user.id))
                            == Some(self.store.current_user_id)
                    }
                    _ => false,
                };
                let reacted = match &event {
                    Event::MessageReactionAdd(event) | Event::MessageReactionRemove(event) => {
                        Some(event.message_id)
                    }
                    Event::MessageReactionRemoveAll(event) => Some(event.message_id),
                    Event::MessageReactionRemoveEmoji(event) => Some(event.message_id),
                    Event::MessageReactionAddMany(event) => Some(event.message_id),
                    _ => None,
                };
                let change = self.store.apply(event);
                if let Some(message_id) = reacted {
                    self.message_reactions_changed(message_id, cx);
                }
                if status_update && let Some(status) = self.selected_self_status() {
                    self.store.set_self_status(status);
                }
                if status_update {
                    self.update_game_activity();
                }
                self.apply_change(change, window, cx);
                if supplemental {
                    self.restore_voice(cx);
                }
            }
        }
    }

    fn apply_change(&mut self, change: Change, window: &mut Window, cx: &mut Context<Self>) {
        self.reconcile_dm_call(cx);
        self.arm_status_expiry_timer(cx);
        self.follow_account_theme(cx);
        self.load_activity_reply_assets(cx);
        self.sync_thread_pane(&change, cx);
        // Background channels only change badges, which nobody can see until a
        // repaint; coalesce a burst into one refresh with the member sync timer.
        if is_background_change(&change, self.selected_channel_id) {
            self.arm_members_timer(cx);
            return;
        }
        let len = self.channel().map_or(0, |channel| channel.messages.len());
        let placeholders = self.channel().is_some_and(|channel| {
            channel
                .messages
                .iter()
                .any(|message| message.delivery != crate::store::Delivery::Sent)
        });
        match change {
            Change::None => return,
            Change::VoiceTimers => {
                self.views.channel_list.update(cx, |_, cx| cx.notify());
                return;
            }
            Change::MessageInserted { channel_id, index }
                if channel_id == self.selected_channel_id =>
            {
                self.messages.splice(index..index, 1);
                self.messages.remeasure_items(if placeholders {
                    0..len
                } else {
                    index.saturating_sub(1)..(index + 2).min(len)
                });
                let own = self
                    .channel()
                    .and_then(|channel| channel.messages.get(index))
                    .is_some_and(|message| message.author_id == self.store.current_user_id);
                if self.unread_since.is_none() && !own && !self.viewing() {
                    self.unread_since =
                        self.channel()
                            .and_then(|channel| {
                                channel.messages[..index].iter().rev().find(|message| {
                                    message.delivery == crate::store::Delivery::Sent
                                })
                            })
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
                // Confirming a successor can also move earlier optimistic placeholders.
                self.messages.remeasure_items(if placeholders {
                    0..len
                } else {
                    removed.min(inserted).saturating_sub(1)..(removed.max(inserted) + 2).min(len)
                });
            }
            Change::MessageChanged { channel_id, index }
                if channel_id == self.selected_channel_id =>
            {
                // Author changes can also change whether the next row has a header.
                self.messages.remeasure_items(index..(index + 2).min(len));
            }
            Change::MessageRemoved { channel_id, index }
                if channel_id == self.selected_channel_id =>
            {
                self.stop_player(Some(window), cx);
                self.messages.splice(index..index + 1, 0);
                self.messages.remeasure_items(if placeholders {
                    0..len
                } else {
                    index.saturating_sub(1)..(index + 1).min(len)
                });
            }
            Change::Members { guild_id } => {
                if guild_id.is_none_or(|id| id == self.selected_guild_id) {
                    self.arm_members_timer(cx);
                    if self.mention_suggestions(cx).is_some() {
                        cx.notify();
                    }
                }
                return;
            }
            Change::Structure => {
                self.sync_channel_rows();
                self.sync_member_rows();
                if self.channel().is_none() && !self.showing_home_page() {
                    if self.is_home() {
                        self.open_friends(window, cx);
                    } else if self.store.guild_by_id(self.selected_guild_id).is_none() {
                        self.open_home(window, cx);
                    } else {
                        self.select_first_guild(window, cx);
                    }
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

    fn sync_channel_rows(&mut self) {
        self.request_activity_members();
        let guild = self
            .store
            .guilds
            .iter()
            .find(|guild| guild.id == self.selected_guild_id);
        let banner = guild.and_then(|guild| self.store.guild_banner(guild.id));
        let selected = self.thread_parent().unwrap_or(self.selected_channel_id);
        let opened = self.selected_channel_id;
        let guild_id = self.selected_guild_id;
        let store = &self.store;
        let joined = self
            .voice
            .call
            .as_ref()
            .filter(|call| call.guild_id == guild_id)
            .map(|call| call.channel_id);
        self.channel_rows.sync_with_banner(
            guild,
            banner,
            selected,
            |channel_id| store.voice_members(guild_id, channel_id).len(),
            |channel_id| store.sidebar_threads(channel_id, opened).len(),
            joined,
        );
        self.request_voice_members();
    }

    pub(crate) fn is_loading(&self, channel_id: u64) -> bool {
        self.loading.contains(&channel_id)
    }

    fn message_index(&self, message_id: u64) -> Option<usize> {
        self.channel()?.message_index(message_id)
    }

    fn remeasure_message(&self, message_id: u64) {
        if let Some(index) = self.message_index(message_id) {
            self.messages.remeasure_items(index..index + 1);
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
        discord::Error::Http(413) => "That file is too large to upload.".into(),
        error => error.to_string(),
    }
}

fn collect_mentions(blocks: &[markdown::Block], ids: &mut Vec<u64>) {
    use crate::text::markdown::{Block, SpanKind};
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
        theme::set_image_scale(window, cx);
        self.rpx_per_pixel = theme::BASE_FONT_SIZE / f32::from(window.rem_size());
        self.window_height = f32::from(window.viewport_size().height) * self.rpx_per_pixel;
        if matches!(self.screen, Screen::Chat) {
            self.update_member_subscription();
            self.load_active_now_apps(cx);
        }
        let body = match &self.screen {
            Screen::Login(form) => login::render(form, window, cx).into_any_element(),
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
                        .border_color(rgba(theme::border()))
                        .bg(rgb(theme::base_lower()))
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
                                .bg(rgb(theme::bg()))
                                .when(self.stream_chat_open(), |this| {
                                    this.flex_none().ml_auto().w(rpx(stream_view::CHAT_WIDTH))
                                })
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
                                                        self.selected_forum().is_some()
                                                            || self.showing_home_page(),
                                                    ),
                                                )),
                                        )
                                        .when(
                                            self.search_results_open()
                                                || (self.show_members
                                                    && (!self.is_home()
                                                        || self.dm_panel_width().is_some())
                                                    && !self.stream_chat_open()
                                                    && !self.thread_pane_open()),
                                            |this| {
                                                this.child(self.views.members.clone().cached(
                                                    views::member_list_style(
                                                        if self.search_results_open() {
                                                            search::RESULTS_WIDTH
                                                        } else {
                                                            self.dm_panel_width()
                                                                .unwrap_or(member_list::WIDTH)
                                                        },
                                                    ),
                                                ))
                                            },
                                        )
                                        .when(
                                            self.thread_pane_open() && !self.search_results_open(),
                                            |this| {
                                                this.child(self.views.thread_pane.clone().cached(
                                                    views::thread_pane_style(
                                                        self.selected_forum().is_some(),
                                                        self.thread_pane_is_dm(),
                                                    ),
                                                ))
                                            },
                                        ),
                                ),
                        ),
                )
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(|app, event: &ModifiersChangedEvent, _, cx| {
                if app.shift_held != event.modifiers.shift {
                    app.shift_held = event.modifiers.shift;
                    if app.hovered_message.is_some() || app.hovered_actions.is_some() {
                        cx.notify();
                    }
                }
            }))
            .on_action(cx.listener(|app, _: &JumpToPresent, window, cx| app.escape(window, cx)))
            .on_scroll_wheel(cx.listener(|app, _, _, cx| {
                app.close_expression_info(cx);
            }))
            .on_action(cx.listener(|app, _: &MarkGuildRead, _, cx| {
                app.mark_guild_read(app.selected_guild_id, cx)
            }))
            .on_action(cx.listener(|app, _: &ComposerUp, window, cx| app.composer_up(window, cx)))
            .on_action(cx.listener(|app, _: &QuickSwitch, window, cx| {
                app.toggle_quick_switcher(window, cx);
            }))
            .on_action(cx.listener(|app, _: &ComposerDown, window, cx| {
                if !(app.move_search(1, cx)
                    || app.move_quick_switcher(1, cx)
                    || app.suggest_input_focused(window, cx)
                        && (app.move_mention_suggestion(1, cx) || app.move_emoji_suggestion(1, cx)))
                {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|app, _: &FocusSearch, window, cx| {
                app.open_search(window, cx);
            }))
            .on_action(cx.listener(|app, _: &ComposerTab, window, cx| {
                if !(app.accept_search(window, cx)
                    || app.suggest_input_focused(window, cx)
                        && (app.accept_mention_suggestion(None, window, cx)
                            || app.accept_emoji_suggestion(None, window, cx)))
                {
                    cx.propagate();
                }
            }))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(rgb(theme::base_lowest()))
            .child(smooth_scroll::interceptor())
            .text_color(rgb(theme::text()))
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
                        .text_color(gpui::black())
                        .child("Connection lost. Reconnecting…"),
                )
            })
            .child(body)
            .child(self.views.overlays.clone().cached(views::overlays_style()))
            .when(
                cx.has_active_drag() && self.store.can_send(self.selected_channel_id),
                |this| {
                    this.child(
                        attachments::drop_overlay(self, cx)
                            .opacity(0.)
                            .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(1.))
                            .on_drop(cx.listener(|app, paths: &ExternalPaths, window, cx| {
                                app.drop_paths(paths.paths().to_vec(), window, cx);
                            })),
                    )
                },
            )
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
        .child(theme::logo(64.))
        .child(theme::spinner(24., theme::muted()))
        .child(
            div()
                .text_size(rpx(14.))
                .text_color(rgb(theme::muted()))
                .child("Connecting to Discord…"),
        )
}

#[cfg(test)]
mod composer_cursor_tests {
    use super::{is_background_change, snap_composer_cursor, snap_target};
    use crate::store::Change;
    use crate::text::composer_tokens;
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
