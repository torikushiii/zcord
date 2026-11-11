mod avatar;
mod elements;
mod tooltip;

use avatar::{DECORATION_SCALE, mobile_status};
pub use avatar::{
    activity_icon, default_avatar, member_avatar, placeholder_avatar, preview_avatar, spec_avatar,
    undecorated_member_avatar, user_avatar, webhook_avatar,
};
pub use elements::{
    WithoutTextSelection, app_tag, badge, guild_tag, icon_button, icon_button_with_hover,
    media_loading, selection_guard, spinner,
};
pub use tooltip::{
    hover_tooltip, hover_tooltip_below, hover_tooltip_body, hover_tooltip_detail, tooltip_above,
    tooltip_arrow_left, tooltip_below, tooltip_bubble, tooltip_card_above, tooltip_entrance,
};

use gpui::{prelude::*, *};
use gpui_component::Theme;
use std::{
    f32::consts::{PI, TAU},
    sync::{
        Arc, OnceLock, RwLock,
        atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{
    media::animated,
    media::images::{self, ImageSpec},
    settings,
    store::{Decoration, GuildTag, Status, Store, User, WebhookAuthor},
};

mod ash;
mod components;
mod dark;
pub mod gradient;
mod light;
mod onyx;
mod profile_avatar;

use ash::ASH;
pub use components::init;
use dark::DARK;
pub use gradient::{Gradient, Layer};
use light::LIGHT;
use onyx::ONYX;
pub use profile_avatar::status_icon;

macro_rules! palette {
    ($($name:ident),* $(,)?) => {
        pub struct Palette {
            $(pub $name: u32,)*
        }

        impl Palette {
            fn map(&self, mut change: impl FnMut(&str, u32) -> u32) -> Palette {
                Palette {
                    $($name: change(stringify!($name), self.$name),)*
                }
            }
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
    base_low,
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
    slider_track,
    text_positive,
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BaseTheme {
    Light,
    Ash,
    Dark,
    #[default]
    Onyx,
}

impl BaseTheme {
    fn palette(self) -> &'static Palette {
        match self {
            Self::Light => &LIGHT,
            Self::Ash => &ASH,
            Self::Dark => &DARK,
            Self::Onyx => &ONYX,
        }
    }
}

static LIGHT_ACTIVE: AtomicBool = AtomicBool::new(false);
// Only ever points at a static palette or a leaked tinted one, so it is never dangling.
static PALETTE: AtomicPtr<Palette> = AtomicPtr::new(&ONYX as *const Palette as *mut Palette);
static GRADIENT: RwLock<Option<Arc<Gradient>>> = RwLock::new(None);

struct Active(BaseTheme, Option<Arc<Gradient>>);

impl Global for Active {}

fn palette() -> &'static Palette {
    // SAFETY: PALETTE only holds pointers from `&'static Palette`s.
    unsafe { &*PALETTE.load(Ordering::Relaxed) }
}

pub fn dark_palette() -> &'static Palette {
    &ONYX
}

pub fn is_light() -> bool {
    LIGHT_ACTIVE.load(Ordering::Relaxed)
}

pub fn gradient() -> Option<Arc<Gradient>> {
    GRADIENT.read().ok()?.clone()
}

pub fn set_theme(base: BaseTheme, gradient: Option<Gradient>, cx: &mut App) {
    let gradient = gradient.map(Arc::new);
    if cx
        .try_global::<Active>()
        .is_some_and(|active| active.0 == base && active.1 == gradient)
    {
        return;
    }
    let palette: &'static Palette = match &gradient {
        Some(gradient) => Box::leak(Box::new(gradient.tint(base.palette()))),
        None => base.palette(),
    };
    PALETTE.store(palette as *const Palette as *mut Palette, Ordering::Relaxed);
    LIGHT_ACTIVE.store(base == BaseTheme::Light, Ordering::Relaxed);
    if let Ok(mut active) = GRADIENT.write() {
        active.clone_from(&gradient);
    }
    cx.set_global(Active(base, gradient));
    components::init(cx);
    Theme::global_mut(cx).font_size = px(BASE_FONT_SIZE * ZOOM_LEVELS[zoom_index(cx)]);
    cx.refresh_windows();
}

pub fn surface(layer: Layer, under: Option<Layer>, solid: u32) -> Hsla {
    match gradient() {
        Some(gradient) => gradient.layer_over(layer, under),
        None => rgb(solid).into(),
    }
}

pub fn gradient_backdrop() -> Option<impl IntoElement> {
    gradient().map(|gradient| {
        div().absolute().inset_0().child(gradient.backdrop()).child(
            div()
                .absolute()
                .inset_0()
                .bg(gradient.layer_over(Layer::Higher, None)),
        )
    })
}

pub const SCRIM: u32 = 0x000000B8;
pub const LIGHTBOX_SCRIM: u32 = 0x000000EB;
pub const BLURPLE: u32 = 0x5865F2;
pub const BLURPLE_ACTIVE: u32 = 0x3A48A3;
pub const RED: u32 = 0xEB5F5E;
pub const GREEN: u32 = 0x41A063;
pub const YELLOW: u32 = 0xCE8100;
pub const NOTIFICATION: u32 = 0xD22D39;
pub const STREAMER: u32 = 0x9147FF;
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

static IMAGE_SCALE: AtomicU32 = AtomicU32::new(0x4000_0000);

pub fn set_image_scale(window: &Window, cx: &App) {
    let scale = window.scale_factor() * ZOOM_LEVELS[zoom_index(cx)];
    IMAGE_SCALE.store(scale.to_bits(), Ordering::Relaxed);
}

pub fn image_px(css: f32) -> u32 {
    (css * f32::from_bits(IMAGE_SCALE.load(Ordering::Relaxed))).ceil() as u32
}

pub fn role_dot(color: u32) -> Div {
    div()
        .flex_none()
        .size(rpx(8.))
        .rounded_full()
        .bg(rgb(color))
}
