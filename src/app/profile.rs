use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};
use gpui_tokio::Tokio;
use palette::{FromColor, Lab, Mix, Srgb};

use crate::{
    animated,
    app::{
        DISCORD_EPOCH_MS, DiscordApp,
        message_body::{BodyColors, BodyContext, label, render_body, wrapped_label},
    },
    discord::{ProfileKind, Session, model},
    images::{self, ImageSpec, profile_frame_layer_url},
    store::User,
    theme::{self, WithoutTextSelection, rpx},
};

#[cfg(test)]
mod activity_tests;
mod avatar_colors;
mod board;

const POPOUT_WIDTH: f32 = 300.;
const MODAL_WIDTH: f32 = 962.;
const MODAL_HEIGHT: f32 = 800.;
const MODAL_MARGIN: f32 = 48.;
const MODAL_RADIUS: f32 = 16.;
const MODAL_CARD_RADIUS: f32 = 16.;
const MODAL_MIN_VIEWPORT_WIDTH: f32 = 929.;
const MODAL_MIN_VIEWPORT_HEIGHT: f32 = 785.;
const BACKDROP_SOURCE: u32 = 512;
const BANNER_SOURCE: u32 = 480;
const BACKDROP_BLUR: f32 = 4.;
const BACKDROP_SATURATION: f32 = 0.5;
const BACKDROP_OPACITY: f32 = 0.3;
const BACKDROP_HEIGHT: f32 = 280.;
const EFFECT_TICK: Duration = Duration::from_millis(100);
const ENTER: Duration = Duration::from_millis(180);
const EFFECT_HEIGHT_RATIO: f32 = 880. / 450.;
const EFFECT_MAX_PX: u32 = 1200;
const THEME_OVERLAY: f32 = 0.6;
const SURFACE_HIGHEST: u32 = 0x26262C;
const MAX_VISIBLE_BADGES: usize = 6;
const BADGE_ART: f32 = 96.;
const MUTUAL_ICON: f32 = 20.;
/// Each stacked mutual icon starts 4px before the previous one ends.
const MUTUAL_STEP: f32 = MUTUAL_ICON - 4.;
const STATUS_OVERLAP: f32 = 10.;
/// Example questions Discord shows in your own empty status bubble.
const STATUS_PROMPTS: [&str; 6] = [
    "Best thing you ate this week?",
    "What's on your mind?",
    "What are you listening to?",
    "Currently obsessed with...",
    "What are you working on?",
    "Favorite game right now?",
];
const THEME_BORDER: f32 = 2.;
const BIO_LINES: usize = 3;

struct ActivityPrompt {
    heading: bool,
    text: &'static str,
}

const ACTIVITY_PROMPTS: [ActivityPrompt; 3] = [
    ActivityPrompt {
        heading: true,
        text: "They appear to be practicing the art of digital zen \u{1F9D8} take a deep breath and say hey/namaste",
    },
    ActivityPrompt {
        heading: true,
        text: "Their gaming console isn't linked \u{1F3AF} ask them what their gaming setup looks like",
    },
    ActivityPrompt {
        heading: false,
        text: "No activity shared yet - drop them a friendly hello to start chatting",
    },
];

pub(crate) struct ProfileCard {
    user_id: u64,
    guild_id: Option<u64>,
    position: Point<Pixels>,
    anchor: Anchor,
    modal: bool,
    profile: Option<Arc<model::Profile>>,
    pub(crate) games: HashMap<u64, Arc<model::Game>>,
    pub(crate) applications: HashMap<u64, Arc<model::PublicApplication>>,
    pub(crate) identities: Option<Arc<model::ApplicationIdentities>>,
    pub(crate) outbox: Option<Arc<model::Outbox>>,
    pub(crate) wishlist: Option<Arc<model::Wishlist>>,
    pub(crate) note: Option<String>,
    pub(crate) badge_details: Option<Arc<model::UserBadges>>,
    hovered_badge: Option<usize>,
    /// `(widget id, game id)` of the Board cover under the pointer, for its name tooltip.
    pub(crate) hovered_cover: Option<(u64, u64)>,
    pub(crate) widget_configs: HashMap<u64, Arc<model::WidgetConfig>>,
    board_requested: bool,
    modal_request_pending: bool,
    details_requested: bool,
    wishlist_requested: bool,
    note_requested: bool,
    pub(crate) shop_menu_open: bool,
    game_timer_started: bool,
    effect_timer_started: bool,
    avatar_theme: Option<ProfileTheme>,
    avatar_theme_source: Option<ImageSpec>,
    effect: Option<Arc<model::CollectibleProduct>>,
    frame: Option<Arc<model::CollectibleProduct>>,
    opened: Instant,
    /// Set when every effect layer has decoded; all layers share this clock.
    effect_started: RefCell<Option<Instant>>,
    tab: Tab,
    /// Index into `STATUS_PROMPTS`, rolled once per popout.
    status_prompt: usize,
    /// Chosen once per opening, not during redraws or tab changes.
    activity_prompt: usize,
    /// Game widgets opened past their first entries with "Show more".
    expanded_widgets: HashSet<u64>,
    /// The last "Show more"/"Show less" per widget: a generation that restarts its animation, and
    /// when it happened.
    pub(crate) widget_toggles: HashMap<u64, (u64, Instant)>,
    _tasks: Vec<Task<()>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Board,
    Activity,
    Wishlist,
    Servers,
    Friends,
}

struct Layout {
    width: f32,
    banner: f32,
    avatar: f32,
    avatar_left: f32,
    ring: f32,
    padding: f32,
    status_left: f32,
    header_min: f32,
    corner: f32,
}

impl ProfileCard {
    fn theme(&self) -> Option<ProfileTheme> {
        ProfileTheme::resolve(self.profile.as_deref(), self.avatar_theme)
    }

    fn palette(&self) -> ProfilePalette {
        ProfilePalette::of(self.theme())
    }

    fn load_applications(
        &mut self,
        session: Session,
        mut application_ids: Vec<u64>,
        cx: &mut Context<DiscordApp>,
    ) {
        application_ids.retain(|id| !self.applications.contains_key(id));
        if application_ids.is_empty() {
            return;
        }
        let (user_id, guild_id) = (self.user_id, self.guild_id);
        let request = Tokio::spawn(cx, async move {
            session.public_applications(&application_ids).await
        });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let applications = match request.await {
                Ok(Ok(applications)) => applications,
                Ok(Err(error)) => {
                    log::warn!("Could not load profile applications for user {user_id}: {error}");
                    return;
                }
                Err(error) => {
                    log::warn!("Profile applications task failed for user {user_id}: {error}");
                    return;
                }
            };
            this.update(cx, |app, cx| {
                let Some(card) = app
                    .profile
                    .as_mut()
                    .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                else {
                    return;
                };
                for application in applications {
                    card.applications.insert(application.id, application);
                }
                app.prefetch_profile_images(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn load_games(
        &mut self,
        session: Session,
        mut game_ids: Vec<u64>,
        cx: &mut Context<DiscordApp>,
    ) {
        game_ids.retain(|id| !self.games.contains_key(id));
        if game_ids.is_empty() {
            return;
        }
        let (user_id, guild_id) = (self.user_id, self.guild_id);
        let request = Tokio::spawn(cx, async move { session.games(&game_ids).await });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let games = match request.await {
                Ok(Ok(games)) => games,
                Ok(Err(error)) => {
                    log::warn!("Could not load profile games for user {user_id}: {error}");
                    return;
                }
                Err(error) => {
                    log::warn!("Profile games task failed for user {user_id}: {error}");
                    return;
                }
            };
            this.update(cx, |app, cx| {
                let Some(card) = app
                    .profile
                    .as_mut()
                    .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                else {
                    return;
                };
                for game in games {
                    card.games.insert(game.id, game);
                }
                app.prefetch_profile_images(cx);
                cx.notify();
            })
            .ok();
        }));
    }
}

impl Layout {
    fn popout(themed: bool) -> Self {
        let inset = if themed { THEME_BORDER } else { 0. };
        Self {
            width: POPOUT_WIDTH,
            banner: 105.,
            avatar: 80.,
            avatar_left: 16. - inset,
            ring: 6.,
            padding: 16.,
            status_left: 105.,
            // Banner + lower half of the avatar + ring + 8px breathing room; the
            // body's own 4px top padding brings the name 12px below the ring.
            header_min: 105. + 40. + 6. + 8. - inset,
            corner: if themed { 6. } else { 7. },
        }
    }

    fn modal(width: f32) -> Self {
        Self {
            width,
            banner: 140.,
            avatar: 120.,
            avatar_left: 32.,
            ring: 8.,
            padding: 32.,
            status_left: 32. + 120. + 20.,
            header_min: 140. + 60. + 20.,
            corner: MODAL_CARD_RADIUS,
        }
    }
}

impl DiscordApp {
    pub(crate) fn open_profile(
        &mut self,
        user_id: u64,
        position: Point<Pixels>,
        anchor: Anchor,
        cx: &mut Context<Self>,
    ) {
        let guild_id = (self.selected_guild_id != 0).then_some(self.selected_guild_id);
        self.profile = Some(ProfileCard {
            user_id,
            guild_id,
            position,
            anchor,
            modal: false,
            profile: None,
            games: HashMap::new(),
            applications: HashMap::new(),
            identities: None,
            outbox: None,
            wishlist: None,
            note: None,
            badge_details: None,
            hovered_badge: None,
            hovered_cover: None,
            widget_configs: HashMap::new(),
            board_requested: false,
            modal_request_pending: false,
            details_requested: false,
            wishlist_requested: false,
            note_requested: false,
            shop_menu_open: false,
            game_timer_started: false,
            effect_timer_started: false,
            avatar_theme: None,
            avatar_theme_source: None,
            effect: None,
            frame: None,
            opened: Instant::now(),
            effect_started: RefCell::new(None),
            tab: Tab::Activity,
            status_prompt: rand::random::<usize>() % STATUS_PROMPTS.len(),
            activity_prompt: rand::random::<usize>() % ACTIVITY_PROMPTS.len(),
            expanded_widgets: HashSet::new(),
            widget_toggles: HashMap::new(),
            _tasks: Vec::new(),
        });
        // The full response also covers the popout; avoid fetching both variants on a cold open.
        if let Some(profile) = self
            .session
            .as_ref()
            .filter(|_| self.live)
            .and_then(|session| session.cached_profile(user_id, guild_id, ProfileKind::Popout))
        {
            self.profile_loaded(user_id, guild_id, ProfileKind::Popout, profile, cx);
        }
        self.load_profile(ProfileKind::Modal, cx);
        self.prefetch_profile_details(cx);
        self.start_profile_game_timer(cx);
        cx.notify();
    }

    pub(crate) fn open_full_profile(&mut self, cx: &mut Context<Self>) {
        let Some(card) = &mut self.profile else {
            return;
        };
        card.modal = true;
        card.opened = Instant::now();
        card.effect_started.replace(None);
        // Discord opens on the Board; `side_panel` falls back to Activity without widgets.
        card.tab = Tab::Board;
        self.load_profile(ProfileKind::Modal, cx);
        self.start_profile_game_timer(cx);
        cx.notify();
    }

    fn toggle_profile_widget(&mut self, widget_id: u64, cx: &mut Context<Self>) {
        if let Some(card) = &mut self.profile {
            if !card.expanded_widgets.remove(&widget_id) {
                card.expanded_widgets.insert(widget_id);
            }
            let generation = card
                .widget_toggles
                .get(&widget_id)
                .map_or(0, |(generation, _)| generation + 1);
            card.widget_toggles
                .insert(widget_id, (generation, Instant::now()));
            cx.notify();
            // Redraw once the animation ends so the rows drop their animation-only clip.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(board::WIDGET_TOGGLE).await;
                this.update(cx, |_, cx| cx.notify()).ok();
            })
            .detach();
        }
    }

    fn open_profile_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.open_full_profile(cx);
        self.select_profile_tab(tab, cx);
    }

    pub(crate) fn hover_profile_cover(
        &mut self,
        key: (u64, u64),
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = &mut self.profile else {
            return;
        };
        let next = if hovered {
            Some(key)
        } else {
            card.hovered_cover.filter(|current| *current != key)
        };
        if card.hovered_cover != next {
            card.hovered_cover = next;
            self.views.overlays.update(cx, |_, cx| cx.notify());
        }
    }

    fn hover_profile_badge(&mut self, index: usize, hovered: bool, cx: &mut Context<Self>) {
        let Some(card) = &mut self.profile else {
            return;
        };
        let next = if hovered {
            Some(index)
        } else {
            card.hovered_badge.filter(|current| *current != index)
        };
        if card.hovered_badge != next {
            card.hovered_badge = next;
            self.views.overlays.update(cx, |_, cx| cx.notify());
        }
    }

    fn select_profile_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if let Some(card) = &mut self.profile {
            card.tab = tab;
            card.shop_menu_open = false;
            cx.notify();
        }
    }

    pub(crate) fn set_shop_menu(&mut self, open: bool, cx: &mut Context<Self>) {
        if let Some(card) = &mut self.profile
            && card.shop_menu_open != open
        {
            card.shop_menu_open = open;
            cx.notify();
        }
    }

    pub(crate) fn close_profile(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.profile.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    fn load_profile(&mut self, kind: ProfileKind, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(card) = &mut self.profile else {
            return;
        };
        if kind == ProfileKind::Modal {
            if card.modal_request_pending || card.board_requested {
                return;
            }
            card.modal_request_pending = true;
        }
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        if let Some(profile) = session.cached_profile(user_id, guild_id, kind) {
            self.profile_loaded(user_id, guild_id, kind, profile, cx);
            return;
        }
        let request = Tokio::spawn(cx, async move {
            session
                .profile(user_id, guild_id, Some(channel_id), kind)
                .await
        });
        card._tasks.push(cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                if let Some(card) = app.profile.as_mut().filter(|card| {
                    card.user_id == user_id && card.guild_id == guild_id
                }) && kind == ProfileKind::Modal {
                    card.modal_request_pending = false;
                }
                match result {
                    Ok(Ok(profile)) => app.profile_loaded(user_id, guild_id, kind, profile, cx),
                    Ok(Err(error)) => log::warn!(
                        "Could not load {kind:?} profile for user {user_id} in guild {guild_id:?}: {error}"
                    ),
                    Err(error) => log::warn!(
                        "Profile task failed for user {user_id}: {error}"
                    ),
                }
            })
            .ok();
        }));
    }

    fn prefetch_profile_details(&mut self, cx: &mut Context<Self>) {
        let channel_id = self.selected_channel_id;
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let Some(card) = &mut self.profile else {
            return;
        };
        if card.details_requested {
            return;
        }
        card.details_requested = true;
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        let game_ids: Vec<_> = self
            .store
            .user(user_id)
            .and_then(|user| user.game.as_ref()?.application_id)
            .into_iter()
            .collect();
        let cached = session.cached_profile_details(user_id, &game_ids, &game_ids);
        card.games.extend(cached.games);
        card.applications.extend(cached.applications);
        card.widget_configs.extend(cached.widget_configs);
        card.identities = cached.identities;
        card.outbox = cached.outbox;
        card.badge_details = cached.badges;
        let mut application_ids = game_ids.clone();
        if let Some(outbox) = &card.outbox {
            for id in outbox_application_ids(outbox) {
                if !application_ids.contains(&id) {
                    application_ids.push(id);
                }
            }
        }
        // Start independent resources now, not after the full profile response or a tab switch.
        if card.identities.is_none() {
            let session = session.clone();
            let request = Tokio::spawn(
                cx,
                async move { session.application_identities(user_id).await },
            );
            card._tasks.push(cx.spawn(async move |this, cx| {
                let identities = match request.await {
                    Ok(Ok(identities)) => identities,
                    Ok(Err(error)) => {
                        log::warn!(
                            "Could not load application identities for user {user_id}: {error}"
                        );
                        return;
                    }
                    Err(error) => {
                        log::warn!(
                            "Application identities task failed for user {user_id}: {error}"
                        );
                        return;
                    }
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile
                        .as_mut()
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.identities = Some(identities);
                    cx.notify();
                })
                .ok();
            }));
        }
        if card.badge_details.is_none() {
            let session = session.clone();
            let request = Tokio::spawn(cx, async move {
                session
                    .user_badges(user_id, guild_id, Some(channel_id))
                    .await
            });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let badges = match request.await {
                    Ok(Ok(badges)) => badges,
                    Ok(Err(error)) => {
                        log::warn!("Could not load user badges for user {user_id}: {error}");
                        return;
                    }
                    Err(error) => {
                        log::warn!("User badges task failed for user {user_id}: {error}");
                        return;
                    }
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile
                        .as_mut()
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.badge_details = Some(badges);
                    cx.notify();
                })
                .ok();
            }));
        }
        if card.outbox.is_none() {
            let outbox_session = session.clone();
            let application_session = session.clone();
            let request = Tokio::spawn(cx, async move { outbox_session.outbox(user_id).await });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let outbox = match request.await {
                    Ok(Ok(outbox)) => outbox,
                    Ok(Err(error)) => {
                        log::warn!("Could not load profile outbox for user {user_id}: {error}");
                        return;
                    }
                    Err(error) => {
                        log::warn!("Profile outbox task failed for user {user_id}: {error}");
                        return;
                    }
                };
                let application_ids = outbox_application_ids(&outbox);
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile
                        .as_mut()
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.outbox = Some(outbox);
                    card.load_applications(application_session, application_ids, cx);
                    app.prefetch_profile_images(cx);
                    cx.notify();
                })
                .ok();
            }));
        }
        if card.note.is_none() && !card.note_requested && user_id != self.store.current_user_id {
            card.note_requested = true;
            let note_session = session.clone();
            let request = Tokio::spawn(cx, async move { note_session.note(user_id).await });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let note = match request.await {
                    Ok(Ok(note)) => note,
                    Ok(Err(error)) => {
                        log::warn!("Could not load profile note for user {user_id}: {error}");
                        return;
                    }
                    Err(error) => {
                        log::warn!("Profile note task failed for user {user_id}: {error}");
                        return;
                    }
                };
                let Some(note) = note else {
                    return;
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile
                        .as_mut()
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.note = Some(note);
                    cx.notify();
                })
                .ok();
            }));
        }
        // The activity accessory needs the current game's config even when that app has no
        // Board widget; `details_requested` above keeps this to one request per card.
        if let Some(application_id) = game_ids.first().copied()
            && !card.widget_configs.contains_key(&application_id)
        {
            let accessory_session = session.clone();
            let request = Tokio::spawn(cx, async move {
                accessory_session.widget_config(application_id).await
            });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let config = match request.await {
                    Ok(Ok(Some(config))) => config,
                    Ok(Ok(None)) => return,
                    Ok(Err(error)) => {
                        log::warn!(
                            "Could not load activity accessory config for application {application_id}: {error}"
                        );
                        return;
                    }
                    Err(error) => {
                        log::warn!(
                            "Activity accessory config task failed for user {user_id}: {error}"
                        );
                        return;
                    }
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile
                        .as_mut()
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.widget_configs.insert(application_id, config);
                    cx.notify();
                })
                .ok();
            }));
        }
        card.load_games(session.clone(), game_ids, cx);
        card.load_applications(session, application_ids, cx);
        self.prefetch_profile_images(cx);
    }

    fn prefetch_profile_images(&self, cx: &App) {
        let Some(card) = &self.profile else {
            return;
        };
        let current = self
            .store
            .user(card.user_id)
            .and_then(|user| user.game.as_ref()?.application_id);
        board::prefetch_images(card, current, cx);
    }

    fn profile_loaded(
        &mut self,
        user_id: u64,
        guild_id: Option<u64>,
        kind: ProfileKind,
        profile: Arc<model::Profile>,
        cx: &mut Context<Self>,
    ) {
        self.store.absorb_profile(guild_id, &profile);
        let game = self.store.user(user_id).and_then(|user| user.game.clone());
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(card) = self
            .profile
            .as_mut()
            .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
        else {
            return;
        };
        // A late popout response lacks the modal's widgets and must not start Board loading.
        if (card.modal || card.board_requested) && kind == ProfileKind::Popout {
            return;
        }
        if kind == ProfileKind::Modal {
            card.modal_request_pending = false;
        }
        let effect = guild_profile(&profile)
            .and_then(|p| p.profile_effect.as_ref())
            .or_else(|| profile.user_profile.as_ref()?.profile_effect.as_ref())
            .map(|effect| effect.sku_id);
        let frame_of = |p: &model::UserProfile| {
            p.collectibles
                .iter()
                .find(|c| c.kind == 3)
                .map(|c| c.sku_id)
        };
        let frame = guild_profile(&profile)
            .and_then(frame_of)
            .or_else(|| profile.user_profile.as_ref().and_then(frame_of));
        let load_board = kind == ProfileKind::Modal && !card.board_requested;
        let mut game_ids = if load_board {
            widget_game_ids(&profile)
        } else {
            Vec::new()
        };
        let widget_application_ids = if load_board {
            widget_application_ids(&profile)
        } else {
            Vec::new()
        };
        let mut application_ids = widget_application_ids.clone();
        if load_board
            && let Some(application_id) = game.as_ref().and_then(|game| game.application_id)
        {
            if !game_ids.contains(&application_id) {
                game_ids.push(application_id);
            }
            if !application_ids.contains(&application_id) {
                application_ids.push(application_id);
            }
        }
        if load_board {
            let cached = session.cached_profile_details(user_id, &game_ids, &application_ids);
            card.games.extend(cached.games);
            card.applications.extend(cached.applications);
            card.identities = cached.identities;
            card.outbox = cached.outbox;
            card.widget_configs.extend(cached.widget_configs);
            if let Some(outbox) = &card.outbox {
                for id in outbox_application_ids(outbox) {
                    if !application_ids.contains(&id) {
                        application_ids.push(id);
                    }
                }
            }
        }
        card.profile = Some(profile);
        if card.wishlist.is_none()
            && !card.wishlist_requested
            && let Some(wishlist_id) = card
                .profile
                .as_ref()
                .and_then(|profile| profile.wishlist_ids.first().copied())
        {
            card.wishlist_requested = true;
            let wishlist_session = session.clone();
            let request =
                Tokio::spawn(cx, async move { wishlist_session.wishlist(wishlist_id).await });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let wishlist = match request.await {
                    Ok(Ok(wishlist)) => wishlist,
                    Ok(Err(error)) => {
                        log::warn!(
                            "Could not load wishlist {wishlist_id} for user {user_id}: {error}"
                        );
                        return;
                    }
                    Err(error) => {
                        log::warn!("Wishlist task failed for user {user_id}: {error}");
                        return;
                    }
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile
                        .as_mut()
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.wishlist = Some(wishlist);
                    cx.notify();
                })
                .ok();
            }));
        }
        for (sku_id, is_effect) in effect
            .map(|sku| (sku, true))
            .into_iter()
            .chain(frame.map(|sku| (sku, false)))
        {
            let session = session.clone();
            let request =
                Tokio::spawn(cx, async move { session.collectible_product(sku_id).await });
            card._tasks.push(cx.spawn(async move |this, cx| {
                let product = match request.await {
                    Ok(Ok(product)) => product,
                    Ok(Err(error)) => {
                        log::warn!(
                            "Could not load profile cosmetic {sku_id} for user {user_id}: {error}"
                        );
                        return;
                    }
                    Err(error) => {
                        log::warn!("Profile cosmetic task failed for user {user_id}: {error}");
                        return;
                    }
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile
                        .as_mut()
                        .filter(|c| c.user_id == user_id && c.guild_id == guild_id)
                    else {
                        return;
                    };
                    if is_effect {
                        card.effect = Some(product);
                    } else {
                        card.frame = Some(product);
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
        if effect.is_some() && !card.effect_timer_started {
            card.effect_timer_started = true;
            card._tasks.push(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(EFFECT_TICK).await;
                    let open = this
                        .update(cx, |app, cx| {
                            let open = app.profile.as_ref().is_some_and(|c| c.user_id == user_id);
                            if open {
                                app.views.overlays.update(cx, |_, cx| cx.notify());
                            }
                            open
                        })
                        .unwrap_or(false);
                    if !open {
                        break;
                    }
                }
            }));
        }
        if load_board {
            card.board_requested = true;
            // Board metadata is independent of the user's activity history.
            card.load_applications(session.clone(), application_ids, cx);
            card.load_games(session.clone(), game_ids, cx);
            let config_ids: Vec<_> = widget_application_ids
                .into_iter()
                .filter(|id| !card.widget_configs.contains_key(id))
                .collect();
            if !config_ids.is_empty() {
                let session = session.clone();
                card._tasks.push(cx.spawn(async move |this, cx| {
                    for application_id in config_ids {
                        let session = session.clone();
                        let request = Tokio::spawn(cx, async move {
                            session.widget_config(application_id).await
                        });
                        let config = match request.await {
                            Ok(Ok(Some(config))) => config,
                            Ok(Ok(None)) => continue,
                            Ok(Err(error)) => {
                                log::warn!(
                                    "Could not load widget config for application {application_id}: {error}"
                                );
                                continue;
                            }
                            Err(error) => {
                                log::warn!(
                                    "Profile widget config task failed for user {user_id}: {error}"
                                );
                                return;
                            }
                        };
                        this.update(cx, |app, cx| {
                            let Some(card) = app
                                .profile
                                .as_mut()
                                .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                            else {
                                return;
                            };
                            card.widget_configs.insert(application_id, config);
                            cx.notify();
                        })
                        .ok();
                    }
                }));
            }
        }
        self.prefetch_profile_images(cx);
        self.start_profile_game_timer(cx);
        self.load_avatar_theme(cx);
        cx.notify();
    }

    fn start_profile_game_timer(&mut self, cx: &mut Context<Self>) {
        let Some(card) = &mut self.profile else {
            return;
        };
        if card.game_timer_started
            || !self
                .store
                .user(card.user_id)
                .and_then(|user| user.game.as_ref())
                .is_some_and(|game| game.started_at.is_some())
        {
            return;
        }
        card.game_timer_started = true;
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        card._tasks.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let open = this
                    .update(cx, |app, cx| {
                        let open = app.profile.as_ref().is_some_and(|card| {
                            card.user_id == user_id && card.guild_id == guild_id
                        });
                        if open {
                            app.views.overlays.update(cx, |_, cx| cx.notify());
                        }
                        open
                    })
                    .unwrap_or(false);
                if !open {
                    break;
                }
            }
        }));
    }

    fn load_avatar_theme(&mut self, cx: &mut Context<Self>) {
        let Some(card) = &mut self.profile else {
            return;
        };
        if !ProfileTheme::needs_avatar(card.profile.as_deref()) {
            card.avatar_theme = None;
            card.avatar_theme_source = None;
            return;
        }
        let (user_id, guild_id) = (card.user_id, card.guild_id);
        let Some(spec) = self
            .store
            .member_avatar_spec(guild_id.unwrap_or(0), user_id)
            .or_else(|| self.store.user(user_id).map(User::avatar_spec))
        else {
            return;
        };
        if card.avatar_theme_source.as_ref() == Some(&spec) {
            return;
        }
        let Some(service) = images::download_service(cx) else {
            return;
        };
        card.avatar_theme = None;
        card.avatar_theme_source = Some(spec.clone());
        let source = spec.clone();
        let request = Tokio::spawn(cx, async move {
            let path = service.load(&source, 128).await?;
            tokio::task::spawn_blocking(move || avatar_colors::load(&path))
                .await
                .ok()
                .flatten()
        });
        card._tasks.push(cx.spawn(async move |this, cx| {
            let colors = request.await.ok().flatten();
            this.update(cx, |app, cx| {
                let Some(card) = app.profile.as_mut().filter(|card| {
                    card.user_id == user_id
                        && card.guild_id == guild_id
                        && card.avatar_theme_source.as_ref() == Some(&spec)
                }) else {
                    return;
                };
                card.avatar_theme =
                    colors.map(|[primary, secondary]| ProfileTheme { primary, secondary });
                if colors.is_none() {
                    card.avatar_theme_source = None;
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

fn guild_profile(profile: &model::Profile) -> Option<&model::UserProfile> {
    profile.guild_member_profile.as_ref()
}

fn widget_game_ids(profile: &model::Profile) -> Vec<u64> {
    let mut ids = Vec::new();
    for widget in &profile.widgets {
        let games = match &widget.data {
            model::WidgetData::FavoriteGames { games }
            | model::WidgetData::CurrentGames { games }
            | model::WidgetData::PlayedGames { games }
            | model::WidgetData::WantToPlayGames { games } => games,
            _ => continue,
        };
        for game in games {
            if !ids.contains(&game.game_id) {
                ids.push(game.game_id);
            }
        }
    }
    ids
}

fn widget_application_ids(profile: &model::Profile) -> Vec<u64> {
    let mut ids = Vec::new();
    for widget in &profile.widgets {
        if let model::WidgetData::Application { application_id } = &widget.data
            && !ids.contains(application_id)
        {
            ids.push(*application_id);
        }
    }
    ids
}

fn outbox_application_ids(outbox: &model::Outbox) -> Vec<u64> {
    let mut ids = Vec::new();
    for entry in &outbox.entries {
        if let model::OutboxKind::PlayedGame {
            application_id: Some(id),
            ..
        }
        | model::OutboxKind::LaunchedActivity {
            application_id: Some(id),
            ..
        } = &entry.kind
            && !ids.contains(id)
        {
            ids.push(*id);
        }
    }
    ids
}

pub fn opener(
    app: &DiscordApp,
    user_id: u64,
    anchor: Anchor,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = app.entity.clone();
    move |event, _, cx| {
        cx.stop_propagation();
        let gap = match anchor {
            Anchor::BottomLeft | Anchor::BottomRight => point(px(0.), px(-16.)),
            Anchor::TopRight => point(px(-16.), px(-40.)),
            _ => point(px(16.), px(-40.)),
        };
        let position = event.position() + gap;
        entity
            .update(cx, |app, cx| {
                app.open_profile(user_id, position, anchor, cx)
            })
            .ok();
    }
}

pub fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    let card = app.profile.as_ref()?;
    if card.modal {
        Some(modal(app, card, window, cx))
    } else {
        Some(popout(app, card, window, cx))
    }
}

#[derive(Clone, Copy)]
struct ProfileTheme {
    primary: u32,
    secondary: u32,
}

impl ProfileTheme {
    fn needs_avatar(profile: Option<&model::Profile>) -> bool {
        profile.is_some_and(|profile| profile.premium_type == Some(2))
            && Self::of(profile).is_none()
    }

    fn resolve(profile: Option<&model::Profile>, avatar: Option<Self>) -> Option<Self> {
        Self::of(profile).or_else(|| avatar.filter(|_| Self::needs_avatar(profile)))
    }

    fn of(profile: Option<&model::Profile>) -> Option<Self> {
        let profile = profile?;
        let colors = |profile: &model::UserProfile| {
            let colors = profile.theme_colors.as_deref()?;
            Some((*colors.first()?, *colors.get(1)?))
        };
        let (primary, secondary) = guild_profile(profile)
            .and_then(colors)
            .or_else(|| profile.user_profile.as_ref().and_then(colors))?;
        Some(Self { primary, secondary })
    }

    fn outer(self) -> Background {
        linear_gradient(
            180.,
            linear_color_stop(rgb(self.primary), 0.),
            linear_color_stop(rgb(self.secondary), 1.),
        )
    }

    fn surface(self) -> u32 {
        self.surface_color(self.primary)
    }

    fn is_light(self) -> bool {
        // Discord selects the profile mode from the primary color's perceived brightness.
        let channel = |shift: u32| ((self.primary >> shift) & 0xFF) as f64;
        0.299 * channel(16) + 0.587 * channel(8) + 0.114 * channel(0) >= 127.5
    }

    fn overlay(self) -> Hsla {
        hsla(0., 0., if self.is_light() { 1. } else { 0. }, THEME_OVERLAY)
    }

    fn surface_color(self, color: u32) -> u32 {
        blend(
            color,
            if self.is_light() { 0xFFFFFF } else { 0 },
            THEME_OVERLAY,
        )
    }

    fn modal_background(self) -> Background {
        let to_lab = |color: u32| -> Lab<_, f64> {
            let channel = |shift: u32| ((color >> shift) & 0xFF) as f64 / 255.;
            Lab::from_color(Srgb::new(channel(16), channel(8), channel(0)))
        };
        // The dialog uses a Lab midpoint with constrained lightness, not the card gradient.
        let midpoint: Srgb<f64> =
            Srgb::from_color(to_lab(self.primary).mix(to_lab(self.secondary), 0.5));
        let mut background = palette::Hsl::from_color(midpoint);
        let lightness = (background.lightness * 100.).round() / 100.;
        background.lightness = if self.is_light() {
            lightness.clamp(0.8, 1.)
        } else {
            lightness.clamp(0., 0.1)
        };
        let background: Srgb<u8> = Srgb::from_color(background).into_format();
        rgb((u32::from(background.red) << 16)
            | (u32::from(background.green) << 8)
            | u32::from(background.blue))
        .into()
    }
}

fn blend(color: u32, overlay: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let base = ((color >> shift) & 0xFF) as f32;
        let overlay = ((overlay >> shift) & 0xFF) as f32;
        (base * (1. - amount) + overlay * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn luminance(color: u32) -> f32 {
    let channel = |shift: u32| {
        let value = ((color >> shift) & 0xFF) as f32 / 255.;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn contrast(foreground: u32, background: u32) -> f32 {
    let foreground = luminance(foreground);
    let background = luminance(background);
    (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
}

#[derive(Clone, Copy)]
struct ProfilePalette {
    light: bool,
    strong: u32,
    text: u32,
    muted: u32,
    faint: u32,
    panel: Hsla,
    /// Filled buttons inside a panel (Play, Get on Steam, Browse the Shop).
    button: Hsla,
    /// Strips that sit a step above a panel, like the activity accessory footer.
    raised: Hsla,
    border: Hsla,
    hover: Hsla,
}

impl ProfilePalette {
    fn of(theme: Option<ProfileTheme>) -> Self {
        let light = theme.is_some_and(ProfileTheme::is_light);
        let mut palette = Self {
            light,
            strong: if light { 0x18141F } else { theme::STRONG },
            text: if light { 0x2E2938 } else { theme::TEXT },
            muted: if light { 0x51495E } else { theme::MUTED },
            faint: if light { 0x62596F } else { theme::FAINT },
            // Discord's dark profile surfaces are a faintly blue white over the card: #18181a,
            // #2a2a2c and #2c2c30 on a black theme.
            panel: if light { hsla(0., 0., 1., 0.2) } else { rgba(0xEBEBFF1A).into() },
            button: if light { hsla(0., 0., 1., 0.35) } else { rgba(0xEBEBFF2E).into() },
            raised: if light { hsla(0., 0., 1., 0.3) } else { rgba(0xEBEBFF30).into() },
            border: hsla(0., 0., if light { 0. } else { 1. }, 0.08),
            hover: hsla(0., 0., if light { 0. } else { 1. }, 0.04),
        };
        if let Some(theme) = theme {
            let backgrounds = [
                theme.surface_color(theme.primary),
                theme.surface_color(theme.secondary),
            ];
            // Keep metadata readable even when the two custom stops have very different brightness.
            for color in [&mut palette.muted, &mut palette.faint] {
                if backgrounds
                    .iter()
                    .any(|background| contrast(*color, *background) < 4.5)
                {
                    *color = palette.text;
                }
            }
        }
        palette
    }
}

fn popout(app: &DiscordApp, card: &ProfileCard, window: &Window, cx: &App) -> AnyElement {
    let theme = card.theme();
    let palette = card.palette();
    let layout = Layout::popout(theme.is_some());
    let surface = theme.map_or(theme::ELEVATED, ProfileTheme::surface);
    let entity = app.entity.clone();
    let open_full = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
    };
    let inner = div()
        .relative()
        .flex()
        .flex_col()
        .rounded(rpx(if theme.is_some() { 6. } else { 8. }))
        .overflow_hidden()
        .pb(rpx(4.))
        .text_color(rgb(palette.text))
        .map(|this| match theme {
            Some(theme) => this.bg(theme.overlay()),
            None => this,
        })
        .child(header(
            app,
            card,
            &layout,
            surface,
            Some(Box::new(open_full)),
            cx,
        ))
        .child(popout_body(app, card, &layout, cx))
        .children(effect_layers(card, layout.width, cx));
    let outer_width = POPOUT_WIDTH + theme.map_or(0., |_| THEME_BORDER * 2.);
    let outer = div()
        .relative()
        .w(rpx(outer_width))
        .rounded(rpx(8.))
        .shadow_lg()
        .map(|this| match theme {
            Some(theme) => this.p(rpx(THEME_BORDER)).bg(theme.outer()),
            None => this
                .bg(rgb(theme::ELEVATED))
                .border_1()
                .border_color(rgb(theme::BORDER)),
        })
        .child(inner)
        .into_any_element();
    let framed = framed(card, outer_width, outer, false, cx);
    let close = app.entity.clone();
    // Keep the frame art that hangs past the card on screen, not just the card itself.
    let overflow = frame_overflow(card, outer_width).unwrap_or_default();
    let margin = |overflow: f32| rpx(overflow).to_pixels(window.rem_size()) + px(12.);
    deferred(
        anchored()
            .position(card.position)
            .anchor(card.anchor)
            .snap_to_window_with_margin(Edges {
                top: margin(overflow.top),
                bottom: margin(overflow.bottom),
                left: margin(overflow.left),
                right: margin(overflow.right),
            })
            .child(
                div()
                    .id("profile-popout")
                    .relative()
                    .occlude()
                    .on_mouse_down_out(move |_, _, cx| {
                        close.update(cx, |app, cx| app.close_profile(cx)).ok();
                    })
                    .child(framed)
                    .with_animation(
                        ("profile-popout-enter", card.user_id),
                        Animation::new(ENTER).with_easing(ease_out_quint()),
                        |this, delta| this.opacity(delta),
                    ),
            ),
    )
    .with_priority(1)
    .into_any_element()
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

fn header(
    app: &DiscordApp,
    card: &ProfileCard,
    layout: &Layout,
    surface: u32,
    on_avatar: Option<ClickHandler>,
    cx: &App,
) -> AnyElement {
    let user = app.store.user(card.user_id);
    let palette = card.palette();
    let status = user
        .filter(|user| !user.playing)
        .and_then(|user| user.activity.clone());
    // Your own profile shows an "add status" prompt in place of an empty bubble.
    let prompt = status.is_none() && card.user_id == app.store.current_user_id;
    let status = status
        .map(|text| (text, false))
        .or_else(|| prompt.then(|| (STATUS_PROMPTS[card.status_prompt].to_owned(), true)));
    let avatar = div()
        .id("profile-avatar")
        .absolute()
        .left(rpx(layout.avatar_left - layout.ring))
        .top(rpx(layout.banner - layout.avatar / 2. - layout.ring))
        .p(rpx(layout.ring))
        .rounded_full()
        .bg(rgb(surface))
        .child(theme::member_avatar(
            &app.store,
            card.guild_id.unwrap_or(0),
            card.user_id,
            layout.avatar,
            user.map(|user| (user.status, surface)),
            true,
            cx,
        ))
        .map(|this| match on_avatar {
            Some(handler) => this.cursor_pointer().on_click(handler),
            None => this,
        });
    let status_height = status.as_ref().map_or(0., |(text, prompt)| {
        if text.chars().count() > if *prompt { 18 } else { 22 } {
            54.
        } else {
            36.
        }
    });
    let height = layout
        .header_min
        .max(layout.banner - STATUS_OVERLAP + status_height);
    div()
        .relative()
        .w_full()
        .h(rpx(height))
        .child(banner(card, layout, cx))
        .child(avatar)
        .children(
            status
                .map(|(text, prompt)| status_bubble(card.user_id, &text, prompt, layout, palette)),
        )
        .into_any_element()
}

fn banner(card: &ProfileCard, layout: &Layout, cx: &App) -> impl IntoElement {
    let profile = card.profile.as_deref();
    let spec = profile.and_then(|p| {
        let member_banner = p.guild_member.as_ref().and_then(|m| m.banner.clone());
        match (member_banner, card.guild_id) {
            (Some(hash), Some(guild_id)) => Some(ImageSpec::GuildMemberBanner {
                guild_id,
                user_id: card.user_id,
                hash,
            }),
            _ => p.user.banner.clone().map(|hash| ImageSpec::Banner {
                user_id: card.user_id,
                hash,
            }),
        }
    });
    let pixels = BANNER_SOURCE;
    let corner = rpx(layout.corner);
    let image = spec.and_then(|spec| {
        let animated = spec
            .animated()
            .and_then(|animated| animated::resolve(&animated, pixels, cx));
        Some(match animated {
            Some(image) => animated::image(image)
                .size_full()
                .rounded_t(corner)
                .object_fit(ObjectFit::Cover)
                .id("profile-banner")
                .into_any_element(),
            None => img(images::resolve(&spec, pixels, cx)?)
                .size_full()
                .rounded_t(corner)
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
        })
    });
    let color = profile
        .and_then(|p| p.user.banner_color.or(p.user.accent_color))
        .or_else(|| profile?.user_profile.as_ref()?.accent_color)
        .or_else(|| card.theme().map(|theme| theme.primary))
        .unwrap_or(theme::SELECTED);
    div()
        .w_full()
        .h(rpx(layout.banner))
        .rounded_t(corner)
        .bg(rgb(color))
        .children(image)
}

fn status_bubble(
    user_id: u64,
    text: &str,
    prompt: bool,
    layout: &Layout,
    palette: ProfilePalette,
) -> impl IntoElement + use<> {
    let surface = if palette.light {
        0xF1EDF5
    } else {
        SURFACE_HIGHEST
    };
    let bubble = |size: f32| {
        div()
            .absolute()
            .size(rpx(size))
            .rounded_full()
            .bg(rgb(surface))
            .border_1()
            .border_color(palette.border)
    };
    div()
        .absolute()
        .left(rpx(layout.status_left))
        .top(rpx(layout.banner - 7.))
        .right(rpx(12.))
        .flex()
        .child(
            // Thought circles come first so the bubble paints over them,
            // hiding their lower half behind its edge.
            div()
                .relative()
                .child(bubble(20.).left(rpx(10.)).top(rpx(-8.)))
                .child(bubble(10.).left(rpx(-3.)).top(rpx(-15.)))
                .child(status_body(user_id, text, prompt, surface, palette)),
        )
}

fn status_body(
    user_id: u64,
    text: &str,
    prompt: bool,
    surface: u32,
    palette: ProfilePalette,
) -> Div {
    div()
        .max_w(rpx(181.))
        .min_w(rpx(42.))
        .px(rpx(12.))
        .py(rpx(8.))
        .rounded(rpx(12.))
        .bg(rgb(surface))
        .border_1()
        .border_color(palette.border)
        .shadow_sm()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(palette.text))
        .map(|this| {
            if prompt {
                // Will open a set-status dialog once one exists.
                this.cursor_pointer()
                    .flex()
                    .items_start()
                    .gap(rpx(4.))
                    .child(theme::icon("circle-plus-filled", 16., palette.muted).mt(rpx(1.)))
                    .child(
                        div()
                            .line_clamp(2)
                            .italic()
                            .text_color(rgb(palette.muted))
                            .child(text.to_owned()),
                    )
            } else {
                this.child(
                    div()
                        .line_clamp(2)
                        .child(wrapped_label(("profile-status", user_id), text)),
                )
            }
        })
}

fn display_names(
    app: &DiscordApp,
    card: &ProfileCard,
    large: bool,
    on_name: Option<ClickHandler>,
    cx: &App,
) -> impl IntoElement + use<> {
    let profile = card.profile.as_deref();
    let palette = card.palette();
    let user = app.store.user(card.user_id);
    let guild_id = card.guild_id.unwrap_or(0);
    let name = app
        .store
        .member_name(guild_id, card.user_id)
        .map(str::to_owned)
        .or_else(|| user.map(|user| user.name.clone()))
        .unwrap_or_else(|| "Unknown User".to_owned());
    let username = profile
        .map(|p| p.user.user.username.clone())
        .filter(|username| !username.is_empty());
    let pronouns = profile
        .and_then(guild_profile)
        .and_then(|p| p.pronouns.clone())
        .filter(|p| !p.is_empty())
        .or_else(|| profile?.user_profile.as_ref()?.pronouns.clone())
        .filter(|p| !p.is_empty());
    let name_style = app.store.member_name_style(guild_id, card.user_id);
    let dot = || {
        div()
            .size(rpx(4.))
            .mx(rpx(1.))
            .rounded_full()
            .bg(rgb(palette.text))
    };
    div()
        .flex()
        .flex_col()
        .gap(rpx(2.))
        .child(
            div()
                .id("profile-display-name")
                .map(|this| match on_name {
                    Some(handler) => this.cursor_pointer().on_click(handler),
                    None => this,
                })
                .text_size(rpx(if large { 24. } else { 20. }))
                .line_height(rpx(if large { 30. } else { 24. }))
                .font_weight(if large {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::BOLD
                })
                .child(theme::display_name(&name, name_style, palette.strong)),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(rpx(6.))
                .gap_y(rpx(if large { 4. } else { 2. }))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(palette.text))
                .children(username.map(|username| div().child(username)))
                .when(user.is_some_and(|user| user.bot), |this| {
                    this.child(theme::app_tag())
                })
                .children(pronouns.map(|pronouns| {
                    div()
                        .flex()
                        .items_center()
                        .gap_x(rpx(6.))
                        .child(dot())
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.strong))
                                .child(pronouns),
                        )
                }))
                .children(user.and_then(|user| user.tag.as_ref()).map(|tag| {
                    theme::guild_tag(tag, cx)
                        .bg(palette.panel)
                        .text_color(rgb(palette.strong))
                }))
                .children(profile.and_then(|profile| badges(app, card, profile, palette, cx))),
        )
        .when(!large, |this| {
            this.children(profile.and_then(|profile| mutuals_line(app, profile, palette, cx)))
        })
}

fn mutuals_line(
    app: &DiscordApp,
    profile: &model::Profile,
    palette: ProfilePalette,
    cx: &App,
) -> Option<AnyElement> {
    let friend_count = profile
        .mutual_friends_count
        .filter(|count| *count > 0)
        .map(|count| count as usize)
        .or_else(|| {
            profile
                .mutual_friends
                .as_ref()
                .map(Vec::len)
                .filter(|len| *len > 0)
        });
    let friend_list = profile.mutual_friends.as_deref().unwrap_or_default();
    let friends = friend_count.map(|count| {
        let avatars = friend_list.iter().take(3).map(|friend| {
            theme::user_avatar(app.store.user(friend.id), friend.id, MUTUAL_ICON, None, false, cx)
                .into_any_element()
        });
        // Only a returned list has a Friends tab to open.
        let tab = (!friend_list.is_empty()).then_some(Tab::Friends);
        mutual_part(app, "profile-mutual-friends", avatars, plural(count, "Mutual Friend"), tab)
    });
    let guilds = profile.mutual_guilds.as_deref().unwrap_or_default();
    let servers = (!guilds.is_empty()).then(|| {
        let icons = guilds
            .iter()
            .filter_map(|mutual| app.store.guild_by_id(mutual.id))
            .take(3)
            .map(|guild| guild_icon(guild, MUTUAL_ICON, cx));
        mutual_part(
            app,
            "profile-mutual-servers",
            icons,
            plural(guilds.len(), "Mutual Server"),
            Some(Tab::Servers),
        )
    });
    let parts: Vec<_> = friends.into_iter().chain(servers).collect();
    if parts.is_empty() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(rpx(6.))
            .py(rpx(4.))
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .text_color(rgb(palette.muted))
            .children(parts.into_iter().enumerate().flat_map(|(index, part)| {
                let spacer = (index > 0).then(|| {
                    div()
                        .size(rpx(4.))
                        .rounded_full()
                        .bg(rgb(palette.muted))
                        .into_any_element()
                });
                spacer.into_iter().chain(std::iter::once(part))
            }))
            .into_any_element(),
    )
}

/// One "N Mutual …" entry: up to three overlapping icons, then the count, opening `tab` on click.
fn mutual_part(
    app: &DiscordApp,
    id: &'static str,
    icons: impl Iterator<Item = AnyElement>,
    text: String,
    tab: Option<Tab>,
) -> AnyElement {
    let entity = app.entity.clone();
    mutual_entry(id, icons, text)
        .when_some(tab, |this, tab| {
            this.cursor_pointer()
                .hover(|style| style.underline())
                .on_click(move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| app.open_profile_tab(tab, cx))
                        .ok();
                })
        })
        .into_any_element()
}

fn mutual_entry(
    id: &'static str,
    icons: impl Iterator<Item = AnyElement>,
    text: String,
) -> Stateful<Div> {
    // A negative margin collapses the stack's width to zero in gpui's layout, so the icons are
    // placed absolutely inside an explicitly sized box instead.
    let icons: Vec<_> = icons
        .enumerate()
        .map(|(index, icon)| {
            div()
                .absolute()
                .top_0()
                .left(rpx(index as f32 * MUTUAL_STEP))
                .size(rpx(MUTUAL_ICON))
                .rounded_full()
                .child(icon)
        })
        .collect();
    let width = MUTUAL_ICON + icons.len().saturating_sub(1) as f32 * MUTUAL_STEP;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(rpx(6.))
        .when(!icons.is_empty(), |this| {
            this.child(
                div()
                    .debug_selector(|| "mutual-icons".into())
                    .relative()
                    .flex_none()
                    .w(rpx(width))
                    .h(rpx(MUTUAL_ICON))
                    .children(icons),
            )
        })
        .child(div().debug_selector(|| "mutual-text".into()).child(text))
}

fn guild_icon(guild: &crate::store::Guild, size: f32, cx: &App) -> AnyElement {
    let icon = guild
        .icon_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(size), cx));
    match icon {
        Some(path) => img(path).size(rpx(size)).rounded_full().into_any_element(),
        None => div()
            .size(rpx(size))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(rgb(guild.icon_color))
            .text_size(rpx(size * 0.4))
            .text_color(rgb(theme::BG))
            .child(guild.initials.clone())
            .into_any_element(),
    }
}

#[derive(Clone)]
struct BadgeTip {
    art: Option<ImageSpec>,
    category: Option<String>,
    title: String,
    detail: Option<String>,
}

impl BadgeTip {
    fn of(badge: &model::Badge, detail: &model::BadgeDetail) -> Self {
        let tier = detail.tier();
        let tier_name = tier
            .and_then(|tier| tier.name.clone())
            .filter(|name| !name.is_empty());
        let name = Some(detail.name.clone()).filter(|name| !name.is_empty());
        // Discord titles Nitro tiers "Nitro Opal"; other levelled badges put the badge name above the tier.
        let (category, title) = match tier_name {
            Some(tier) if detail.badge_id == 1 => {
                (None, name.map_or(tier.clone(), |name| format!("{name} {tier}")))
            }
            Some(tier) => (name, tier),
            None => (None, name.unwrap_or_else(|| badge.description.clone())),
        };
        let art = [
            tier.and_then(|tier| tier.complex_icon_animated_url.as_deref()),
            detail.complex_icon_animated_url.as_deref(),
            tier.and_then(|tier| tier.simple_icon_raster_url.as_deref()),
            detail.simple_icon_raster_url.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(badge_art)
        .next()
        .or_else(|| {
            Some(ImageSpec::Badge {
                icon: badge.icon.clone(),
            })
        });
        let detail = detail
            .info_label
            .clone()
            .or_else(|| Some(badge.description.clone()))
            .filter(|text| !text.is_empty() && !text.eq_ignore_ascii_case(&title));
        Self {
            art,
            category,
            title,
            detail,
        }
    }
}

// The SVG variants of badge art can't be decoded, so only the PNG URLs reach here.
fn badge_art(url: &str) -> ImageSpec {
    match url
        .rsplit_once("/badge-icons/")
        .and_then(|(_, file)| file.strip_suffix(".png"))
    {
        Some(icon) => ImageSpec::Badge {
            icon: icon.to_owned(),
        },
        None => ImageSpec::Collectible {
            url: url.to_owned(),
        },
    }
}

/// Rich badge card, centred above the badge like Discord's.
fn badge_tooltip(tip: &BadgeTip, cx: &App) -> impl IntoElement {
    let art = tip
        .art
        .as_ref()
        .and_then(|spec| images::resolve(spec, theme::image_px(BADGE_ART), cx));
    let body = div()
        .flex()
        .flex_col()
        .items_center()
        .min_w(rpx(192.))
        .max_w(rpx(256.))
        .px(rpx(16.))
        .py(rpx(16.))
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .shadow_lg()
        .text_center()
        .children(art.map(|path| {
            img(path)
                .size(rpx(BADGE_ART))
                .mb(rpx(8.))
                .object_fit(ObjectFit::Contain)
        }))
        .children(tip.category.clone().map(|category| {
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::MUTED))
                .child(category)
        }))
        .child(
            div()
                .text_size(rpx(18.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .italic()
                .text_color(rgb(theme::STRONG))
                .child(tip.title.to_uppercase()),
        )
        .children(tip.detail.clone().map(|detail| {
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::MUTED))
                .child(detail)
        }));
    div()
        .absolute()
        .top(rpx(-4.))
        .left(relative(0.5))
        .size_0()
        .child(
            deferred(
                anchored().anchor(Anchor::BottomCenter).child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .child(body)
                        // Overlaps the border by 1px so the arrow reads as part of the card.
                        .child(
                            theme::icon("tooltip-arrow-down", 12., theme::ELEVATED)
                                .h(rpx(6.))
                                .mt(rpx(-1.)),
                        ),
                ),
            )
            // Above the popout, which is itself deferred at priority 1.
            .with_priority(2),
        )
}

fn badges(
    app: &DiscordApp,
    card: &ProfileCard,
    profile: &model::Profile,
    palette: ProfilePalette,
    cx: &App,
) -> Option<AnyElement> {
    let details = card.badge_details.as_deref();
    if profile.badges.is_empty() {
        return None;
    }
    let shown = if profile.badges.len() > MAX_VISIBLE_BADGES + 1 {
        MAX_VISIBLE_BADGES
    } else {
        profile.badges.len()
    };
    let hidden = profile.badges.len() - shown;
    Some(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(rpx(2.))
            .w_auto()
            .max_w_full()
            .px(rpx(3.))
            .py(rpx(1.))
            .rounded(rpx(8.))
            .bg(palette.panel)
            .border_1()
            .border_color(palette.border)
            .children(
                profile
                    .badges
                    .iter()
                    .take(shown)
                    .enumerate()
                    .map(|(index, badge)| {
                        let spec = ImageSpec::Badge {
                            icon: badge.icon.clone(),
                        };
                        let rich = details
                            .and_then(|details| details.detail_for(badge))
                            .map(|detail| BadgeTip::of(badge, detail));
                        // Warm the art before hover; the tooltip only renders what is cached.
                        if let Some(spec) = rich.as_ref().and_then(|tip| tip.art.as_ref()) {
                            images::resolve(spec, theme::image_px(BADGE_ART), cx);
                        }
                        let hovered = card.hovered_badge == Some(index);
                        let link = badge.link.clone();
                        let entity = app.entity.clone();
                        div()
                            .id(("profile-badge", index))
                            .relative()
                            .size(rpx(20.))
                            .when(link.is_some(), |this| this.cursor_pointer())
                            .on_hover(move |hovered, _, cx| {
                                entity
                                    .update(cx, |app, cx| app.hover_profile_badge(index, *hovered, cx))
                                    .ok();
                            })
                            .when(hovered, |this| match &rich {
                                Some(rich) => this.child(badge_tooltip(rich, cx)),
                                None => this.child(theme::tooltip_above(badge.description.clone())),
                            })
                            .on_click(move |_, _, cx| {
                                if let Some(link) = &link {
                                    cx.open_url(link);
                                }
                            })
                            .children(
                                images::resolve(&spec, theme::image_px(20.), cx)
                                    .map(|path| img(path).size_full()),
                            )
                    }),
            )
            .when(hidden > 0, |this| {
                this.child(
                    div()
                        .h(rpx(20.))
                        .min_w(rpx(20.))
                        .px(rpx(2.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(rpx(14.))
                        .text_color(rgb(palette.text))
                        .child(format!("+{hidden}")),
                )
            })
            .into_any_element(),
    )
}

fn popout_body(
    app: &DiscordApp,
    card: &ProfileCard,
    layout: &Layout,
    cx: &App,
) -> impl IntoElement {
    let user = app.store.user(card.user_id);
    let palette = card.palette();
    let entity = app.entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(rpx(12.))
        .px(rpx(layout.padding))
        .pt(rpx(4.))
        .pb(rpx(8.))
        .child(display_names(
            app,
            card,
            false,
            Some(Box::new(move |_, _, cx| {
                entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
            })),
            cx,
        ))
        .children(bio(app, card, true, cx))
        .children(board::popout_widget(app, card, cx))
        .children(
            board::popout_playing(app, card, cx)
                .or_else(|| user.and_then(|user| playing(user, palette))),
        )
        .children(roles(app, card))
}

fn bio(app: &DiscordApp, card: &ProfileCard, clamped: bool, cx: &App) -> Option<AnyElement> {
    let profile = card.profile.as_deref()?;
    let palette = card.palette();
    let text = guild_profile(profile)
        .and_then(|p| p.bio.clone())
        .filter(|bio| !bio.trim().is_empty())
        .or_else(|| profile.user_profile.as_ref()?.bio.clone())
        .filter(|bio| !bio.trim().is_empty())?;
    let parsed = app.parsed(card.user_id, &text);
    let context = BodyContext {
        store: &app.store,
        guild_id: card.guild_id.unwrap_or(0),
        message_id: card.user_id,
        revealed: &app.revealed_spoilers,
        entity: app.entity.clone(),
        colors: BodyColors {
            strong: palette.strong,
            muted: palette.muted,
            faint: palette.faint,
            link: palette.light.then_some(0x154C8A),
        },
    };
    Some(
        div()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .text_color(rgb(palette.strong))
            .when(clamped, |this| {
                this.max_h(rpx(18. * BIO_LINES as f32)).overflow_hidden()
            })
            .child(render_body(&parsed.blocks, false, false, &context, cx))
            .into_any_element(),
    )
}

fn playing(user: &User, palette: ProfilePalette) -> Option<AnyElement> {
    let activity = user.activity.as_ref().filter(|_| user.playing)?;
    Some(section(
        "Playing",
        div()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .p(rpx(12.))
            .rounded(rpx(8.))
            .bg(palette.panel)
            .border_1()
            .border_color(palette.border)
            .child(theme::icon("gamepad-2", 20., theme::GREEN))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.strong))
                    .child(label(("profile-playing", user.id), activity)),
            )
            .into_any_element(),
        palette,
    ))
}

fn roles(app: &DiscordApp, card: &ProfileCard) -> Option<AnyElement> {
    let palette = card.palette();
    let roles = app.store.member_roles(card.guild_id?, card.user_id);
    if roles.is_empty() {
        return None;
    }
    Some(section(
        "Roles",
        div()
            .flex()
            .flex_wrap()
            .gap(rpx(4.))
            .children(roles.into_iter().map(|role| {
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .min_h(rpx(24.))
                    .pl(rpx(6.))
                    .pr(rpx(8.))
                    .rounded(rpx(8.))
                    .border_1()
                    .border_color(palette.border)
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(palette.text))
                    .child(
                        div()
                            .size(rpx(12.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(role.color.unwrap_or(palette.muted))),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(label(("profile-role", role.id), &role.name)),
                    )
            }))
            .into_any_element(),
        palette,
    ))
}

fn section(title: &'static str, body: AnyElement, palette: ProfilePalette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .child(
            div()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.muted))
                .child(title),
        )
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(palette.text))
                .child(body),
        )
        .into_any_element()
}

fn member_since(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    let palette = card.palette();
    let format = |date: chrono::NaiveDate| date.format("%b %-d, %Y").to_string();
    let created = (card.user_id >> 22) as i64 + DISCORD_EPOCH_MS as i64;
    let created = chrono::DateTime::from_timestamp_millis(created).map(|date| date.date_naive());
    let joined = card
        .profile
        .as_deref()
        .and_then(|p| p.guild_member.as_ref()?.joined_at.as_deref())
        .and_then(|joined| chrono::DateTime::parse_from_rfc3339(joined).ok())
        .map(|date| date.date_naive());
    let guild = card.guild_id.and_then(|id| app.store.guild_by_id(id));
    section(
        "Member Since",
        div()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .children(created.map(|date| {
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .child(theme::icon("discord", 16., palette.muted))
                    .child(format(date))
            }))
            .children(joined.zip(guild).map(|(date, guild)| {
                let icon = guild
                    .icon_spec()
                    .and_then(|spec| images::resolve(&spec, theme::image_px(16.), cx));
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(8.))
                    .child(div().size(rpx(4.)).rounded_full().bg(rgb(palette.muted)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rpx(4.))
                            .child(match icon {
                                Some(path) => {
                                    img(path).size(rpx(16.)).rounded(rpx(4.)).into_any_element()
                                }
                                None => div()
                                    .size(rpx(16.))
                                    .rounded(rpx(4.))
                                    .bg(rgb(guild.icon_color))
                                    .into_any_element(),
                            })
                            .child(format(date)),
                    )
            }))
            .into_any_element(),
        palette,
    )
}

fn connections(profile: &model::Profile, palette: ProfilePalette) -> Option<AnyElement> {
    if profile.connected_accounts.is_empty() {
        return None;
    }
    Some(section(
        "Connections",
        div()
            .flex()
            .flex_col()
            .gap(rpx(16.))
            .children(profile.connected_accounts.iter().enumerate().map(|(index, account)| {
                let url = connection_url(account);
                div()
                    .id(("profile-connection", index))
                    .flex()
                    .items_center()
                    .gap(rpx(12.))
                    .min_w_0()
                    .child(
                        div()
                            .size(rpx(28.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(rpx(8.))
                            .bg(rgb(0xFFFFFF))
                            .map(|this| match connection_icon(&account.kind) {
                                Some((icon, color)) => this.child(theme::icon(icon, 18., color)),
                                None => this
                                    .text_size(rpx(13.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(0x2E3035))
                                    .child(connection_initial(&account.kind)),
                            }),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(rpx(16.))
                            .text_color(rgb(palette.strong))
                            .child(account.name.clone()),
                    )
                    .when_some(url, |this, url| {
                        this.cursor_pointer()
                            .child(theme::icon("arrow-up-right", 16., palette.strong))
                            .on_click(move |_, _, cx| cx.open_url(&url))
                    })
            }))
            .into_any_element(),
        palette,
    ))
}

/// Icon and brand color, drawn on the white connection tile.
fn connection_icon(kind: &str) -> Option<(&'static str, u32)> {
    Some(match kind {
        "github" => ("connections/github", 0x181717),
        "twitch" => ("connections/twitch", 0x9146FF),
        "youtube" => ("connections/youtube", 0xFF0000),
        "steam" => ("connections/steam", 0x000000),
        "spotify" => ("connections/spotify", 0x1ED760),
        "xbox" => ("connections/xbox", 0x107C10),
        "playstation" => ("connections/playstation", 0x0070D1),
        "reddit" => ("connections/reddit", 0xFF4500),
        "twitter" => ("connections/twitter", 0x000000),
        "domain" => ("connections/domain", 0x000000),
        "battlenet" => ("connections/battlenet", 0x148EFF),
        "leagueoflegends" => ("connections/leagueoflegends", 0xC28F2C),
        "tiktok" => ("connections/tiktok", 0x000000),
        "bluesky" => ("connections/bluesky", 0x0285FF),
        "instagram" => ("connections/instagram", 0xFF0069),
        "facebook" => ("connections/facebook", 0x0866FF),
        "epicgames" => ("connections/epicgames", 0x313131),
        "riotgames" => ("connections/riotgames", 0xEB0029),
        "roblox" => ("connections/roblox", 0x000000),
        "paypal" => ("connections/paypal", 0x002991),
        "ebay" => ("connections/ebay", 0xE53238),
        "crunchyroll" => ("connections/crunchyroll", 0xFF5E00),
        _ => return None,
    })
}

/// Public profile link for platforms Discord links out to; the rest render without the arrow.
fn connection_url(account: &model::ConnectedAccount) -> Option<String> {
    let (name, id) = (&account.name, &account.id);
    Some(match account.kind.as_str() {
        "github" => format!("https://github.com/{name}"),
        "twitch" => format!("https://www.twitch.tv/{name}"),
        "youtube" => format!("https://www.youtube.com/channel/{id}"),
        "steam" => format!("https://steamcommunity.com/profiles/{id}"),
        "spotify" => format!("https://open.spotify.com/user/{id}"),
        "reddit" => format!("https://www.reddit.com/u/{name}"),
        "twitter" => format!("https://x.com/{name}"),
        "domain" => format!("https://{name}"),
        "tiktok" => format!("https://www.tiktok.com/@{name}"),
        "bluesky" => format!("https://bsky.app/profile/{id}"),
        "instagram" => format!("https://www.instagram.com/{name}"),
        "roblox" => format!("https://www.roblox.com/users/{id}/profile"),
        "ebay" => format!("https://www.ebay.com/usr/{name}"),
        _ => return None,
    })
}

fn connection_initial(kind: &str) -> String {
    connection_name(kind)
        .chars()
        .next()
        .unwrap_or('?')
        .to_string()
}

fn profile_close_button(palette: ProfilePalette) -> Stateful<Div> {
    let (background, hover) = if palette.light {
        (hsla(0., 0., 1., 0.6), hsla(0., 0., 1., 0.85))
    } else {
        (rgba(0x00000066).into(), palette.hover)
    };
    theme::icon_button_with_hover("profile-close", "x", 18., palette.text, hover).bg(background)
}

fn modal_backdrop_corners(rem_size: Pixels) -> Corners<Pixels> {
    let radius = rpx(MODAL_RADIUS).to_pixels(rem_size);
    Corners {
        top_left: radius,
        top_right: radius,
        ..Corners::default()
    }
}

fn modal(app: &DiscordApp, card: &ProfileCard, window: &Window, cx: &App) -> AnyElement {
    let theme = card.theme();
    let palette = card.palette();
    let css = |pixels: Pixels| pixels / window.rem_size() * theme::BASE_FONT_SIZE;
    let viewport = window.viewport_size();
    let (viewport, viewport_height) = (css(viewport.width), css(viewport.height));
    let windowed = viewport >= MODAL_MIN_VIEWPORT_WIDTH;
    let modal_width = if windowed {
        MODAL_WIDTH.min(viewport - MODAL_MARGIN * 2.)
    } else {
        viewport
    };
    let modal_height = if viewport_height >= MODAL_MIN_VIEWPORT_HEIGHT {
        MODAL_HEIGHT.min(viewport_height - MODAL_MARGIN * 2.)
    } else {
        viewport_height
    };
    let card_inset = if windowed { MODAL_MARGIN * 0.91 } else { 0. };
    let card_width = (viewport * 0.455 - card_inset).clamp(300., 400.);
    let layout = Layout::modal(card_width);
    let surface = theme.map_or(theme::ELEVATED, ProfileTheme::surface);
    let background: Background =
        theme.map_or(rgb(theme::BG).into(), ProfileTheme::modal_background);
    let profile_card = div()
        .relative()
        .w(rpx(card_width))
        .h_full()
        .flex()
        .flex_col()
        .rounded_t(rpx(MODAL_CARD_RADIUS))
        .shadow(vec![
            BoxShadow::new(
                px(0.),
                px(2.),
                hsla(0., 0., 0., if palette.light { 0.16 } else { 0.3 }),
            )
            .blur_radius(px(8.)),
        ])
        .overflow_hidden()
        .map(|this| match theme {
            Some(theme) => this.bg(theme.outer()),
            None => this.bg(rgb(theme::ELEVATED)),
        })
        .child(
            div()
                .id("profile-modal-card")
                .flex()
                .flex_col()
                .size_full()
                .rounded_t(rpx(layout.corner))
                .text_color(rgb(palette.text))
                .map(|this| match theme {
                    Some(theme) => this.bg(theme.overlay()),
                    None => this,
                })
                .child(header(app, card, &layout, surface, None, cx))
                .child(
                    div()
                        .id("profile-modal-body")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .gap(rpx(24.))
                        .px(rpx(layout.padding))
                        .pb(rpx(layout.padding))
                        .overflow_y_scroll()
                        .child(display_names(app, card, true, None, cx))
                        .children(
                            bio(app, card, false, cx).map(|bio| section("About Me", bio, palette)),
                        )
                        .child(member_since(app, card, cx))
                        .children(roles(app, card))
                        .children(
                            card.profile
                                .as_deref()
                                .and_then(|profile| connections(profile, palette)),
                        )
                        .children(card.note.as_ref().map(|note| {
                            section(
                                "Note",
                                div()
                                    .text_size(rpx(14.))
                                    .line_height(rpx(18.))
                                    .text_color(rgb(palette.text))
                                    .child(note.clone())
                                    .into_any_element(),
                                palette,
                            )
                        })),
                ),
        )
        .children(effect_layers(card, card_width, cx))
        .into_any_element();
    let framed = framed(card, card_width, profile_card, true, cx);
    let banner_backdrop = card
        .profile
        .as_deref()
        .and_then(|p| p.user.banner.clone())
        .map(|hash| ImageSpec::Banner {
            user_id: card.user_id,
            hash,
        })
        .and_then(|spec| {
            let sigma = BACKDROP_BLUR * BACKDROP_SOURCE as f32 / modal_width;
            images::blurred_with(&spec, BACKDROP_SOURCE, sigma, BACKDROP_SATURATION, cx)
        })
        .map(|image| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(rpx(BACKDROP_HEIGHT))
                .overflow_hidden()
                .opacity(BACKDROP_OPACITY)
                .child(
                    canvas(
                        move |_, _, _| image,
                        |bounds, image, window, _| {
                            if bounds.is_empty() {
                                return;
                            }
                            let fitted = ObjectFit::Cover.get_bounds(bounds, image.size(0));
                            // Fade the image itself, leaving the profile theme visible underneath.
                            let fade = EdgeFade {
                                bounds,
                                band: bounds.size.height,
                                band_top: None,
                                band_bottom: None,
                                band_left: None,
                                band_right: None,
                                top: false,
                                bottom: true,
                                left: false,
                                right: false,
                            };
                            window.with_edge_fade(Some(fade), |window| {
                                if let Err(error) = window.paint_image_fitted(
                                    bounds,
                                    fitted,
                                    // Parent overflow clipping is rectangular, so round the image too.
                                    modal_backdrop_corners(window.rem_size()),
                                    image,
                                    0,
                                    false,
                                ) {
                                    log::warn!("Could not paint profile backdrop: {error}");
                                }
                            });
                        },
                    )
                    .size_full(),
                )
        });
    let close = app.entity.clone();
    let dismiss = app.entity.clone();
    let dialog = div()
        .id("profile-modal")
        .relative()
        .w(rpx(modal_width))
        .h(rpx(modal_height))
        .rounded(rpx(MODAL_RADIUS))
        .text_color(rgb(palette.text))
        .occlude()
        .without_text_selection()
        .on_click(|_, _, cx| cx.stop_propagation())
        // Clip the dialog surface, not the frame layers that extend beyond it.
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(rpx(MODAL_RADIUS))
                .overflow_hidden()
                .bg(background)
                .children(banner_backdrop),
        )
        .child(div().absolute().top(rpx(8.)).right(rpx(8.)).child(
            profile_close_button(palette).on_click(move |_, _, cx| {
                close.update(cx, |app, cx| app.close_profile(cx)).ok();
            }),
        ))
        .child(
            div()
                .size_full()
                .flex()
                .pt(rpx(32.))
                .pl(rpx(32.))
                .pr(rpx(16.))
                .child(framed)
                .child(side_panel(app, card, cx)),
        );
    div()
        .id("profile-modal-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_click(move |_, _, cx| {
            dismiss.update(cx, |app, cx| app.close_profile(cx)).ok();
        })
        .with_animation(
            ("profile-modal-backdrop", card.user_id),
            Animation::new(ENTER).with_easing(ease_out_quint()),
            |this, delta| this.bg(hsla(0., 0., 0., 0.7 * delta)),
        )
        .child(dialog.with_animation(
            ("profile-modal-enter", card.user_id),
            Animation::new(ENTER).with_easing(ease_out_quint()),
            |this, delta| this.opacity(delta).top(rpx(16. * (1. - delta))),
        ))
        .into_any_element()
}

fn side_panel(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    let profile = card.profile.as_deref();
    let palette = card.palette();
    let mutual_guilds: Vec<&model::MutualGuild> = profile
        .and_then(|p| p.mutual_guilds.as_ref())
        .into_iter()
        .flatten()
        .collect();
    let mutual_friends: Vec<&model::User> = profile
        .and_then(|p| p.mutual_friends.as_ref())
        .into_iter()
        .flatten()
        .collect();
    let friend_count = profile
        .and_then(|p| p.mutual_friends_count)
        .filter(|count| *count > 0)
        .map(|count| count as usize)
        .or_else(|| (!mutual_friends.is_empty()).then_some(mutual_friends.len()));
    let has_board = board::has_board(card);
    let has_wishlist = board::has_wishlist(card);
    let current = match card.tab {
        Tab::Board if !has_board => Tab::Activity,
        Tab::Wishlist if !has_wishlist => Tab::Activity,
        tab => tab,
    };
    let tab = |id: Tab, title: String| {
        let entity = app.entity.clone();
        let selected = current == id;
        div()
            .id(SharedString::from(format!("profile-tab-{}", id as u8)))
            .pb(rpx(12.))
            .border_b_2()
            .border_color(if selected {
                rgb(palette.strong)
            } else {
                rgba(0)
            })
            .cursor_pointer()
            .text_size(rpx(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(if selected {
                palette.strong
            } else {
                palette.muted
            }))
            .hover(move |style| style.text_color(rgb(palette.strong)))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.select_profile_tab(id, cx))
                    .ok();
            })
            .child(title)
    };
    let panel = match current {
        Tab::Board => board::board_panel(app, card, cx),
        Tab::Activity => activity_panel(app, card, cx),
        Tab::Wishlist => board::wishlist_panel(app, card, cx),
        Tab::Servers => servers_panel(app, &mutual_guilds, palette, cx),
        Tab::Friends => friends_panel(app, &mutual_friends, palette, cx),
    };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .pl(rpx(32.))
        .pt(rpx(20.))
        .child(
            div()
                .flex()
                .items_end()
                .gap(rpx(24.))
                .border_b_1()
                .border_color(palette.border)
                .when(has_board, |this| {
                    this.child(tab(Tab::Board, "Board".to_owned()))
                })
                .child(tab(Tab::Activity, "Activity".to_owned()))
                .when(has_wishlist, |this| {
                    this.child(tab(Tab::Wishlist, "Wishlist".to_owned()))
                })
                .when(!mutual_guilds.is_empty(), |this| {
                    this.child(tab(
                        Tab::Servers,
                        plural(mutual_guilds.len(), "Mutual Server"),
                    ))
                })
                .children(friend_count.map(|count| {
                    let title = plural(count, "Mutual Friend");
                    if mutual_friends.is_empty() {
                        div()
                            .pb(rpx(12.))
                            .text_size(rpx(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.faint))
                            .child(title)
                            .into_any_element()
                    } else {
                        tab(Tab::Friends, title).into_any_element()
                    }
                })),
        )
        .child(tab_panel(panel))
        .into_any_element()
}

fn tab_panel(panel: AnyElement) -> Stateful<Div> {
    div()
        .id("profile-tab-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .pt(rpx(20.))
        .pb(rpx(32.))
        .pr(rpx(16.))
        .overflow_y_scroll()
        .child(panel)
}

fn activity_panel(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    if let Some(activity) = board::activity(app, card, cx) {
        return activity;
    }
    let palette = card.palette();
    let user = app.store.user(card.user_id);
    if let Some(playing) = user.and_then(|user| playing(user, palette)) {
        return playing;
    }
    let name = user.map_or("This user", |user| user.name.as_str());
    activity_empty(name, &ACTIVITY_PROMPTS[card.activity_prompt], palette).into_any_element()
}

fn activity_empty(name: &str, prompt: &ActivityPrompt, palette: ProfilePalette) -> Div {
    div()
        .debug_selector(|| "profile-activity-empty".into())
        .flex()
        .flex_col()
        .flex_1()
        .w_full()
        .items_center()
        .justify_center()
        .px(rpx(16.))
        .py(rpx(24.))
        .child(
            div()
                .debug_selector(|| "profile-activity-copy".into())
                .flex()
                .flex_col()
                .w_full()
                .max_w(rpx(360.))
                .gap(rpx(8.))
                .text_center()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::NORMAL)
                .text_color(rgb(palette.text))
                .when(prompt.heading, |this| {
                    this.child(
                        div()
                            .debug_selector(|| "profile-activity-heading".into())
                            .w_full()
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("{name} doesn't have any activity to share here")),
                    )
                })
                .child(
                    div()
                        .debug_selector(|| "profile-activity-flavor".into())
                        .w_full()
                        .child(prompt.text),
                ),
        )
}

fn servers_panel(
    app: &DiscordApp,
    mutuals: &[&model::MutualGuild],
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .children(mutuals.iter().filter_map(|mutual| {
            let guild = app.store.guild_by_id(mutual.id)?;
            let icon = guild
                .icon_spec()
                .and_then(|spec| images::resolve(&spec, theme::image_px(40.), cx));
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(12.))
                    .p(rpx(8.))
                    .rounded(rpx(8.))
                    .hover(move |style| style.bg(palette.hover))
                    .child(match icon {
                        Some(path) => img(path)
                            .size(rpx(40.))
                            .rounded(rpx(12.))
                            .into_any_element(),
                        None => div()
                            .size(rpx(40.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(rpx(12.))
                            .bg(rgb(guild.icon_color))
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::BG))
                            .child(guild.initials.clone())
                            .into_any_element(),
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rpx(16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(palette.strong))
                                    .child(label(("mutual-guild", guild.id), &guild.name)),
                            )
                            .children(mutual.nick.as_ref().map(|nick| {
                                div()
                                    .truncate()
                                    .text_size(rpx(12.))
                                    .text_color(rgb(palette.muted))
                                    .child(label(("mutual-nick", guild.id), nick))
                            })),
                    ),
            )
        }))
        .into_any_element()
}

fn friends_panel(
    app: &DiscordApp,
    friends: &[&model::User],
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .children(friends.iter().map(|friend| {
            let name = friend
                .global_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or(&friend.username);
            div()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .p(rpx(8.))
                .rounded(rpx(8.))
                .hover(move |style| style.bg(palette.hover))
                .child(theme::user_avatar(
                    app.store.user(friend.id),
                    friend.id,
                    40.,
                    None,
                    false,
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(16.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.strong))
                                .child(label(("mutual-friend", friend.id), name)),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(12.))
                                .text_color(rgb(palette.muted))
                                .child(label(
                                    ("mutual-friend-username", friend.id),
                                    &friend.username,
                                )),
                        ),
                )
        }))
        .into_any_element()
}

fn profile_frame(card: &ProfileCard) -> Option<(u64, &model::CollectibleItem)> {
    let product = card.frame.as_ref()?;
    let item = product.items.iter().find(|item| item.kind == 3)?;
    Some((product.sku_id, item))
}

/// How far the profile frame's art reaches past a card `width` wide, in CSS pixels.
fn frame_overflow(card: &ProfileCard, width: f32) -> Option<Edges<f32>> {
    let (_, item) = profile_frame(card)?;
    let scale = width / item.inner_width.unwrap_or(1200).max(1) as f32;
    let horizontal = item.overflow_horizontal.unwrap_or(0) as f32 * scale;
    Some(Edges {
        top: item.overflow_top.unwrap_or(0) as f32 * scale,
        bottom: item.overflow_bottom.unwrap_or(0) as f32 * scale,
        left: horizontal,
        right: horizontal,
    })
}

fn framed(card: &ProfileCard, width: f32, body: AnyElement, fill: bool, cx: &App) -> AnyElement {
    let frame = profile_frame(card);
    let layers = |front: bool| -> Vec<AnyElement> {
        let (Some((sku, item)), Some(overflow)) = (frame, frame_overflow(card, width)) else {
            return Vec::new();
        };
        let overflow_x = overflow.left;
        let overflow_top = overflow.top;
        let overflow_bottom = overflow.bottom;
        let frame_width = width + overflow_x * 2.;
        item.layers
            .iter()
            .filter(|layer| (layer.order == "front") == front)
            // The full-profile dialog runs to the window's bottom edge, so Discord drops the
            // bottom staple there and keeps it only on the popout.
            .filter(|layer| !(fill && layer.kind == "staple" && layer.anchor == "bottom"))
            .filter_map(|layer| {
                let spec = ImageSpec::Collectible {
                    url: profile_frame_layer_url(sku, layer.id),
                };
                let path = images::resolve(&spec, theme::image_px(frame_width), cx)?;
                let image = img(path.clone()).w(rpx(frame_width)).flex_none();
                let base = div().absolute().left(rpx(-overflow_x)).w(rpx(frame_width));
                // Border art is a uniform strip of side rails with no fixed height; Discord repeats
                // it down the card and masks it out of the overflow, where the staples sit. Pin one
                // copy to each end of the card and let each fill half, so it fits any card up to
                // twice the art's height without overlapping.
                if layer.kind == "border" {
                    let half = |top: bool| {
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .map(|half| if top { half.justify_start() } else { half.justify_end() })
                            .child(img(path.clone()).w(rpx(frame_width)).flex_none())
                    };
                    return Some(
                        base.top_0()
                            .bottom_0()
                            .flex()
                            .flex_col()
                            .child(half(true))
                            .child(half(false))
                            .into_any_element(),
                    );
                }
                let base = match (layer.kind.as_str(), layer.anchor.as_str()) {
                    ("rail", anchor) => base
                        .top_0()
                        .bottom_0()
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .map(|rail| match anchor {
                            "top" => rail.justify_start(),
                            "bottom" => rail.justify_end(),
                            _ => rail.justify_center(),
                        }),
                    (_, "bottom") => base.bottom(rpx(-overflow_bottom)),
                    _ => base.top(rpx(-overflow_top)),
                };
                Some(base.child(image).into_any_element())
            })
            .collect()
    };
    div()
        .relative()
        .when(fill, |this| this.h_full())
        .children(layers(false))
        .child(body)
        .children(layers(true))
        .into_any_element()
}

fn connection_name(kind: &str) -> String {
    match kind {
        "github" => "GitHub".into(),
        "twitch" => "Twitch".into(),
        "youtube" => "YouTube".into(),
        "steam" => "Steam".into(),
        "spotify" => "Spotify".into(),
        "xbox" => "Xbox".into(),
        "playstation" => "PlayStation".into(),
        "reddit" => "Reddit".into(),
        "twitter" => "X".into(),
        "domain" => "Domain".into(),
        "battlenet" => "Battle.net".into(),
        "leagueoflegends" => "League".into(),
        "tiktok" => "TikTok".into(),
        "bluesky" => "Bluesky".into(),
        "instagram" => "Instagram".into(),
        "facebook" => "Facebook".into(),
        "epicgames" => "Epic Games".into(),
        "riotgames" => "Riot".into(),
        "roblox" => "Roblox".into(),
        "paypal" => "PayPal".into(),
        "ebay" => "eBay".into(),
        "crunchyroll" => "Crunchyroll".into(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn effect_start(started: &mut Option<Instant>, now: Instant) -> Instant {
    *started.get_or_insert(now)
}

enum EffectLoad<T> {
    Pending,
    Ready(T),
    Failed,
}

/// Waits only for pending layers; failed layers are skipped once nothing is pending.
fn settle_effect_layers<T>(layers: impl IntoIterator<Item = EffectLoad<T>>) -> Option<Vec<T>> {
    let mut pending = false;
    let mut ready = Vec::new();
    for layer in layers {
        match layer {
            EffectLoad::Pending => pending = true,
            EffectLoad::Ready(image) => ready.push(image),
            EffectLoad::Failed => {}
        }
    }
    (!pending).then_some(ready)
}

fn effect_layers(card: &ProfileCard, width: f32, cx: &App) -> Vec<AnyElement> {
    let Some(item) = card
        .effect
        .as_ref()
        .and_then(|product| product.items.iter().find(|item| item.kind == 1))
    else {
        return Vec::new();
    };
    let height = width * EFFECT_HEIGHT_RATIO;
    let states = item.effects.iter().enumerate().map(|(index, layer)| {
        match animated::state(
            &ImageSpec::Collectible {
                url: layer.src.clone(),
            },
            theme::image_px(height).min(EFFECT_MAX_PX),
            cx,
        ) {
            animated::Load::Pending => EffectLoad::Pending,
            animated::Load::Ready(image) => EffectLoad::Ready((index, image)),
            animated::Load::Failed => EffectLoad::Failed,
        }
    });
    // Showing a subset while another layer decodes makes intro layers pop in out of step.
    let Some(images) = settle_effect_layers(states) else {
        return Vec::new();
    };
    let started = effect_start(&mut card.effect_started.borrow_mut(), Instant::now());
    let elapsed = started.elapsed().as_millis() as u64;
    let mut layers: Vec<_> = images
        .into_iter()
        .filter_map(|(index, image)| {
            let layer = &item.effects[index];
            let start = u64::from(layer.start);
            let duration = u64::from(layer.duration).max(1);
            let since = elapsed.checked_sub(start)?;
            let replay = if layer.looped {
                let period = duration + u64::from(layer.loop_delay);
                (since % period < duration).then_some(since / period)?
            } else {
                (since < duration).then_some(0)?
            };
            Some((layer.z_index, index, replay, image))
        })
        .collect();
    layers.sort_by_key(|(z, ..)| *z);
    layers
        .into_iter()
        .map(|(_, index, replay, image)| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .child(
                    animated::image(image)
                        .id(SharedString::from(format!(
                            "profile-effect-{index}-{replay}"
                        )))
                        .w(rpx(width)),
                )
                .into_any_element()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        EffectLoad, ProfilePalette, ProfileTheme, THEME_OVERLAY, blend, contrast, effect_start,
        modal_backdrop_corners, outbox_application_ids, profile_close_button, settle_effect_layers,
    };
    use crate::discord::model;
    use gpui::{px, rgb};
    use std::time::{Duration, Instant};

    #[test]
    fn effect_layers_wait_for_pending_and_skip_failed() {
        use EffectLoad::{Failed, Pending, Ready};
        assert_eq!(settle_effect_layers([Ready(1), Pending]), None);
        assert_eq!(
            settle_effect_layers([Ready(1), Failed, Ready(2)]),
            Some(vec![1, 2])
        );
        assert_eq!(
            settle_effect_layers([Failed, Failed]),
            Some(Vec::<u8>::new())
        );

        let now = Instant::now();
        let mut started = None;
        assert_eq!(effect_start(&mut started, now), now);
        assert_eq!(
            effect_start(&mut started, now + Duration::from_secs(1)),
            now
        );
    }

    fn profile() -> model::Profile {
        serde_json::from_str(include_str!("../discord/fixtures/profile_modal.json")).unwrap()
    }

    #[test]
    fn recent_application_requests_skip_duplicates_and_non_game_entries() {
        let outbox = serde_json::from_value(serde_json::json!({"entries": [
            {"id": "1", "extra": {"type": "played_game_extra", "application_id": "7"}},
            {"id": "2", "extra": {"type": "played_game_extra", "application_id": "7"}},
            {"id": "3", "extra": {"type": "played_game_extra"}},
            {"id": "4", "extra": {"type": "listened_session_extra", "application_id": "8"}},
            {"id": "5", "extra": {"type": "played_game_extra", "application_id": "9"}}
        ]}))
        .unwrap();
        assert_eq!(outbox_application_ids(&outbox), [7, 9]);
    }

    #[test]
    fn blurred_banner_matches_dialog_top_corners_at_each_ui_scale() {
        for scale in [1., 1.25, 1.5, 2.] {
            let corners = modal_backdrop_corners(px(crate::theme::BASE_FONT_SIZE * scale));
            assert_eq!(corners.top_left, px(16. * scale));
            assert_eq!(corners.top_right, px(16. * scale));
            assert_eq!(corners.bottom_left, px(0.));
            assert_eq!(corners.bottom_right, px(0.));
        }
    }

    #[test]
    fn profile_theme_uses_global_colors_when_the_guild_has_no_theme() {
        let profile = profile();
        let theme = ProfileTheme::of(Some(&profile)).unwrap();
        assert_eq!((theme.primary, theme.secondary), (11687486, 14261667));
    }

    #[test]
    fn offline_user_keeps_cosmetics_and_theme_after_profile_loading() {
        let profile = profile();
        let ready = serde_json::from_str(include_str!("../discord/fixtures/ready.json")).unwrap();
        let mut store = crate::store::Store::from_ready(ready);
        store.absorb_profile(Some(1191168960364544011), &profile);
        let user = store.user(profile.user.user.id).unwrap();
        assert!(user.status == crate::store::Status::Offline);
        assert!(user.decoration.is_some());
        assert_eq!(
            user.name_style.as_ref().unwrap().colors,
            [16758138, 16749423, 16743544, 15756170]
        );
        assert_eq!(ProfileTheme::of(Some(&profile)).unwrap().primary, 11687486);
    }

    #[test]
    fn complete_guild_theme_overrides_global_colors() {
        let mut profile = profile();
        profile.guild_member_profile.as_mut().unwrap().theme_colors =
            Some(vec![0x336699, 0x993366]);
        let theme = ProfileTheme::of(Some(&profile)).unwrap();
        assert_eq!((theme.primary, theme.secondary), (0x336699, 0x993366));
    }

    #[test]
    fn incomplete_guild_theme_does_not_hide_the_global_theme() {
        let mut profile = profile();
        for colors in [vec![], vec![0x336699]] {
            profile.guild_member_profile.as_mut().unwrap().theme_colors = Some(colors);
            let theme = ProfileTheme::of(Some(&profile)).unwrap();
            assert_eq!((theme.primary, theme.secondary), (11687486, 14261667));
        }
        profile.user_profile.as_mut().unwrap().theme_colors = None;
        assert!(ProfileTheme::of(Some(&profile)).is_none());
        assert!(ProfileTheme::of(None).is_none());
    }

    #[test]
    fn nitro_profiles_without_custom_colors_use_the_avatar_palette() {
        let mut profile = profile();
        profile.user_profile.as_mut().unwrap().theme_colors = None;
        let avatar = ProfileTheme {
            primary: 0xDCD2AF,
            secondary: 0x292722,
        };
        assert!(ProfileTheme::needs_avatar(Some(&profile)));
        let resolved = ProfileTheme::resolve(Some(&profile), Some(avatar)).unwrap();
        assert_eq!(
            (resolved.primary, resolved.secondary),
            (avatar.primary, avatar.secondary)
        );
        assert!(ProfilePalette::of(Some(resolved)).light);
        assert!(ProfileTheme::resolve(Some(&profile), None).is_none());
        for premium_type in [None, Some(0), Some(1), Some(3)] {
            profile.premium_type = premium_type;
            assert!(!ProfileTheme::needs_avatar(Some(&profile)));
            assert!(ProfileTheme::resolve(Some(&profile), Some(avatar)).is_none());
        }
    }

    #[test]
    fn saved_colors_always_take_priority_over_an_avatar_palette() {
        let mut profile = profile();
        let avatar = ProfileTheme {
            primary: 0xDCD2AF,
            secondary: 0x292722,
        };
        let resolved = ProfileTheme::resolve(Some(&profile), Some(avatar)).unwrap();
        assert_eq!((resolved.primary, resolved.secondary), (11687486, 14261667));
        assert!(!ProfileTheme::needs_avatar(Some(&profile)));
        profile.guild_member_profile.as_mut().unwrap().theme_colors =
            Some(vec![0x112233, 0x445566]);
        let resolved = ProfileTheme::resolve(Some(&profile), Some(avatar)).unwrap();
        assert_eq!((resolved.primary, resolved.secondary), (0x112233, 0x445566));
    }

    #[test]
    fn dialog_background_matches_discord_lab_midpoint_and_lightness_limits() {
        // Reference values from Chroma's Lab mix followed by Discord's HSL lightness clamp.
        for (primary, secondary, expected) in [
            (0x336699, 0x993366, 0x1D141F),
            (0x339966, 0x993366, 0x1C1917),
            (0x8000FF, 0x00FFFF, 0x000B33),
            (0xA59BBC, 0x9387A9, 0xC9C3D5),
            (0x15223A, 0x30203B, 0x141221),
            (0x00FFFF, 0x8000FF, 0x99B0FF),
            (0, 0xFFFFFF, 0x191A19),
            (0xFFFFFF, 0, 0xCCCCCC),
        ] {
            let background = ProfileTheme { primary, secondary }.modal_background();
            let actual: gpui::Rgba = background
                .as_solid()
                .expect("dialog must not repeat the card gradient")
                .into();
            let expected = rgb(expected);
            for (actual, expected) in [
                (actual.r, expected.r),
                (actual.g, expected.g),
                (actual.b, expected.b),
            ] {
                assert!((actual - expected).abs() <= 1. / 255. + f32::EPSILON);
            }
        }
    }

    #[test]
    fn secondary_brightness_does_not_change_the_primary_color_mode() {
        for primary in [0x8000FF, 0x00C000, 0xFF0000, 0x7F7F7F] {
            for secondary in [0, 0x00FFFF, 0xFFFFFF] {
                let theme = ProfileTheme { primary, secondary };
                assert!(!theme.is_light());
                assert_eq!(theme.overlay(), gpui::hsla(0., 0., 0., THEME_OVERLAY));
            }
        }
        for secondary in [0, 0x8000FF, 0xFFFFFF] {
            assert!(
                ProfileTheme {
                    primary: 0x808080,
                    secondary
                }
                .is_light()
            );
        }
    }

    #[test]
    fn pale_profile_colors_use_a_light_overlay_instead_of_becoming_charcoal() {
        let theme = ProfileTheme {
            primary: 0xA59BBC,
            secondary: 0x9387A9,
        };
        assert!(theme.is_light());
        assert_eq!(theme.surface(), 0xDBD7E4);
        assert_eq!(theme.surface_color(theme.secondary), 0xD4CFDD);
        assert_eq!(theme.overlay(), gpui::hsla(0., 0., 1., THEME_OVERLAY));
    }

    #[test]
    fn dark_profile_colors_keep_the_dark_overlay() {
        let theme = ProfileTheme {
            primary: 0x15223A,
            secondary: 0x30203B,
        };
        assert!(!theme.is_light());
        assert_eq!(theme.surface(), blend(theme.primary, 0, THEME_OVERLAY));
        assert_eq!(theme.overlay(), gpui::hsla(0., 0., 0., THEME_OVERLAY));
    }

    #[test]
    fn profile_foreground_colors_follow_the_local_theme_not_the_app_theme() {
        let mut profile = profile();
        profile.user_profile.as_mut().unwrap().theme_colors = Some(vec![0xA59BBC, 0x9387A9]);
        let light = ProfilePalette::of(ProfileTheme::of(Some(&profile)));
        assert!(light.light);
        assert_eq!(light.strong, 0x18141F);
        profile.user_profile.as_mut().unwrap().theme_colors = Some(vec![0x15223A, 0x30203B]);
        let dark = ProfilePalette::of(ProfileTheme::of(Some(&profile)));
        assert!(!dark.light);
        assert_eq!(dark.strong, crate::theme::STRONG);
        profile.user_profile.as_mut().unwrap().theme_colors = Some(vec![0x8000FF, 0x00FFFF]);
        let saturated = ProfilePalette::of(ProfileTheme::of(Some(&profile)));
        assert!(!saturated.light);
        assert_eq!(saturated.strong, crate::theme::STRONG);
        let default = ProfilePalette::of(None);
        assert!(!default.light);
        assert_eq!(default.text, crate::theme::TEXT);
    }

    #[test]
    fn close_button_constructs_without_duplicate_hover_styles() {
        let mut profile = profile();
        for colors in [
            None,
            Some(vec![0xA59BBC, 0x9387A9]),
            Some(vec![0x15223A, 0x30203B]),
        ] {
            profile.user_profile.as_mut().unwrap().theme_colors = colors;
            let _button =
                profile_close_button(ProfilePalette::of(ProfileTheme::of(Some(&profile))));
        }
    }

    #[test]
    fn profile_text_has_readable_contrast_at_both_gradient_stops() {
        let mut profile = profile();
        for colors in [
            [0xA59BBC, 0x9387A9],
            [0x15223A, 0x30203B],
            [0xFFFFFF, 0],
            [0xFFFF00, 0x0000FF],
            [0x8000FF, 0x00FFFF],
        ] {
            profile.user_profile.as_mut().unwrap().theme_colors = Some(colors.to_vec());
            let palette = ProfilePalette::of(ProfileTheme::of(Some(&profile)));
            let theme = ProfileTheme::of(Some(&profile)).unwrap();
            for background in colors.map(|color| theme.surface_color(color)) {
                for foreground in [palette.strong, palette.text, palette.muted, palette.faint] {
                    assert!(contrast(foreground, background) >= 4.5);
                }
            }
        }
    }
}
