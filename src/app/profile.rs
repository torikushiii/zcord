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
        messages::message_body::{BodyColors, BodyContext, label, render_body},
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
const POPOUT_WAIT: Duration = Duration::from_millis(1500);
const EFFECT_HEIGHT_RATIO: f32 = 880. / 450.;
const EFFECT_MAX_PX: u32 = 1200;
const THEME_OVERLAY: f32 = 0.6;
const MAX_VISIBLE_BADGES: usize = 6;
const BADGE_ART: f32 = 96.;
const MUTUAL_ICON: f32 = 16.;
const MUTUAL_STEP: f32 = MUTUAL_ICON - 3.;
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
    panel: bool,
    guild_id: Option<u64>,
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
    pub(crate) hovered_cover: Option<(u64, u64)>,
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
    cosmetics_pending: HashSet<u64>,
    opened: Instant,
    effect_started: RefCell<Option<Instant>>,
    tab: Tab,
    status_prompt: usize,
    activity_prompt: usize,
    expanded_widgets: HashSet<u64>,
    pub(crate) widget_toggles: HashMap<u64, (u64, Instant)>,
    commands: Vec<model::ApplicationCommand>,
    commands_requested: bool,
    commands_pending: bool,
    hovered_command: Option<u64>,
    _tasks: Vec<Task<()>>,
}

#[derive(Clone)]
enum ProfileOrigin {
    Member,
    Account(AccountPanelBounds),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stack {
    Widget,
    Activity,
}

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

    pub(crate) fn is_account(&self) -> bool {
        matches!(&self.origin, ProfileOrigin::Account(_))
    }

    pub(crate) fn popout_position(&self, window: &Window, overflow: &Edges<f32>) -> Point<Pixels> {
        let ProfileOrigin::Account(panel) = &self.origin else {
            return self.position;
        };
        let Some(panel) = panel.get() else {
            return self.position;
        };
        account_popout_position(panel, overflow, window.rem_size())
    }

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

fn banner_backdrop(spec: &ImageSpec, width: f32, corner: f32, cx: &App) -> Option<AnyElement> {
    let pixels = theme::image_px(width);
    let sigma = BACKDROP_BLUR * pixels as f32 / width;
    let layer =
        |sigma: f32| images::blurred_at_width(spec, pixels, pixels, sigma, BACKDROP_SATURATION, cx);
    let (sharp, blurred) = (layer(0.)?, layer(sigma)?);
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

const BLURRED_BACKDROP_BAND: f32 = 200.;

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
mod tests;
