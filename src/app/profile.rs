use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};
use gpui_tokio::Tokio;

use crate::{
    app::{
        DISCORD_EPOCH_MS, DiscordApp,
        message_body::{BodyColors, BodyContext, label, render_body},
    },
    discord::{ProfileKind, Session, model},
    media::animated,
    media::images::{self, ImageSpec, profile_frame_layer_url},
    store::User,
    theme::{self, WithoutTextSelection, rpx},
};

mod account_logic;
#[cfg(test)]
mod activity_tests;
pub(super) mod avatar_colors;
mod badges;
mod board;
mod connections;
mod frame;
mod gate;
mod identity_editor;
mod image_editor;
mod loading;
mod modal;
mod mutuals;
mod palette;
pub(super) mod panel;
mod popout;
#[cfg(test)]
mod scope_tests;
pub(crate) mod styles;
mod styles_dialogs;
pub(crate) mod styles_panel;
mod widget_editor;
pub(crate) use account_logic::StatusChange;
pub(crate) use account_logic::{AccountAction, account_action};
pub(crate) use board::presence_art;
use loading::{widget_application_ids, widget_game_ids};
use modal::modal;
use palette::{ProfilePalette, ProfileTheme};
use popout::popout;

const POPOUT_WIDTH: f32 = 300.;
/// Gap between the account panel and the popout above it, in CSS pixels.
const ACCOUNT_GAP: f32 = 8.;
const MODAL_WIDTH: f32 = 962.;
const MODAL_HEIGHT: f32 = 800.;
const MODAL_MARGIN: f32 = 48.;
const MODAL_RADIUS: f32 = 16.;
const MODAL_CARD_RADIUS: f32 = 16.;
const MODAL_MIN_VIEWPORT_WIDTH: f32 = 929.;
const MODAL_MIN_VIEWPORT_HEIGHT: f32 = 785.;
const BANNER_SOURCE: u32 = 480;
const BACKDROP_BLUR: f32 = 15.;
const BACKDROP_SATURATION: f32 = 0.5;
const BACKDROP_OPACITY: f32 = 0.3;
const BACKDROP_FADE: f32 = 280.;
const EFFECT_TICK: Duration = Duration::from_millis(100);
const ENTER: Duration = Duration::from_millis(180);
/// Longest the popout shows its skeleton waiting for data before showing what it has.
const POPOUT_WAIT: Duration = Duration::from_millis(1500);
const EFFECT_HEIGHT_RATIO: f32 = 880. / 450.;
const EFFECT_MAX_PX: u32 = 1200;
const THEME_OVERLAY: f32 = 0.6;
const MAX_VISIBLE_BADGES: usize = 6;
const BADGE_ART: f32 = 96.;
const MUTUAL_ICON: f32 = 16.;
/// Each stacked mutual icon starts 3px before the previous one ends.
const MUTUAL_STEP: f32 = MUTUAL_ICON - 3.;
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

/// Live window-space bounds of the bottom account strip, recorded by the channel list on
/// prepaint and shared with the popout opened from it.
pub(crate) type AccountPanelBounds = Rc<Cell<Option<Bounds<Pixels>>>>;

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
    /// The DM side panel's card, stored in `dm_profile` rather than `profile`.
    panel: bool,
    guild_id: Option<u64>,
    /// Unlike `guild_id`, kept while viewing the main profile so the menu can switch back.
    pub(crate) source_guild_id: Option<u64>,
    pub(crate) more_menu_open: bool,
    hovered_action: Option<&'static str>,
    status_menu: popout::status::State,
    scope_snapshots: HashMap<Option<u64>, Arc<model::Profile>>,
    origin: ProfileOrigin,
    position: Point<Pixels>,
    anchor: Anchor,
    modal: bool,
    profile: Option<Arc<model::Profile>>,
    pub(crate) games: HashMap<u64, Arc<model::Game>>,
    games_pending: HashSet<u64>,
    pub(crate) applications: HashMap<u64, Arc<model::PublicApplication>>,
    applications_pending: HashSet<u64>,
    pub(crate) identities: Option<Arc<model::ApplicationIdentities>>,
    identities_pending: bool,
    pub(crate) outbox: Option<Arc<model::Outbox>>,
    pub(crate) wishlist: Option<Arc<model::Wishlist>>,
    pub(crate) note: Option<String>,
    pub(crate) badge_details: Option<Arc<model::UserBadges>>,
    hovered_badge: Option<usize>,
    /// `(widget id, game id)` of the Board cover under the pointer, for its name tooltip.
    pub(crate) hovered_cover: Option<(u64, u64)>,
    /// Whether the pointer is over each rich-presence image (indexed by `ArtSlot`), for their
    /// `large_text`/`small_text` tooltips. A small image sits on its large one, so both can be.
    pub(crate) hovered_art: [bool; 4],
    stack_hover: [StackHover; 2],
    pub(crate) widget_configs: HashMap<u64, Arc<model::WidgetConfig>>,
    widget_configs_pending: HashSet<u64>,
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
    /// Effect and frame SKUs still being fetched.
    cosmetics_pending: HashSet<u64>,
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
    commands: Vec<model::ApplicationCommand>,
    commands_requested: bool,
    commands_pending: bool,
    hovered_command: Option<u64>,
    _tasks: Vec<Task<()>>,
}

#[derive(Clone)]
enum ProfileOrigin {
    /// A member avatar or name; keeps its guild context and click-derived position.
    Member,
    /// The bottom account strip; uses the global profile and follows the strip's bounds.
    Account(AccountPanelBounds),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stack {
    Widget,
    Activity,
}

/// `generation` counts pointer enters and leaves, restarting the lift animation on each.
#[derive(Clone, Copy, Default)]
pub(crate) struct StackHover {
    hovered: bool,
    generation: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtSlot {
    Large,
    Small,
    ListeningLarge,
    ListeningSmall,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Board,
    Activity,
    Wishlist,
    Servers,
    Friends,
    Bio,
    DataAccess,
}

struct Layout {
    width: f32,
    banner: f32,
    avatar: f32,
    avatar_left: f32,
    ring: f32,
    padding: f32,
    status_left: f32,
    status_right: f32,
    status_overlap: f32,
    header_min: f32,
    corner: f32,
}

impl ProfileCard {
    pub(crate) fn can_switch_scope(&self) -> bool {
        self.source_guild_id.is_some()
    }

    /// Account popouts show account actions instead of server nickname and roles.
    pub(crate) fn is_account(&self) -> bool {
        matches!(&self.origin, ProfileOrigin::Account(_))
    }

    /// `overflow` is the frame overflow in CSS pixels, from `frame::frame_overflow`.
    pub(crate) fn popout_position(&self, window: &Window, overflow: &Edges<f32>) -> Point<Pixels> {
        let ProfileOrigin::Account(panel) = &self.origin else {
            return self.position;
        };
        let Some(panel) = panel.get() else {
            return self.position;
        };
        account_popout_position(panel, overflow, window.rem_size())
    }

    /// Reserves the frame's top overflow, the 12px viewport margin and the native client inset.
    /// `None` for member popouts or before the account strip has painted.
    pub(crate) fn account_max_height(
        &self,
        window: &Window,
        overflow: &Edges<f32>,
    ) -> Option<Pixels> {
        let ProfileOrigin::Account(panel) = &self.origin else {
            return None;
        };
        let panel = panel.get()?;
        Some(account_available_height(
            panel,
            overflow,
            window.rem_size(),
            window.client_inset().unwrap_or(px(0.)),
        ))
    }

    fn loading(&self) -> bool {
        if self.modal {
            self.profile.is_none() && self.modal_request_pending
        } else {
            !self.ready()
        }
    }

    /// Everything the popout draws has arrived (or failed), so it can fill in at once
    /// instead of growing as each request lands. `POPOUT_WAIT` caps the wait.
    fn ready(&self) -> bool {
        let settled = !self.modal_request_pending
            && !self.identities_pending
            && self.games_pending.is_empty()
            && self.applications_pending.is_empty()
            && self.widget_configs_pending.is_empty()
            && self.cosmetics_pending.is_empty()
            && !self.commands_pending
            && !(self.avatar_theme_source.is_some() && self.avatar_theme.is_none());
        settled || self.opened.elapsed() >= POPOUT_WAIT
    }

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
        application_ids.retain(|id| {
            !self.applications.contains_key(id) && !self.applications_pending.contains(id)
        });
        if application_ids.is_empty() {
            return;
        }
        let pending: HashSet<u64> = application_ids.iter().copied().collect();
        self.applications_pending.extend(pending.iter().copied());
        let (user_id, guild_id, panel) = (self.user_id, self.guild_id, self.panel);
        let metadata_session = session.clone();
        let request = Tokio::spawn(cx, async move {
            session.public_applications(&application_ids).await
        });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let applications = match request.await {
                Ok(Ok(applications)) => applications,
                Ok(Err(error)) => {
                    log::warn!("Could not load profile applications for user {user_id}: {error}");
                    Vec::new()
                }
                Err(error) => {
                    log::warn!("Profile applications task failed for user {user_id}: {error}");
                    Vec::new()
                }
            };
            this.update(cx, |app, cx| {
                let Some(card) = app
                    .profile_slot(panel)
                    .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                else {
                    return;
                };
                card.applications_pending.retain(|id| !pending.contains(id));
                if applications.is_empty() {
                    cx.notify();
                    return;
                }
                for application in applications {
                    card.applications.insert(application.id, application);
                }
                card.load_linked_metadata(metadata_session, cx);
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
        game_ids.retain(|id| !self.games.contains_key(id) && !self.games_pending.contains(id));
        if game_ids.is_empty() {
            return;
        }
        let pending: HashSet<u64> = game_ids.iter().copied().collect();
        self.games_pending.extend(pending.iter().copied());
        let (user_id, guild_id, panel) = (self.user_id, self.guild_id, self.panel);
        let metadata_session = session.clone();
        let request = Tokio::spawn(cx, async move { session.games(&game_ids).await });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let games = match request.await {
                Ok(Ok(games)) => games,
                Ok(Err(error)) => {
                    log::warn!("Could not load profile games for user {user_id}: {error}");
                    Vec::new()
                }
                Err(error) => {
                    log::warn!("Profile games task failed for user {user_id}: {error}");
                    Vec::new()
                }
            };
            this.update(cx, |app, cx| {
                let Some(card) = app
                    .profile_slot(panel)
                    .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                else {
                    return;
                };
                card.games_pending.retain(|id| !pending.contains(id));
                if games.is_empty() {
                    cx.notify();
                    return;
                }
                for game in games {
                    card.games.insert(game.id, game);
                }
                card.load_linked_metadata(metadata_session, cx);
                app.prefetch_profile_images(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn load_widget_configs(
        &mut self,
        session: Session,
        mut application_ids: Vec<u64>,
        cx: &mut Context<DiscordApp>,
    ) {
        application_ids.retain(|id| {
            !self.widget_configs.contains_key(id) && !self.widget_configs_pending.contains(id)
        });
        if application_ids.is_empty() {
            return;
        }
        self.widget_configs_pending
            .extend(application_ids.iter().copied());
        let (user_id, guild_id, panel) = (self.user_id, self.guild_id, self.panel);
        for application_id in application_ids {
            let session = session.clone();
            let request = Tokio::spawn(
                cx,
                async move { session.widget_config(application_id).await },
            );
            self._tasks.push(cx.spawn(async move |this, cx| {
                let config = match request.await {
                    Ok(Ok(config)) => config,
                    Ok(Err(error)) => {
                        log::warn!(
                            "Could not load widget config for application {application_id}: {error}"
                        );
                        None
                    }
                    Err(error) => {
                        log::warn!("Profile widget config task failed for user {user_id}: {error}");
                        None
                    }
                };
                this.update(cx, |app, cx| {
                    let Some(card) = app
                        .profile_slot(panel)
                        .filter(|card| card.user_id == user_id && card.guild_id == guild_id)
                    else {
                        return;
                    };
                    card.widget_configs_pending.remove(&application_id);
                    if let Some(config) = config {
                        card.widget_configs.insert(application_id, config);
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
    }

    /// Partner applications and their games can have different IDs; follow both directions.
    fn load_linked_metadata(&mut self, session: Session, cx: &mut Context<DiscordApp>) {
        let game_ids = self
            .applications
            .values()
            .flat_map(|application| application.linked_games.iter().map(|game| game.id))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let application_ids = self
            .games
            .values()
            .flat_map(|game| {
                game.linked_applications
                    .iter()
                    .map(|application| application.id)
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        self.load_games(session.clone(), game_ids, cx);
        self.load_applications(session, application_ids, cx);
    }
}

fn account_popout_position(
    panel: Bounds<Pixels>,
    overflow: &Edges<f32>,
    rem_size: Pixels,
) -> Point<Pixels> {
    let scale = |css: f32| rpx(css).to_pixels(rem_size);
    point(
        panel.left() + scale(overflow.left),
        panel.top() - scale(ACCOUNT_GAP) - scale(overflow.bottom),
    )
}

fn account_available_height(
    panel: Bounds<Pixels>,
    overflow: &Edges<f32>,
    rem_size: Pixels,
    client_inset: Pixels,
) -> Pixels {
    let scale = |css: f32| rpx(css).to_pixels(rem_size);
    (panel.top()
        - scale(ACCOUNT_GAP)
        - scale(overflow.bottom)
        - scale(overflow.top)
        - px(12.)
        - client_inset)
        .max(px(0.))
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
            status_right: 12.,
            status_overlap: 7.,
            // Banner + lower half of the avatar + ring + 8px breathing room; the
            // body's own 4px top padding brings the name 12px below the ring.
            header_min: 105. + 40. + 6. + 8. - inset,
            corner: if themed { 6. } else { 7. },
        }
    }

    fn panel(themed: bool) -> Self {
        let inset = if themed { THEME_BORDER } else { 0. };
        Self {
            width: panel::CARD_WIDTH - inset * 2.,
            banner: 120.,
            header_min: 120. + 40. + 6. + 8. - inset,
            ..Self::popout(themed)
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
            status_right: 16.,
            status_overlap: 8.,
            header_min: 140. + 60. + 20.,
            corner: MODAL_CARD_RADIUS,
        }
    }
}

pub(crate) fn shown(app: &DiscordApp, user_id: u64) -> impl Fn(&App) -> bool + 'static {
    super::menu::shown(app, move |app| {
        app.profile
            .as_ref()
            .is_some_and(|card| card.user_id == user_id && !card.modal)
    })
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

pub fn account_opener(
    app: &DiscordApp,
    user_id: u64,
    panel: AccountPanelBounds,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = app.entity.clone();
    move |_, window, cx| {
        cx.stop_propagation();
        // The panel paints before it can be clicked, so this is only a fallback for an
        // unpainted panel; never place the account popout at the pointer.
        let rem_size = window.rem_size();
        let position = panel.get().map_or_else(
            || point(px(0.), window.viewport_size().height),
            |panel| account_popout_position(panel, &Edges::<f32>::default(), rem_size),
        );
        entity
            .update(cx, |app, cx| {
                app.open_account_profile(user_id, position, panel.clone(), cx)
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

fn effect_playback(
    layer: &model::EffectLayer,
    started: Instant,
    now: Instant,
) -> Option<(u64, Instant)> {
    let elapsed = now.checked_duration_since(started)?.as_millis() as u64;
    let start = u64::from(layer.start);
    let duration = u64::from(layer.duration).max(1);
    let since = elapsed.checked_sub(start)?;
    if layer.looped && layer.loop_delay == 0 {
        // A continuous layer stays mounted: its APNG decides whether to loop or hold.
        return Some((0, started + Duration::from_millis(start)));
    }
    let (replay, cycle_start) = if layer.looped {
        let period = duration + u64::from(layer.loop_delay);
        if since % period >= duration {
            return None;
        }
        let replay = since / period;
        (replay, start + replay * period)
    } else {
        if since >= duration {
            return None;
        }
        (0, start)
    };
    Some((replay, started + Duration::from_millis(cycle_start)))
}

fn effect_finished(layer: &model::EffectLayer, started: Instant, now: Instant) -> bool {
    let end = u64::from(layer.start) + u64::from(layer.duration).max(1);
    !layer.looped
        && now
            .checked_duration_since(started)
            .is_some_and(|elapsed| elapsed.as_millis() as u64 >= end)
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

fn effect_clip_geometry(
    bounds: Bounds<Pixels>,
    corner: f32,
    top_only: bool,
    rem_size: Pixels,
) -> (Bounds<Pixels>, Pixels) {
    let radius = rpx(corner)
        .to_pixels(rem_size)
        .min(bounds.size.width / 2.)
        .min(bounds.size.height / 2.);
    let mut rounded = bounds;
    if top_only {
        // Keep the bottom arcs below the rectangular clip for the full-profile card.
        rounded.size.height += radius;
    }
    (rounded, radius)
}

fn effect_image(
    image: Arc<RenderImage>,
    id: SharedString,
    started_at: Instant,
    plays: Option<NonZeroU32>,
    corner: f32,
    top_only: bool,
) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let mut image = animated::image(image)
                .play_from(started_at, plays)
                .id(id)
                .w(bounds.size.width)
                .into_any_element();
            image.prepaint_as_root(
                bounds.origin,
                size(
                    AvailableSpace::Definite(bounds.size.width),
                    AvailableSpace::MaxContent,
                ),
                window,
                cx,
            );
            image
        },
        move |bounds, mut image, window, cx| {
            let visible = bounds.intersect(&window.content_mask().bounds);
            if visible.is_empty() {
                return;
            }
            let (rounded, radius) =
                effect_clip_geometry(visible, corner, top_only, window.rem_size());
            // Overflow clipping alone is rectangular; round only the effect's paint scope.
            window.with_content_mask(Some(ContentMask { bounds: visible }), |window| {
                window.with_rounded_clip(rounded, radius, |window| image.paint(window, cx));
            });
        },
    )
    .absolute()
    .inset_0()
}

fn effect_layers(
    card: &ProfileCard,
    width: f32,
    corner: f32,
    top_only: bool,
    cx: &App,
) -> Vec<AnyElement> {
    let Some(item) = card
        .effect
        .as_ref()
        .and_then(|product| product.items.iter().find(|item| item.kind == 1))
    else {
        return Vec::new();
    };
    effect_item_layers(item, &card.effect_started, width, corner, top_only, cx)
}

/// `item`'s layers over a card `width` wide, timed from `clock`, which starts once all of
/// them have decoded.
fn effect_item_layers(
    item: &model::CollectibleItem,
    clock: &RefCell<Option<Instant>>,
    width: f32,
    corner: f32,
    top_only: bool,
    cx: &App,
) -> Vec<AnyElement> {
    let height = width * EFFECT_HEIGHT_RATIO;
    let size = theme::image_px(height).min(EFFECT_MAX_PX);
    let now = cx.background_executor().now();
    let started = *clock.borrow();
    // Once the clock runs, a layer that has played its only time isn't asked for again, so
    // its frames can go; an intro is often the largest layer.
    let states = item
        .effects
        .iter()
        .enumerate()
        .filter(|(_, layer)| started.is_none_or(|started| !effect_finished(layer, started, now)))
        .map(|(index, layer)| {
            let spec = ImageSpec::Collectible {
                url: layer.src.clone(),
            };
            match animated::state(&spec, size, cx) {
                animated::Load::Pending => EffectLoad::Pending,
                animated::Load::Ready(image) => {
                    EffectLoad::Ready((index, image, animated::play_count(&spec, size, cx)))
                }
                animated::Load::Failed => EffectLoad::Failed,
            }
        });
    // Showing a subset while another layer decodes makes intro layers pop in out of step.
    let Some(images) = settle_effect_layers(states) else {
        return Vec::new();
    };
    let started = effect_start(&mut clock.borrow_mut(), now);
    let mut layers: Vec<_> = images
        .into_iter()
        .filter_map(|(index, image, plays)| {
            let layer = &item.effects[index];
            let (replay, layer_started) = effect_playback(layer, started, now)?;
            let plays = if layer.looped && layer.loop_delay == 0 {
                plays
            } else {
                Some(NonZeroU32::MIN)
            };
            Some((layer.z_index, index, replay, image, layer_started, plays))
        })
        .collect();
    layers.sort_by_key(|(z, ..)| *z);
    layers
        .into_iter()
        .map(|(_, index, replay, image, layer_started, plays)| {
            effect_image(
                image,
                format!("profile-effect-{index}-{replay}").into(),
                layer_started,
                plays,
                corner,
                top_only,
            )
            .into_any_element()
        })
        .collect()
}

/// The banner behind the full profile and the name style preview, as Discord draws it: at
/// half saturation, fitted to `width` and repeated down, faded out over its first 280px and
/// blurred from half to fully over the same span. `None` until the banner has loaded.
fn banner_backdrop(spec: &ImageSpec, width: f32, corner: f32, cx: &App) -> Option<AnyElement> {
    let pixels = theme::image_px(width);
    let sigma = BACKDROP_BLUR * pixels as f32 / width;
    let layer =
        |sigma: f32| images::blurred_at_width(spec, pixels, pixels, sigma, BACKDROP_SATURATION, cx);
    let (sharp, blurred) = (layer(0.)?, layer(sigma)?);
    // Discord's two masks multiply to (1 - t)² for the sharp copy, which is exactly gpui's
    // edge fade over the whole span, and to 1 - t² for the blurred one, which a shorter fade
    // under a flat top follows closely.
    Some(
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(rpx(BACKDROP_FADE))
            .child(faded_banner(
                sharp,
                BACKDROP_FADE,
                corner,
                BACKDROP_OPACITY / 2.,
            ))
            .child(faded_banner(
                blurred,
                BLURRED_BACKDROP_BAND,
                corner,
                BACKDROP_OPACITY / 2.,
            ))
            .into_any_element(),
    )
}

/// The band over which the blurred copy fades out, ending where the sharp one does.
const BLURRED_BACKDROP_BAND: f32 = 200.;

/// `image` repeated down the backdrop at full width, fading out over the `band` above its
/// bottom edge.
fn faded_banner(image: Arc<RenderImage>, band: f32, corner: f32, opacity: f32) -> impl IntoElement {
    div().absolute().inset_0().opacity(opacity).child(
        canvas(
            move |_, _, _| image,
            move |bounds, image, window, _| {
                if window.content_mask().bounds.is_empty() {
                    return;
                }
                let natural = image.size(0);
                let tile =
                    bounds.size.width * (natural.height.0 as f32 / natural.width.0.max(1) as f32);
                if tile <= px(0.) {
                    return;
                }
                let radius = rpx(corner).to_pixels(window.rem_size());
                let fade = EdgeFade {
                    bounds,
                    band: rpx(band).to_pixels(window.rem_size()),
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
                    let mut top = bounds.top();
                    let mut first = true;
                    while top < bounds.bottom() {
                        let tile_bounds =
                            Bounds::new(point(bounds.left(), top), size(bounds.size.width, tile));
                        let corners = Corners {
                            top_left: if first { radius } else { px(0.) },
                            top_right: if first { radius } else { px(0.) },
                            ..Corners::default()
                        };
                        window
                            .paint_image_fitted(
                                tile_bounds,
                                tile_bounds,
                                corners,
                                image.clone(),
                                0,
                                false,
                            )
                            .ok();
                        top += tile;
                        first = false;
                    }
                });
            },
        )
        .size_full(),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        EffectLoad, THEME_OVERLAY, account_available_height, account_popout_position,
        effect_clip_geometry, effect_finished, effect_playback, effect_start,
        loading::outbox_application_ids,
        modal::profile_close_button,
        palette::{ProfilePalette, ProfileTheme, blend, contrast},
        settle_effect_layers,
    };
    use crate::discord::model;
    use gpui::{Bounds, Edges, point, px, rgb, size};
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

    #[test]
    fn profile_effect_intro_plays_once_and_only_the_loop_restarts() {
        let products: Vec<model::CollectibleProduct> = serde_json::from_str(include_str!(
            "../../crates/discord/src/fixtures/collectibles_products.json"
        ))
        .unwrap();
        let effect = products.iter().find(|product| product.kind == 1).unwrap();
        let intro = &effect.items[0].effects[0];
        let repeat = &effect.items[0].effects[1];
        let started = Instant::now();
        let at = |ms| started + Duration::from_millis(ms);
        for ms in [0, 1000, 4980] {
            assert_eq!(effect_playback(intro, started, at(ms)), Some((0, started)));
            assert!(!effect_finished(intro, started, at(ms)));
        }
        for ms in [4981, 7981, 10_000, 15_961, 50_000] {
            assert_eq!(effect_playback(intro, started, at(ms)), None);
            assert!(effect_finished(intro, started, at(ms)));
            assert!(!effect_finished(repeat, started, at(ms)));
        }
        for ms in [0, 4981, 7980, 12_961, 15_960] {
            assert_eq!(effect_playback(repeat, started, at(ms)), None);
        }
        for ms in [7981, 10_000, 12_960] {
            assert_eq!(
                effect_playback(repeat, started, at(ms)),
                Some((0, at(7981)))
            );
        }
        for ms in [15_961, 16_000, 20_940] {
            assert_eq!(
                effect_playback(repeat, started, at(ms)),
                Some((1, at(15_961)))
            );
        }
        assert_eq!(effect_playback(repeat, started, at(20_941)), None);
        assert_eq!(effect_playback(intro, at(1), started), None);
    }

    #[test]
    fn profile_effect_zero_duration_never_divides_by_zero() {
        let layer = model::EffectLayer {
            src: String::new(),
            looped: true,
            duration: 0,
            start: 100,
            loop_delay: 10,
            z_index: 0,
        };
        let started = Instant::now();
        let at = |ms| started + Duration::from_millis(ms);
        assert_eq!(effect_playback(&layer, started, at(99)), None);
        assert_eq!(
            effect_playback(&layer, started, at(100)),
            Some((0, at(100)))
        );
        assert_eq!(effect_playback(&layer, started, at(101)), None);
        assert_eq!(effect_playback(&layer, started, at(110)), None);
        assert_eq!(
            effect_playback(&layer, started, at(111)),
            Some((1, at(111)))
        );
    }

    #[test]
    fn profile_effect_sakura_base_stays_mounted_while_the_overlay_repeats() {
        let base = model::EffectLayer {
            src: String::new(),
            looped: true,
            duration: 2880,
            start: 0,
            loop_delay: 0,
            z_index: 100,
        };
        let overlay = model::EffectLayer {
            start: 2880,
            loop_delay: 2880,
            z_index: 101,
            ..base.clone()
        };
        let started = Instant::now();
        let at = |ms| started + Duration::from_millis(ms);
        for ms in [0, 2879, 2880, 2999, 3000, 5760, 8640, 50_000] {
            assert_eq!(effect_playback(&base, started, at(ms)), Some((0, started)));
        }
        for ms in [0, 2879, 5760, 8639, 11_520] {
            assert_eq!(effect_playback(&overlay, started, at(ms)), None);
        }
        for ms in [2880, 3000, 5759] {
            assert_eq!(
                effect_playback(&overlay, started, at(ms)),
                Some((0, at(2880)))
            );
        }
        for ms in [8640, 10_000, 11_519] {
            assert_eq!(
                effect_playback(&overlay, started, at(ms)),
                Some((1, at(8640)))
            );
        }
        let delayed = model::EffectLayer { start: 100, ..base };
        assert_eq!(effect_playback(&delayed, started, at(99)), None);
        assert_eq!(
            effect_playback(&delayed, started, at(3000)),
            Some((0, at(100)))
        );
    }

    #[test]
    fn profile_effect_clip_matches_popout_and_modal_corners_at_each_scale() {
        for scale in [1., 1.125, 1.25, 1.5, 2.] {
            let rem_size = px(crate::theme::BASE_FONT_SIZE * scale);
            let bounds = Bounds::new(
                point(px(20. * scale), px(40. * scale)),
                size(px(300. * scale), px(420. * scale)),
            );
            for corner in [6., 8.] {
                let (rounded, radius) = effect_clip_geometry(bounds, corner, false, rem_size);
                assert_eq!(rounded, bounds);
                assert_eq!(radius, px(corner * scale));
            }
            let (rounded, radius) = effect_clip_geometry(bounds, 16., true, rem_size);
            assert_eq!(radius, px(16. * scale));
            assert_eq!(rounded.origin, bounds.origin);
            assert_eq!(rounded.size.width, bounds.size.width);
            assert_eq!(rounded.bottom() - radius, bounds.bottom());
        }
    }

    #[test]
    fn profile_effect_clip_uses_the_visible_scroll_viewport_and_clamps_its_radius() {
        let card = Bounds::new(point(px(20.), px(40.)), size(px(300.), px(600.)));
        let viewport = Bounds::new(point(px(20.), px(100.)), size(px(300.), px(200.)));
        let visible = card.intersect(&viewport);
        let (rounded, radius) = effect_clip_geometry(visible, 8., false, px(16.));
        assert_eq!(rounded, viewport);
        assert_eq!(radius, px(8.));

        let small = Bounds::new(point(px(0.), px(0.)), size(px(12.), px(8.)));
        let (rounded, radius) = effect_clip_geometry(small, 16., true, px(16.));
        assert_eq!(radius, px(4.));
        assert_eq!(rounded.bottom() - radius, small.bottom());
    }

    #[test]
    fn account_popout_sits_above_the_panel_and_clears_frame_overflow() {
        let panel = Bounds {
            origin: point(px(8.), px(600.)),
            size: size(px(287.), px(58.)),
        };
        let overflow = Edges {
            top: 0.,
            right: 4.,
            bottom: 12.,
            left: 4.,
        };
        assert_eq!(
            account_popout_position(panel, &overflow, px(16.)),
            point(px(12.), px(580.))
        );
        // The gap and overflow scale with zoom just like the card.
        assert_eq!(
            account_popout_position(panel, &overflow, px(24.)),
            point(px(14.), px(570.))
        );
        // Without a frame only the zoomed gap separates the card from the panel.
        assert_eq!(
            account_popout_position(panel, &Edges::default(), px(16.)),
            point(px(8.), px(592.))
        );
    }

    #[test]
    fn account_max_height_reserves_frame_overflow_gap_margin_and_zoom() {
        let panel = Bounds {
            origin: point(px(8.), px(600.)),
            size: size(px(287.), px(58.)),
        };
        let overflow = Edges {
            top: 10.,
            right: 4.,
            bottom: 12.,
            left: 4.,
        };
        // No frame: 600 - 8px gap - 12px viewport margin.
        assert_eq!(
            account_available_height(panel, &Edges::default(), px(16.), px(0.)),
            px(580.)
        );
        // 600 - 8 - 12 - 10 - 12.
        assert_eq!(
            account_available_height(panel, &overflow, px(16.), px(0.)),
            px(558.)
        );
        // At 1.5x zoom: 600 - 12 - 18 - 15 - 12.
        assert_eq!(
            account_available_height(panel, &overflow, px(24.), px(0.)),
            px(543.)
        );
        // The native inset is subtracted once, outside the zoomed lengths.
        assert_eq!(
            account_available_height(panel, &overflow, px(16.), px(6.)),
            px(552.)
        );
        // The result never goes negative near the top of the window.
        assert_eq!(
            account_available_height(
                Bounds {
                    origin: point(px(8.), px(10.)),
                    size: size(px(287.), px(58.)),
                },
                &Edges::default(),
                px(16.),
                px(0.),
            ),
            px(0.)
        );
    }

    fn profile() -> model::Profile {
        serde_json::from_str(include_str!(
            "../../crates/discord/src/fixtures/profile_modal.json"
        ))
        .unwrap()
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
    fn profile_theme_uses_global_colors_when_the_guild_has_no_theme() {
        let profile = profile();
        let theme = ProfileTheme::of(Some(&profile)).unwrap();
        assert_eq!((theme.primary, theme.secondary), (11687486, 14261667));
    }

    #[test]
    fn offline_user_keeps_cosmetics_and_theme_after_profile_loading() {
        let profile = profile();
        let ready =
            serde_json::from_str(include_str!("../../crates/discord/src/fixtures/ready.json"))
                .unwrap();
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
        assert_eq!(dark.strong, crate::theme::strong());
        profile.user_profile.as_mut().unwrap().theme_colors = Some(vec![0x8000FF, 0x00FFFF]);
        let saturated = ProfilePalette::of(ProfileTheme::of(Some(&profile)));
        assert!(!saturated.light);
        assert_eq!(saturated.strong, crate::theme::strong());
        let default = ProfilePalette::of(None);
        assert!(!default.light);
        assert_eq!(default.text, crate::theme::text());
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
