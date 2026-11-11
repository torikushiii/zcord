mod root;

mod events;

mod accounts;
mod activity;
mod appearance;
mod calls;
mod chat;
mod compose;
mod connection;
mod direct;
mod display_name;
mod expressions;
mod favorites_saving;
mod forward_dialog;
mod guilds;
mod history;
mod hover;
mod inbox;
mod lightbox;
mod login;
mod members;
mod menu;
pub(crate) mod messages;
mod notifications;
mod profile;
mod read_state;
mod search;
mod selection;
mod settings_page;
mod shell;
mod smooth_scroll;
mod state;
mod switcher;
mod titlebar;
mod views;

use crate::discord::{
    self, Event, FatalError, OutgoingMessage, OutgoingReply, Session, UploadProgress,
};
pub(crate) use appearance::apply_saved_theme;
use login::LoginForm;
pub use shell::AppShell;
use shell::{Restart, Start};

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
const JUMP_PAGE: usize = 30;
const OLDER_MESSAGES_THRESHOLD: usize = 8;

pub(crate) const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
const ACK_DELAY: Duration = Duration::from_secs(1);
const ACK_SEND_DELAY: Duration = Duration::from_secs(3);
const OLDER_RETRY_DELAY: Duration = Duration::from_secs(5);
pub(crate) const MESSAGE_LIMIT: usize = 2000;
const TYPING_RESEND: Duration = Duration::from_secs(9);
const TYPING_DURATION: Duration = Duration::from_secs(10);
const COALESCE_DELAY: Duration = Duration::from_millis(150);
const JUMP_STEP_FRACTION: f32 = 0.2;
const JUMP_GLIDE_VIEWPORTS: f32 = 3.;
pub(crate) const JUMP_HIGHLIGHT: Duration = Duration::from_millis(2000);
pub(crate) const JUMP_MISSING: Duration = Duration::from_secs(4);

struct JumpScroll {
    target: JumpTarget,
    nonce: u64,
}

#[derive(Clone, Copy)]
enum JumpTarget {
    Message(u64),
    Present,
    Offset(Pixels),
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
        ScrollPageUp,
        ScrollPageDown,
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
    files: Vec<messages::attachments::StagedFile>,
    entry: Option<Arc<serde_json::Value>>,
    pub error: String,
}

#[derive(Default)]
struct ForumPage {
    offset: usize,
    has_more: bool,
    loading: bool,
    loaded: bool,
    generation: u64,
    sort: Option<crate::discord::ForumSort>,
    gallery: Option<bool>,
    tags: Vec<u64>,
    match_all: bool,
    menu: bool,
    menu_closed: Option<Instant>,
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
    pub(crate) accounts: accounts::Accounts,
    account_theme: Option<crate::discord::settings_proto::AccountTheme>,
    activity_feed: activity::activity_feed::ActivityFeed,
    activity_ticker: Option<Task<()>>,
    pub(crate) activity_popout: Option<activity::activity_popout::ActivityPopout>,
    pub(crate) activity_card_tops: std::rc::Rc<std::cell::RefCell<HashMap<u64, Pixels>>>,
    pub(crate) selected_guild_id: u64,
    pub(crate) selected_channel_id: u64,
    pub(crate) dm_error: Option<(u64, String)>,
    pub(crate) menus: state::Menus,
    calls: calls::Calls,
    pub(crate) show_members: bool,
    pub(crate) compose: state::Compose,
    pub(crate) uploads: state::Uploads,
    pub(crate) hover: state::Hover,
    pub(in crate::app) message_menu: Option<chat::MessageMenu>,
    pub(in crate::app) composer_menu: Option<chat::ComposerMenu>,
    pub(crate) shift_held: bool,
    pub(in crate::app) channel_popout: Option<chat::ChannelPopout>,
    pub(in crate::app) popout_closed: Option<(chat::PopoutKind, Instant)>,
    pub(in crate::app) inbox: Option<inbox::Inbox>,
    pub(in crate::app) last_inbox_tab: inbox::InboxTab,
    pub(in crate::app) schedule_edit: Option<inbox::ScheduleEdit>,
    pub(in crate::app) custom_reminder: Option<messages::saved::CustomReminder>,
    reminder_timer: Option<Task<()>>,
    notified_reminders: HashSet<u64>,
    pub(in crate::app) create_thread: Option<chat::CreateThread>,
    pub(in crate::app) thread_menu: Option<chat::ThreadMenu>,
    pub(in crate::app) thread_pane: Option<chat::ThreadPane>,
    pub(crate) thread_previews: HashSet<u64>,
    pub(crate) sharing_paused: Option<Point<Pixels>>,
    pub(crate) profile: Option<profile::ProfileCard>,
    pub(crate) profile_styles: Option<profile::styles::StylesEditor>,
    pub(crate) status_editor: Option<profile::custom_status::StatusEditor>,
    pub(crate) self_status: profile::StatusChange,
    pub(crate) settings_page: Option<settings_page::SettingsPage>,
    settings_animation: u64,
    pub(crate) lightbox: Option<lightbox::Lightbox>,
    pub(crate) stream: state::Stream,
    voice_lobby: Option<(u64, u64)>,
    pub(crate) activity_privacy: activity::activity_privacy::Saving,
    pub(crate) text_images: messages::text_images::Saving,
    pub(crate) favorites_saving: favorites_saving::Saving,
    pub(crate) favorites_view: bool,
    streamer_notice_dismissed: bool,
    affinity_refresh: Option<Task<()>>,
    dm_profile: Option<profile::ProfileCard>,
    pub(crate) emoji: state::EmojiPopup,
    pub(crate) suggest: state::Suggestions,
    pub(crate) gifs: expressions::gif_picker::Gifs,
    pub(crate) stickers: expressions::sticker_picker::Stickers,
    pub(crate) home: direct::home::Home,
    recent_channels: crate::quick_switcher::Recent,
    typing: HashMap<u64, Vec<(u64, Instant)>>,
    typing_timer: Option<Task<()>>,
    typing_sent: Option<(u64, Instant)>,
    pub(crate) messages: ListState,
    pub(crate) channel_rows: guilds::channel_list::Rows,
    pub(crate) member_rows: ListState,
    member_layout: members::member_list::Layout,
    member_subscription: Option<(u64, u64, Vec<(u32, u32)>)>,
    pub(crate) live: bool,
    pub(crate) voice: calls::voice::Voice,
    pub(crate) reconnecting: bool,
    session: Option<Session>,
    pub(crate) game: state::Game,
    pending_token: Option<String>,
    pub(in crate::app) history: state::History,
    forum_pages: HashMap<u64, ForumPage>,
    forum_columns: Rc<Cell<usize>>,
    forum_header: Option<guilds::forum_actions::ForumHeader>,
    pub(crate) invites: HashMap<String, InviteState>,
    member_top: usize,
    open_folders: HashSet<u64>,
    pub(crate) revealed_spoilers: HashSet<(u64, u16)>,
    revealed_blocked_run_starts: HashSet<u64>,
    revealed_profiles: HashSet<u64>,
    pub(crate) window_height: f32,
    pub(crate) rpx_per_pixel: f32,
    reaction_tips: expressions::reaction_tooltip::ReactionTips,
    pub(crate) expression_info: expressions::expression_info::State,
    pub(crate) video: state::Video,
    pub(crate) jumps: state::Jumps,
    pub(crate) rail_morphs: guilds::guild_rail::Morphs,
    pub(crate) folder_folds: guilds::guild_rail::FolderFolds,
    pub(crate) entity: WeakEntity<Self>,
    pub(crate) views: views::PanelViews,
    focus_handle: FocusHandle,
    delete_focus: FocusHandle,
    window_active: bool,
    pub(crate) unread: state::Unread,
    status_expiry: Option<SystemTime>,
    status_expiry_timer: Option<Task<()>>,
    members_timer: Option<Task<()>>,
    parsed: RefCell<ParseCache>,
    _events: Option<Task<()>>,
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
        messages::attachments::clear_pasted(cx);
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
            accounts: accounts::Accounts::default(),
            account_theme: None,
            activity_feed: activity::activity_feed::ActivityFeed::default(),
            activity_ticker: None,
            activity_popout: None,
            activity_card_tops: Default::default(),
            selected_guild_id: 0,
            selected_channel_id: 0,
            dm_error: None,
            menus: state::Menus {
                dm: None,
                dm_dialog: None,
                guild: None,
                member: None,
                quick_switcher: None,
                quick_switcher_opens: 0,
                message_search: None,
                forward: None,
                reactions: None,
                reactions_openings: 0,
            },
            calls: calls::Calls::default(),
            show_members: true,
            compose: state::Compose {
                input: composer,
                reply: None,
                editing: None,
                confirm_delete: None,
                delete_closing: None,
                failed_sends: HashMap::new(),
                last_nonce: 0,
                pending_sends: compose::sending::PendingSends::default(),
                drafts: HashMap::new(),
                send_fill: Cell::default(),
                pane_send_fill: Cell::default(),
                _subscription: subscription,
                _cursor_subscription: cursor_subscription,
            },
            uploads: state::Uploads {
                staged: HashMap::new(),
                refusal: None,
                progress: HashMap::new(),
                next_id: 1,
                ticking: false,
                hovered_button: None,
            },
            hover: state::Hover {
                message: None,
                actions: None,
                member: None,
                account: false,
                rail: None,
                add_reaction: None,
            },
            message_menu: None,
            composer_menu: None,
            shift_held: false,
            channel_popout: None,
            popout_closed: None,
            inbox: None,
            last_inbox_tab: Default::default(),
            schedule_edit: None,
            custom_reminder: None,
            reminder_timer: None,
            notified_reminders: HashSet::new(),
            create_thread: None,
            thread_menu: None,
            thread_pane: None,
            thread_previews: HashSet::new(),
            sharing_paused: None,
            profile: None,
            status_editor: None,
            profile_styles: None,
            settings_page: None,
            settings_animation: 0,
            lightbox: None,
            emoji: state::EmojiPopup {
                picker: None,
                closing: None,
                animation: 0,
                closed_at: None,
            },
            suggest: state::Suggestions {
                emoji: Default::default(),
                mention: Default::default(),
                scroll: ScrollHandle::new(),
                mention_names: HashMap::new(),
            },
            gifs: Default::default(),
            stickers: Default::default(),
            home: Default::default(),
            recent_channels: Default::default(),
            typing: HashMap::new(),
            typing_timer: None,
            typing_sent: None,
            messages,
            channel_rows: guilds::channel_list::Rows::with_collapsed(
                crate::settings::get(cx).collapsed_categories,
            ),
            member_rows: ListState::new(0, ListAlignment::Top, px(400.)),
            member_layout: members::member_list::Layout::default(),
            member_subscription: None,
            live: false,
            voice: calls::voice::Voice::from_settings(cx),
            reconnecting: false,
            session: None,
            game: state::Game {
                enabled: game_enabled,
                generation: 0,
                application: None,
                current: None,
            },
            self_status: profile::StatusChange::default(),
            pending_token: None,
            history: state::History::default(),
            forum_pages: HashMap::new(),
            forum_columns: Rc::new(Cell::new(3)),
            forum_header: None,
            invites: HashMap::new(),
            member_top: 0,
            open_folders: HashSet::new(),
            revealed_spoilers: HashSet::new(),
            revealed_blocked_run_starts: HashSet::new(),
            revealed_profiles: HashSet::new(),
            window_height: f32::MAX,
            rpx_per_pixel: 1.,
            reaction_tips: Default::default(),
            stream: state::Stream {
                view: None,
                stage_chat: None,
                nonce: 0,
                members_shown: true,
                members_toggles: 0,
                volume: None,
                hovered_control: None,
            },
            voice_lobby: None,
            activity_privacy: Default::default(),
            text_images: Default::default(),
            favorites_saving: Default::default(),
            favorites_view: false,
            streamer_notice_dismissed: false,
            affinity_refresh: None,
            dm_profile: None,
            expression_info: expressions::expression_info::State::default(),
            video: state::Video {
                player: None,
                fullscreen: None,
                hovered: None,
                drag: None,
                dragged_at: None,
                seek_bounds: Rc::new(RefCell::new(None)),
                volume_bounds: Rc::new(RefCell::new(None)),
                volume_hover: [false; 2],
            },
            jumps: state::Jumps {
                scroll: None,
                highlight: None,
                missing: false,
                missing_nonce: 0,
            },
            rail_morphs: guilds::guild_rail::Morphs::default(),
            folder_folds: guilds::guild_rail::FolderFolds::default(),
            entity: app_entity.downgrade(),
            views,
            focus_handle: cx.focus_handle(),
            delete_focus: cx.focus_handle(),
            window_active: window.is_window_active(),
            unread: state::Unread {
                since: None,
                bar: false,
                manual: None,
                ack_timer: None,
                outgoing_acks: HashMap::new(),
            },
            status_expiry: None,
            status_expiry_timer: None,
            members_timer: None,
            parsed: RefCell::new(ParseCache::default()),
            _events: None,
        };
        let games = Tokio::spawn(cx, async move { crate::games::start(game_enabled_rx) });
        let runtime = Tokio::handle(cx);
        cx.spawn(async move |this, cx| {
            let Ok(mut output) = games.await else {
                return;
            };
            while output.changed().await.is_ok() {
                let value = output.borrow().clone();
                let (generation, session) =
                    match this.update(cx, |app, _| (app.game.generation, app.session.clone())) {
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
                    if app.game.generation != generation {
                        return;
                    }
                    if resolved.is_some() && !*app.game.enabled.borrow() {
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
        cx.on_blur(&app.delete_focus, window, |app, window, cx| {
            if app.compose.confirm_delete.is_none() {
                app.focus_composer(window, cx);
            }
        })
        .detach();
        cx.observe_window_bounds(window, |_, window, cx| {
            crate::settings::remember_window(window, cx)
        })
        .detach();
        appearance::device_appearance_changed(window, cx);
        cx.observe_window_appearance(window, |_, window, cx| {
            appearance::device_appearance_changed(window, cx);
        })
        .detach();
        cx.observe_window_activation(window, |app, window, cx| {
            app.window_active = window.is_window_active();
            crate::settings::set_window_focused(cx, app.window_active);
            if app.window_active {
                crate::settings::apply_reduce_motion(cx);
            }
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
                app.jumps.scroll = None;
                // GPUI runs this handler while the list's state is mutably borrowed, so any
                // `app.messages` call here panics; read the event and defer list access.
                let entity = app.entity.clone();
                cx.defer(move |cx| {
                    entity
                        .update(cx, |app, cx| app.check_message_edges(true, cx))
                        .ok();
                });
                if app.live && (!event.is_following_tail || app.unread.bar) {
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
                                if app.unread.bar {
                                    app.unread.bar = false;
                                    app.views.messages.update(cx, |_, cx| cx.notify());
                                }
                                app.schedule_ack(cx);
                            })
                            .ok();
                    });
                }
                app.schedule_ack(cx);
                if app.unread.since.is_some() || app.history.jump_return.is_some() {
                    app.views.messages.update(cx, |_, cx| cx.notify());
                }
            })
            .ok();
        });
        app
    }

    fn apply_game_activity(
        &mut self,
        game: Option<crate::games::GameActivity>,
        cx: &mut Context<Self>,
    ) {
        let (activity, running) =
            game.map_or((None, None), |game| (Some(game.activity), game.running));
        self.game.application = running.as_ref().and_then(|game| game.application_id);
        self.game.current = running.map(|game| game.name);
        if let Some(session) = &self.session {
            session.set_local_activity(activity);
        }
        if let Some(application_id) = self.game.application {
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
        let was_enabled = *self.game.enabled.borrow();
        let _ = self.game.enabled.send_if_modified(|current| {
            if *current == enabled {
                false
            } else {
                *current = enabled;
                true
            }
        });
        if was_enabled && !enabled {
            self.game.generation = self.game.generation.wrapping_add(1);
            self.game.application = None;
            self.game.current = None;
            if let Some(session) = &self.session {
                session.set_local_activity(None);
            }
        }
        self.clear_stale_sharing_card();
    }

    fn clear_stale_sharing_card(&mut self) {
        if self.game.current.is_none() || !guilds::channel_list::self_invisible(self) {
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

    pub(crate) fn is_loading(&self, channel_id: u64) -> bool {
        self.history.loading.contains(&channel_id)
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

fn snap_composer_cursor(input: &Entity<TextareaState>, previous: &Cell<usize>, cx: &mut App) {
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
    } else if previous == range.end || cursor - range.start <= range.end - cursor {
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
        discord::Error::Rejected { code: 50007, .. } => {
            "Your message could not be delivered. This is usually because you don't share a server with the recipient or the recipient is only accepting direct messages from friends.".into()
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
