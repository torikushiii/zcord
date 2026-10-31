use gpui::{prelude::*, *};
use gpui_component::Theme;
use std::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use crate::{
    media::animated,
    media::images::{self, ImageSpec},
    settings,
    store::{Decoration, GuildTag, Status, Store, User, WebhookAuthor},
};

mod midnight;
mod profile_avatar;

pub use midnight::init;
pub use profile_avatar::status_icon;

pub const BG: u32 = 0x000000;
pub const SURFACE_HIGH: u32 = 0x0A0A0C;
pub const ELEVATED: u32 = 0x121214;
pub const SURFACE_HIGHEST: u32 = 0x17181B;
// Layer colors are RGBA, not RGB: they must blend with their actual surface.
pub const BORDER: u32 = 0x96969F3D;
pub const BORDER_SUBTLE: u32 = 0x9696A033;
pub const BORDER_MUTED: u32 = 0x9595A229;
pub const BORDER_STRONG: u32 = 0x96969F70;
pub const HOVER: u32 = 0x94949C1F;
pub const ACTIVE: u32 = 0x9696A033;
pub const SELECTED: u32 = 0x96969F3D;
pub const CONTROL: u32 = 0x99999914;
pub const CODE_BACKGROUND: u32 = 0x5966F214;
pub const SCRIM: u32 = 0x000000B8;
pub const LIGHTBOX_SCRIM: u32 = 0x000000EB;
pub const PLACEHOLDER_STRONG: u32 = 0x2C2D32;
pub const TEXT: u32 = 0xD4D5D8;
pub const TEXT_SELECTION: u32 = 0x0438A2;
pub const STRONG: u32 = 0xDCDCDF;
pub const MUTED: u32 = 0x96979E;
pub const FAINT: u32 = 0x81828A;
pub const INTERACTIVE_MUTED: u32 = 0x4F505A;
pub const LINK: u32 = 0x2781E7;
pub const FOCUS: u32 = 0x6AA8F4;
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
        image,
        user.and_then(|user| user.decoration.as_ref())
            .and_then(|data| decoration(user_id, data, size, animate, notch, cx)),
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
    avatar(&initials, user_color(author_id), size, None, image, None)
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
    let animated = animate
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
    let animates = animate && decoration.asset.starts_with("a_");
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
                .text_color(rgb(BG))
                .text_size(rpx(size * 0.36))
                .font_weight(FontWeight::SEMIBOLD)
                .child(initials.to_owned())
                .into_any_element(),
        })
        .children(decoration)
        .children(status.map(|(status, ring)| {
            if let Some(notch) = notch {
                return profile_avatar::indicator(notch, status, ring);
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

pub const TOOLTIP_BG: u32 = 0x0A0A0C;
const TOOLTIP_RING: u32 = 0x262629;
const TOOLTIP_ENTER: Duration = Duration::from_millis(120);

pub fn tooltip_bubble() -> Div {
    div()
        .max_w(rpx(196.))
        .px(rpx(11.))
        .py(rpx(7.))
        .rounded(rpx(8.))
        .bg(rgb(TOOLTIP_BG))
        .border_1()
        .border_color(rgb(TOOLTIP_RING))
        .shadow(vec![BoxShadow {
            color: rgba(0x0000003D).into(),
            offset: point(px(0.), px(12.)),
            blur_radius: px(24.),
            spread_radius: px(0.),
            inset: false,
        }])
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(STRONG))
}

fn tooltip_arrow(fill: &str, edge: &str, width: f32, height: f32) -> Div {
    div()
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .child(icon(fill, width, TOOLTIP_BG).h(rpx(height)))
        .child(
            icon(edge, width, TOOLTIP_RING)
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

/// Tooltip shown centred above an element, arrow pointing down. Add it as a
/// child of a `relative()` element while that element is hovered.
pub fn tooltip_above(label: impl Into<SharedString>) -> impl IntoElement {
    TooltipAbove {
        label: label.into(),
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

struct TextTooltip(SharedString);

impl Render for TextTooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        tooltip_entrance(
            tooltip_bubble()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .child(self.0.clone()),
            cx,
        )
    }
}

/// For `.tooltip(..)` on elements without their own hover state.
pub fn text_tooltip(label: impl Into<SharedString>, cx: &mut App) -> AnyView {
    let label = label.into();
    cx.new(|_| TextTooltip(label)).into()
}

#[derive(IntoElement)]
struct TooltipAbove {
    label: SharedString,
}

impl RenderOnce for TooltipAbove {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let body = div()
            .flex()
            .flex_col()
            .items_center()
            .child(
                tooltip_bubble()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .child(self.label),
            )
            .child(tooltip_arrow_down());
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

pub fn app_tag() -> impl IntoElement {
    div()
        .flex_none()
        .px(rpx(4.))
        .rounded(rpx(3.))
        .bg(rgb(BLURPLE))
        .text_size(rpx(10.))
        .line_height(rpx(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white())
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
        .bg(rgba(SELECTED))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(STRONG))
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
    icon_button_with_hover(id, name, size, color, rgba(HOVER).into())
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
