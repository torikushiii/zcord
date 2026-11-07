use gpui::{prelude::*, *};
use gpui_component::Theme;
use std::{
    f32::consts::{PI, TAU},
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{
    media::animated,
    media::images::{self, ImageSpec},
    settings,
    store::{Decoration, GuildTag, Status, Store, User, WebhookAuthor},
};

mod components;
mod dark;
mod light;
mod profile_avatar;

pub use components::init;
use dark::DARK;
use light::LIGHT;
pub use profile_avatar::status_icon;

macro_rules! palette {
    ($($name:ident),* $(,)?) => {
        pub struct Palette {
            $(pub $name: u32,)*
        }

        $(
            pub fn $name() -> u32 {
                palette().$name
            }
        )*
    };
}

palette!(
    bg,
    base_lower,
    base_lowest,
    surface_high,
    elevated,
    surface_highest,
    border,
    border_subtle,
    border_muted,
    border_strong,
    hover,
    active,
    selected,
    control,
    code_background,
    placeholder_strong,
    text,
    text_selection,
    strong,
    muted,
    faint,
    interactive_muted,
    link,
    focus,
    tooltip_bg,
    tooltip_ring,
    outline,
    border_soft,
    border_emphasis,
    border_bright,
    divider,
    recessed,
    control_bg,
    control_selected,
    dialog_bg,
    dialog_border,
    input_bg,
    input_border,
    button_bg,
    button_border,
    button_subtle,
    button_subtle_hover,
    toolbar_bg,
    toolbar_border,
    toolbar_hover,
    connector,
    embed_border,
    text_secondary,
    bright,
    control_thumb,
    icon_bg,
    spoiler_bg,
    spoiler_hover,
    spoiler_revealed_bg,
    timestamp_bg,
    menu_border,
    shadow,
    tooltip_shadow,
    control_shadow,
    mention_text,
    mention_bg,
    reaction_selected_bg,
    reply_bg,
    mention_row_bg,
    mention_row_hover,
    jump_highlight,
    connection_tile_bg,
    connection_tile_text,
    upload_sheet,
    upload_fold,
    accent_border,
    account_hover,
);

static LIGHT_ACTIVE: AtomicBool = AtomicBool::new(false);

fn palette() -> &'static Palette {
    if is_light() { &LIGHT } else { &DARK }
}

/// The dark palette whatever the app's theme, for surfaces that stay dark in light mode, like a
/// profile with a dark custom theme.
pub fn dark_palette() -> &'static Palette {
    &DARK
}

pub fn is_light() -> bool {
    LIGHT_ACTIVE.load(Ordering::Relaxed)
}

pub fn set_light(light: bool, cx: &mut App) {
    settings::update(cx, |settings| settings.light_theme = light);
    if LIGHT_ACTIVE.swap(light, Ordering::Relaxed) == light {
        return;
    }
    components::init(cx);
    Theme::global_mut(cx).font_size = px(BASE_FONT_SIZE * ZOOM_LEVELS[zoom_index(cx)]);
    cx.refresh_windows();
}

pub const SCRIM: u32 = 0x000000B8;
pub const LIGHTBOX_SCRIM: u32 = 0x000000EB;
pub const BLURPLE: u32 = 0x5865F2;
pub const BLURPLE_ACTIVE: u32 = 0x3A48A3;
pub const RED: u32 = 0xEB5F5E;
pub const GREEN: u32 = 0x41A063;
pub const YELLOW: u32 = 0xCE8100;
pub const NOTIFICATION: u32 = 0xD22D39;
pub const STATUS_ONLINE: u32 = 0x3D9E60;
pub const STATUS_IDLE: u32 = 0xFFCB6E;
pub const STATUS_DND: u32 = 0xDC4247;
pub const STATUS_OFFLINE: u32 = 0x9D9EA5;

pub const BASE_FONT_SIZE: f32 = 16.;
pub const ZOOM_LEVELS: [f32; 11] = [0.5, 0.67, 0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2.];
pub const DEFAULT_ZOOM: usize = 5;

struct Zoom(usize);

impl Global for Zoom {}

pub fn set_zoom(index: usize, cx: &mut App) {
    let index = index.min(ZOOM_LEVELS.len() - 1);
    cx.set_global(Zoom(index));
    settings::update(cx, |settings| settings.zoom = Some(index));
    Theme::global_mut(cx).font_size = px(BASE_FONT_SIZE * ZOOM_LEVELS[index]);
    cx.refresh_windows();
}

pub fn zoom_index(cx: &App) -> usize {
    cx.try_global::<Zoom>().map_or(DEFAULT_ZOOM, |zoom| zoom.0)
}

pub fn rpx(css_pixels: f32) -> Rems {
    rems(css_pixels / BASE_FONT_SIZE)
}

pub fn user_color(id: u64) -> Hsla {
    hsla(((id % 360) * 47 % 360) as f32 / 360., 0.55, 0.67, 1.)
}

pub fn status_color(status: Status) -> u32 {
    match status {
        Status::Online => STATUS_ONLINE,
        Status::Idle => STATUS_IDLE,
        Status::DoNotDisturb => STATUS_DND,
        Status::Offline => STATUS_OFFLINE,
    }
}

/// Avatar/status cutout rings need the opaque result of a row's translucent layer.
pub fn composite(overlay: u32, background: u32) -> u32 {
    let color = rgb(background).blend(rgba(overlay));
    let channel = |value: f32| (value * 255.).round() as u32;
    (channel(color.r) << 16) | (channel(color.g) << 8) | channel(color.b)
}

pub fn logo(size: f32) -> Img {
    img(crate::assets::LOGO_PATH).size(rpx(size)).flex_none()
}

pub fn icon(name: &str, size: f32, color: u32) -> Svg {
    svg()
        .path(SharedString::from(format!("app/{name}.svg")))
        .size(rpx(size))
        .flex_none()
        .text_color(rgb(color))
}

/// Device pixels per CSS pixel (display scale × zoom), as `f32` bits. Starts at 2 so images
/// requested before the first root render are sharp on HiDPI screens.
static IMAGE_SCALE: AtomicU32 = AtomicU32::new(0x4000_0000);

pub fn set_image_scale(window: &Window, cx: &App) {
    let scale = window.scale_factor() * ZOOM_LEVELS[zoom_index(cx)];
    IMAGE_SCALE.store(scale.to_bits(), Ordering::Relaxed);
}

pub fn image_px(css: f32) -> u32 {
    (css * f32::from_bits(IMAGE_SCALE.load(Ordering::Relaxed))).ceil() as u32
}

pub fn user_avatar(
    user: Option<&User>,
    user_id: u64,
    size: f32,
    status: Option<(Status, u32)>,
    animate: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let notch = profile_avatar::Geometry::for_avatar(size, status.is_some());
    let image = user.and_then(|user| avatar_image(&user.avatar_spec(), size, animate, notch, cx));
    avatar(
        user.map_or("?", |user| &user.initials),
        user_color(user_id),
        size,
        status,
        user.is_some_and(|user| user.on_mobile),
        user_id,
        image,
        user.and_then(|user| user.decoration.as_ref())
            .and_then(|data| decoration(user_id, data, size, animate, notch, cx)),
    )
}

/// A plain round avatar for `spec`, for users known only from an API response.
pub fn spec_avatar(
    spec: &ImageSpec,
    user_id: u64,
    name: &str,
    size: f32,
    cx: &App,
) -> impl IntoElement + use<> {
    let initials: String = name.chars().take(2).collect();
    avatar(
        &initials,
        user_color(user_id),
        size,
        None,
        false,
        user_id,
        avatar_image(spec, size, false, None, cx),
        None,
    )
}

pub fn member_avatar(
    store: &Store,
    guild_id: u64,
    user_id: u64,
    size: f32,
    status: Option<(Status, u32)>,
    animate: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    member_avatar_with(store, guild_id, user_id, size, status, animate, true, cx)
}

pub fn webhook_avatar(
    author_id: u64,
    author: &WebhookAuthor,
    size: f32,
    animate: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let spec = crate::store::avatar_spec(author_id, author.avatar.as_deref(), Some("0000"));
    let image = avatar_image(&spec, size, animate, None, cx);
    let initials: String = author.name.chars().take(2).collect();
    avatar(
        &initials,
        user_color(author_id),
        size,
        None,
        false,
        author_id,
        image,
        None,
    )
}

/// A local `file` wins over `spec`; profile style previews show edits before they're uploaded.
pub fn preview_avatar(
    user: Option<&User>,
    user_id: u64,
    size: f32,
    status: Option<(Status, u32)>,
    spec: Option<ImageSpec>,
    file: Option<std::path::PathBuf>,
    decorated_with: Option<&Decoration>,
    cx: &App,
) -> impl IntoElement + use<> {
    let notch = profile_avatar::Geometry::for_avatar(size, status.is_some());
    let image = match file {
        Some(path) => Some(img(path).size_full().rounded_full().into_any_element()),
        None => spec
            .or_else(|| user.map(User::avatar_spec))
            .and_then(|spec| avatar_image(&spec, size, true, notch, cx)),
    };
    avatar(
        user.map_or("?", |user| &user.initials),
        user_color(user_id),
        size,
        status,
        user.is_some_and(|user| user.on_mobile),
        user_id,
        image,
        decorated_with.and_then(|data| decoration(user_id, data, size, true, notch, cx)),
    )
}

/// Discord's grey stand-in avatar, which style previews use to show decorations on their own.
pub fn placeholder_avatar(
    size: f32,
    decorated_with: Option<&Decoration>,
    cx: &App,
) -> impl IntoElement + use<> {
    let image = div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(0x5C5E66))
        .child(icon("discord", size * 0.6, 0x3B3D44))
        .into_any_element();
    avatar(
        "",
        hsla(0., 0., 0., 0.),
        size,
        None,
        false,
        0,
        Some(image),
        decorated_with.and_then(|data| decoration(0, data, size, true, None, cx)),
    )
}

/// Discord leaves avatar decorations off small avatars such as voice channel members.
pub fn undecorated_member_avatar(
    store: &Store,
    guild_id: u64,
    user_id: u64,
    size: f32,
    cx: &App,
) -> impl IntoElement + use<> {
    member_avatar_with(store, guild_id, user_id, size, None, false, false, cx)
}

#[allow(clippy::too_many_arguments)]
fn member_avatar_with(
    store: &Store,
    guild_id: u64,
    user_id: u64,
    size: f32,
    status: Option<(Status, u32)>,
    animate: bool,
    decorated: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let user = store.user(user_id);
    let spec = store
        .member_avatar_spec(guild_id, user_id)
        .or_else(|| user.map(User::avatar_spec));
    let notch = profile_avatar::Geometry::for_avatar(size, status.is_some());
    let image = spec.and_then(|spec| avatar_image(&spec, size, animate, notch, cx));
    avatar(
        user.map_or("?", |user| &user.initials),
        user_color(user_id),
        size,
        status,
        user.is_some_and(|user| user.on_mobile),
        user_id,
        image,
        store
            .member_decoration(guild_id, user_id)
            .filter(|_| decorated)
            .and_then(|data| decoration(user_id, data, size, animate, notch, cx)),
    )
}

fn avatar_image(
    spec: &ImageSpec,
    size: f32,
    animate: bool,
    notch: Option<profile_avatar::Geometry>,
    cx: &App,
) -> Option<AnyElement> {
    let pixels = image_px(size);
    let animated = (animate || crate::settings::always_animate())
        .then(|| spec.animated())
        .flatten()
        .and_then(|animated| animated::resolve(&animated, pixels, cx));
    if let Some(notch) = notch {
        let source = match animated {
            Some(image) => profile_avatar::Source::Decoded(image),
            None => profile_avatar::Source::File(images::resolve(spec, pixels, cx)?),
        };
        return Some(profile_avatar::image(
            "profile-avatar-image",
            source,
            notch,
            false,
        ));
    }
    Some(match animated {
        Some(image) => animated::image(image)
            .size_full()
            .rounded_full()
            .id("avatar-animated")
            .into_any_element(),
        None => img(images::resolve(spec, pixels, cx)?)
            .size_full()
            .rounded_full()
            .into_any_element(),
    })
}

const SPINNER_RADIUS: f32 = 7.;
const SPINNER_STROKE: f32 = 2.;
const SPINNER_CIRCUMFERENCE: f32 = 44.;
const SPINNER_LONGEST_GAP: f32 = 11.;
const SPINNER_TURN_SECS: f32 = 0.5;
const SPINNER_DASH_SECS: f32 = 0.8;
const SPINNER_STEPS: usize = 48;
const SPINNER_CAP_STEPS: usize = 8;

pub fn media_loading() -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(spinner(32., strong()))
}

pub fn spinner(size: f32, color: u32) -> impl IntoElement {
    static START: OnceLock<Instant> = OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            window.request_animation_frame();
            let unit = bounds.size.width / px(16.);
            let elapsed = start.elapsed().as_secs_f32();
            let cycle = (elapsed / SPINNER_DASH_SECS).rem_euclid(2.);
            let growth = if cycle <= 1. { cycle } else { 2. - cycle };
            let gap = SPINNER_LONGEST_GAP + (SPINNER_CIRCUMFERENCE - SPINNER_LONGEST_GAP) * growth;
            let sweep = (SPINNER_CIRCUMFERENCE - gap) / SPINNER_RADIUS;
            let from = (elapsed / SPINNER_TURN_SECS).fract() * TAU;
            let center = bounds.center();
            let at = |radial: f32, tangent: f32, angle: f32| {
                let (sin, cos) = angle.sin_cos();
                center
                    + point(
                        px((radial * cos - tangent * sin) * unit),
                        px((radial * sin + tangent * cos) * unit),
                    )
            };
            let half = SPINNER_STROKE / 2.;
            let step = |index: usize, steps: usize| index as f32 / steps as f32;
            let outline = (0..=SPINNER_STEPS)
                .map(|index| {
                    at(
                        SPINNER_RADIUS + half,
                        0.,
                        from + sweep * step(index, SPINNER_STEPS),
                    )
                })
                .chain((1..SPINNER_CAP_STEPS).map(|index| {
                    let (sin, cos) = (step(index, SPINNER_CAP_STEPS) * PI).sin_cos();
                    at(SPINNER_RADIUS + half * cos, half * sin, from + sweep)
                }))
                .chain((0..=SPINNER_STEPS).rev().map(|index| {
                    at(
                        SPINNER_RADIUS - half,
                        0.,
                        from + sweep * step(index, SPINNER_STEPS),
                    )
                }))
                .chain((1..SPINNER_CAP_STEPS).map(|index| {
                    let (sin, cos) = (step(index, SPINNER_CAP_STEPS) * PI).sin_cos();
                    at(SPINNER_RADIUS - half * cos, -half * sin, from)
                }));
            let mut path = PathBuilder::fill();
            for (index, position) in outline.enumerate() {
                if index == 0 {
                    path.move_to(position);
                } else {
                    path.line_to(position);
                }
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(color));
            }
        },
    )
    .flex_none()
    .size(rpx(size))
}

const DECORATION_SCALE: f32 = 1.2;

fn decoration(
    user_id: u64,
    decoration: &Decoration,
    size: f32,
    animate: bool,
    notch: Option<profile_avatar::Geometry>,
    cx: &App,
) -> Option<AnyElement> {
    if size < 24. {
        return None;
    }
    let frame = size * DECORATION_SCALE;
    let pixels = image_px(frame);
    let animates =
        (animate || crate::settings::always_animate()) && decoration.asset.starts_with("a_");
    let animated = animates
        .then(|| animated::resolve(&decoration.image_spec(true), pixels, cx))
        .flatten();
    let still = if animated.is_none() {
        images::resolve(&decoration.image_spec(false), pixels, cx)
    } else {
        None
    };
    let image = if let Some(notch) = notch {
        let source = match animated {
            Some(image) => profile_avatar::Source::Decoded(image),
            None => profile_avatar::Source::File(still?),
        };
        profile_avatar::image(("decoration", user_id), source, notch, true)
    } else {
        match animated {
            Some(image) => animated::image(image)
                .id(("decoration", user_id))
                .size_full()
                .into_any_element(),
            None => img(still?).size_full().into_any_element(),
        }
    };
    Some(
        div()
            .absolute()
            .left(rpx((size - frame) / 2.))
            .top(rpx((size - frame) / 2.))
            .size(rpx(frame))
            .child(image)
            .into_any_element(),
    )
}

/// The icon Discord puts before a user's activity line: a gamepad while playing, a note while
/// listening.
pub fn activity_icon(user: &User) -> Option<&'static str> {
    if user.playing {
        Some("gamepad-2")
    } else if user.activity_kind == Some(2) {
        Some("music")
    } else {
        None
    }
}

pub fn avatar(
    initials: &str,
    color: Hsla,
    size: f32,
    status: Option<(Status, u32)>,
    on_mobile: bool,
    user_id: u64,
    image: Option<AnyElement>,
    decoration: Option<AnyElement>,
) -> impl IntoElement + use<> {
    let notch = profile_avatar::Geometry::for_avatar(size, status.is_some());
    let dot = (size * 0.3).round();
    div()
        .debug_selector(|| "user-avatar".into())
        .relative()
        .flex_none()
        .size(rpx(size))
        .child(match image {
            Some(image) => image,
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .when(notch.is_none(), |this| this.bg(color))
                .children(notch.map(|notch| profile_avatar::background(notch, color)))
                .text_color(rgb(bg()))
                .text_size(rpx(size * 0.36))
                .font_weight(FontWeight::SEMIBOLD)
                .child(initials.to_owned())
                .into_any_element(),
        })
        .children(decoration)
        .children(status.map(|(status, ring)| {
            let phone = on_mobile && status == Status::Online;
            if let Some(notch) = notch {
                return if phone {
                    profile_avatar::mobile_indicator(notch, ring)
                } else {
                    profile_avatar::indicator(notch, status, ring)
                };
            }
            if phone {
                return div()
                    .debug_selector(|| "avatar-status".into())
                    .absolute()
                    .right(rpx(-2.))
                    .bottom(rpx(-2.))
                    .p(rpx(2.))
                    .rounded(rpx(dot * 0.3))
                    .bg(rgb(ring))
                    .child(mobile_status(dot, ring))
                    .into_any_element();
            }
            div()
                .debug_selector(|| "avatar-status".into())
                .absolute()
                .right(rpx(-2.))
                .bottom(rpx(-2.))
                .size(rpx(dot + 4.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .border_2()
                .border_color(rgb(ring))
                .bg(rgb(status_color(status)))
                .children(status_cutout(status, dot, ring))
                .into_any_element()
        }))
        .children(status.map(|(status, _)| StatusTooltip {
            user_id,
            target: (dot + 4.).min(size),
            label: status.label(),
        }))
}

/// A hover target over the status indicator, holding its own hover so that avatars don't need
/// app state for it.
#[derive(IntoElement)]
struct StatusTooltip {
    user_id: u64,
    target: f32,
    label: &'static str,
}

impl RenderOnce for StatusTooltip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = ("status-tooltip", self.user_id);
        let hovered = window.use_keyed_state(key.clone(), cx, |_, _| false);
        let shown = *hovered.read(cx);
        div()
            .id(key)
            .absolute()
            .right(rpx(-2.))
            .bottom(rpx(-2.))
            .size(rpx(self.target))
            .on_hover(move |over, _, cx| {
                hovered.update(cx, |hovered, cx| {
                    *hovered = *over;
                    cx.notify();
                });
            })
            .when(shown, |this| this.child(tooltip_above(self.label)))
    }
}

/// Discord's phone mask: a body 1.5 times as tall as wide, with a screen and a home button.
fn mobile_status(width: f32, surface: u32) -> Div {
    let height = width * 1.5;
    div()
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(width * 0.1875))
        .bg(rgb(STATUS_ONLINE))
        .child(
            div()
                .absolute()
                .left(rpx(width * 0.125))
                .top(rpx(height / 6.))
                .w(rpx(width * 0.75))
                .h(rpx(height * 0.5))
                .bg(rgb(surface)),
        )
        .child(
            div()
                .absolute()
                .left(rpx(width * 0.375))
                .top(rpx(height * 0.75))
                .w(rpx(width * 0.25))
                .h(rpx(height / 6.))
                .rounded_full()
                .bg(rgb(surface)),
        )
}

fn status_cutout(status: Status, dot: f32, ring: u32) -> Option<Div> {
    let cutout = div().flex_none().bg(rgb(ring));
    match status {
        Status::Online => None,
        Status::DoNotDisturb => Some(
            cutout
                .w(rpx(dot * 0.75))
                .h(rpx((dot * 0.25).max(2.)))
                .rounded(rpx(dot)),
        ),
        Status::Idle => Some(
            cutout
                .absolute()
                .left(rpx(-dot * 0.125))
                .top(rpx(-dot * 0.125))
                .size(rpx(dot * 0.75))
                .rounded_full(),
        ),
        Status::Offline => Some(cutout.size(rpx(dot * 0.5)).rounded_full()),
    }
}

const TOOLTIP_ENTER: Duration = Duration::from_millis(120);

pub fn tooltip_bubble() -> Div {
    div()
        .max_w(rpx(196.))
        .px(rpx(11.))
        .py(rpx(7.))
        .rounded(rpx(8.))
        .bg(rgb(tooltip_bg()))
        .border_1()
        .border_color(rgb(tooltip_ring()))
        .shadow(vec![BoxShadow {
            color: rgba(tooltip_shadow()).into(),
            offset: point(px(0.), px(12.)),
            blur_radius: px(24.),
            spread_radius: px(0.),
            inset: false,
        }])
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(strong()))
}

fn tooltip_arrow(fill: &str, edge: &str, width: f32, height: f32) -> Div {
    div()
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .child(icon(fill, width, tooltip_bg()).h(rpx(height)))
        .child(
            icon(edge, width, tooltip_ring())
                .h(rpx(height))
                .absolute()
                .top_0()
                .left_0(),
        )
}

pub fn tooltip_arrow_left() -> Div {
    tooltip_arrow("tooltip-arrow", "tooltip-arrow-edge", 9., 16.).mr(rpx(-1.))
}

fn tooltip_arrow_down() -> Div {
    tooltip_arrow("tooltip-arrow-down", "tooltip-arrow-down-edge", 16., 9.).mt(rpx(-1.))
}

fn tooltip_arrow_up() -> Div {
    tooltip_arrow("tooltip-arrow-up", "tooltip-arrow-up-edge", 16., 9.).mb(rpx(-1.))
}

/// Tooltip shown centred below an element, arrow pointing up, for elements with no room above.
/// Add it as a child of a `relative()` element while that element is hovered.
pub fn tooltip_below(label: impl Into<SharedString>) -> impl IntoElement {
    TooltipBelow {
        label: label.into(),
    }
}

/// Shows a `tooltip_above` while the pointer is over its `relative()` parent, holding its own
/// hover so that the parent's view needs no state for it.
pub fn hover_tooltip(
    key: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> impl IntoElement {
    HoverTooltip {
        key: key.into(),
        label: label.into(),
        detail: None,
        body: None,
        below: false,
    }
}

pub fn hover_tooltip_below(
    key: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> impl IntoElement {
    HoverTooltip {
        key: key.into(),
        label: label.into(),
        detail: None,
        body: None,
        below: true,
    }
}

pub fn hover_tooltip_body(key: impl Into<ElementId>, body: impl IntoElement) -> impl IntoElement {
    HoverTooltip {
        key: key.into(),
        label: SharedString::default(),
        detail: None,
        body: Some(body.into_any_element()),
        below: false,
    }
}

/// A hover tooltip with a second, regular-weight line under a bold label.
pub fn hover_tooltip_detail(
    key: impl Into<ElementId>,
    label: impl Into<SharedString>,
    detail: impl Into<SharedString>,
) -> impl IntoElement {
    HoverTooltip {
        key: key.into(),
        label: label.into(),
        detail: Some(detail.into()),
        body: None,
        below: false,
    }
}

#[derive(IntoElement)]
struct HoverTooltip {
    key: ElementId,
    label: SharedString,
    detail: Option<SharedString>,
    body: Option<AnyElement>,
    below: bool,
}

impl RenderOnce for HoverTooltip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let hovered = window.use_keyed_state(self.key.clone(), cx, |_, _| false);
        let shown = *hovered.read(cx);
        div()
            .id(self.key)
            .absolute()
            .inset_0()
            .on_hover(move |over, _, cx| {
                hovered.update(cx, |hovered, cx| {
                    *hovered = *over;
                    cx.notify();
                });
            })
            .when(shown, |this| {
                if self.below {
                    this.child(TooltipBelow { label: self.label })
                } else {
                    this.child(TooltipAbove {
                        label: self.label,
                        detail: self.detail,
                        body: self.body,
                    })
                }
            })
    }
}

/// Tooltip shown centred above an element, arrow pointing down. Add it as a
/// child of a `relative()` element while that element is hovered.
pub fn tooltip_above(label: impl Into<SharedString>) -> impl IntoElement {
    TooltipAbove {
        label: label.into(),
        detail: None,
        body: None,
    }
}

/// `relative` keeps the rise out of the anchor's layout.
pub fn tooltip_entrance(body: Div, cx: &App) -> AnyElement {
    if cx.reduce_motion() {
        return body.into_any_element();
    }
    let animation = Animation::new(TOOLTIP_ENTER).with_easing(ease_out_quint());
    body.relative()
        .with_animation("tooltip-enter", animation, |this, delta| {
            this.opacity(delta).top(rpx(4. * (1. - delta)))
        })
        .into_any_element()
}

#[derive(IntoElement)]
struct TooltipAbove {
    label: SharedString,
    detail: Option<SharedString>,
    body: Option<AnyElement>,
}

impl RenderOnce for TooltipAbove {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let bubble = div().child(
            tooltip_bubble()
                .flex()
                .flex_col()
                .items_center()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .map(|this| match (self.body, self.detail) {
                    (Some(body), _) => this.items_start().child(body),
                    (None, Some(detail)) => this
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(strong()))
                                .child(self.label),
                        )
                        .child(div().child(detail)),
                    (None, None) => this.child(self.label),
                }),
        );
        // The arrow stays on the target while only the bubble slides to fit the window. A single
        // anchored body would flip its anchor at the window edge and point at a neighbour instead.
        div()
            .absolute()
            .top(rpx(-4.))
            .left(relative(0.5))
            .size_0()
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::BottomCenter)
                        .child(tooltip_entrance(tooltip_arrow_down(), cx)),
                )
                .with_priority(2),
            )
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::BottomCenter)
                        // The arrow is 9px tall and overlaps the bubble's border by 1px.
                        .offset(point(px(0.), -rpx(8.).to_pixels(window.rem_size())))
                        .snap_to_window_with_margin(px(8.))
                        .child(tooltip_entrance(bubble, cx)),
                )
                .with_priority(2),
            )
    }
}

/// `body` centred above its `relative()` parent, over everything else.
fn float_above(body: Div, cx: &App) -> Div {
    div()
        .absolute()
        .top(rpx(-4.))
        .left(relative(0.5))
        .size_0()
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::BottomCenter)
                    .child(tooltip_entrance(body, cx)),
            )
            // Tooltips also appear inside deferred popouts, so they must outrank them.
            .with_priority(2),
        )
}

/// A tooltip bubble with no arrow holding `content`, centred above its `relative()` parent.
pub fn tooltip_card_above(content: impl IntoElement, cx: &App) -> Div {
    float_above(div().child(content), cx)
}

#[derive(IntoElement)]
struct TooltipBelow {
    label: SharedString,
}

impl RenderOnce for TooltipBelow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let body = div()
            .flex()
            .flex_col()
            .items_center()
            .child(tooltip_arrow_up())
            .child(
                tooltip_bubble()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .child(self.label),
            );
        div()
            .absolute()
            .bottom(rpx(-4.))
            .left(relative(0.5))
            .size_0()
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::TopCenter)
                        .child(tooltip_entrance(body, cx)),
                )
                .with_priority(2),
            )
    }
}

pub fn badge(count: u32, ring: u32) -> impl IntoElement {
    let label = if count >= 1000 {
        format!("{}K+", count / 1000)
    } else {
        count.to_string()
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .min_w(rpx(20.))
        .h(rpx(20.))
        .px(rpx(5.))
        .rounded_full()
        .bg(rgb(NOTIFICATION))
        .border_2()
        .border_color(rgb(ring))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white())
        .child(label)
}

pub fn app_tag(verified: bool) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(16.))
        .pl(rpx(if verified { 2. } else { 4. }))
        .pr(rpx(4.))
        .rounded(rpx(4.))
        .bg(rgb(BLURPLE))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white())
        .when(verified, |this| {
            this.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(rpx(16.))
                    .child(icon("check-bold", 14., 0xFFFFFF)),
            )
        })
        .child("APP")
}

pub fn guild_tag(tag: &GuildTag, cx: &App) -> Div {
    let badge = tag
        .badge_spec()
        .and_then(|spec| images::resolve(&spec, image_px(12.), cx));
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(3.))
        .px(rpx(4.))
        .rounded(rpx(4.))
        .bg(rgba(selected()))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(strong()))
        .children(badge.map(|path| img(path).size(rpx(12.)).flex_none()))
        .child(tag.text.clone())
}

/// For clickable elements that stop their click from propagating: the click's mouse-up then
/// never reaches the window's text selection, which would keep extending a selection begun
/// by the same press. Suppressing selection on mouse-down avoids starting one at all.
pub trait WithoutTextSelection: InteractiveElement + Sized {
    fn without_text_selection(self) -> Self {
        self.on_mouse_down(MouseButton::Left, |_, _, cx| {
            gpui_base::GlobalState::suppress_text_selection(cx)
        })
    }
}

impl<E: InteractiveElement> WithoutTextSelection for E {}

pub fn icon_button(id: impl Into<ElementId>, name: &str, size: f32, color: u32) -> Stateful<Div> {
    icon_button_with_hover(id, name, size, color, rgba(hover()).into())
}

pub fn icon_button_with_hover(
    id: impl Into<ElementId>,
    name: &str,
    size: f32,
    color: u32,
    hover: Hsla,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(size + 8.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(icon(name, size, color))
}
