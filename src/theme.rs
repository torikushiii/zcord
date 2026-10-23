use gpui::{prelude::*, *};

use crate::{
    animated,
    images::{self, ImageSpec},
    store::{Decoration, GuildTag, NameStyle, Status, Store, User},
};

mod profile_avatar;

pub const BG: u32 = 0x08080A;
pub const ELEVATED: u32 = 0x121214;
pub const BORDER: u32 = 0x202024;
pub const HOVER: u32 = 0x151518;
pub const SELECTED: u32 = 0x24242A;
pub const PLACEHOLDER_STRONG: u32 = 0x2E2E36;
pub const TEXT: u32 = 0xDBDEE1;
pub const STRONG: u32 = 0xF2F3F5;
pub const MUTED: u32 = 0x949BA4;
pub const FAINT: u32 = 0x6D6F78;
pub const BLURPLE: u32 = 0x5865F2;
pub const RED: u32 = 0xF23F43;
pub const GREEN: u32 = 0x23A55A;
pub const YELLOW: u32 = 0xF0B232;

pub const BASE_FONT_SIZE: f32 = 16.;
pub const ZOOM_LEVELS: [f32; 11] = [0.5, 0.67, 0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2.];

pub fn rpx(css_pixels: f32) -> Rems {
    rems(css_pixels / BASE_FONT_SIZE)
}

pub fn user_color(id: u64) -> Hsla {
    hsla(((id % 360) * 47 % 360) as f32 / 360., 0.55, 0.67, 1.)
}

pub fn status_color(status: Status) -> u32 {
    match status {
        Status::Online => GREEN,
        Status::Idle => YELLOW,
        Status::DoNotDisturb => RED,
        Status::Offline => FAINT,
    }
}

pub fn icon(name: &str, size: f32, color: u32) -> Svg {
    svg()
        .path(SharedString::from(format!("app/{name}.svg")))
        .size(rpx(size))
        .flex_none()
        .text_color(rgb(color))
}

pub fn image_px(css: f32) -> u32 {
    (css * 2.).ceil() as u32
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

pub fn display_name(text: &str, style: Option<&NameStyle>, fallback: u32) -> AnyElement {
    let colors: Vec<Rgba> = style
        .map(|style| style.colors.iter().map(|color| rgb(*color)).collect())
        .unwrap_or_default();
    if colors.len() < 2 {
        let color = colors.first().copied().unwrap_or(rgb(fallback));
        return div()
            .text_color(color)
            .child(SharedString::from(text.to_owned()))
            .into_any_element();
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let last = chars.len().saturating_sub(1).max(1) as f32;
    let highlights = chars.iter().enumerate().map(|(index, (offset, ch))| {
        let color = gradient_at(&colors, index as f32 / last);
        (
            *offset..offset + ch.len_utf8(),
            HighlightStyle {
                color: Some(color.into()),
                ..Default::default()
            },
        )
    });
    div()
        .text_color(colors[0])
        .child(StyledText::new(text.to_owned()).with_highlights(highlights))
        .into_any_element()
}

fn gradient_at(colors: &[Rgba], position: f32) -> Rgba {
    let scaled = position.clamp(0., 1.) * (colors.len() - 1) as f32;
    let index = (scaled.floor() as usize).min(colors.len() - 2);
    let t = scaled - index as f32;
    let (a, b) = (colors[index], colors[index + 1]);
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
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
    let dot = size * 0.3;
    div()
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

pub const TOOLTIP_BG: u32 = 0x1F1F24;

/// Body of a Discord-style tooltip. Pair it with a `tooltip-arrow*` icon and
/// position it with `deferred(anchored())`; callers set the text size.
pub fn tooltip_bubble() -> Div {
    div()
        .max_w(rpx(196.))
        .px(rpx(12.))
        .py(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(TOOLTIP_BG))
        .shadow_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(STRONG))
}

/// Tooltip shown centred above an element, arrow pointing down. Add it as a
/// child of a `relative()` element while that element is hovered.
pub fn tooltip_above(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .absolute()
        .top(rpx(-4.))
        .left(relative(0.5))
        .size_0()
        .child(deferred(
            anchored().anchor(Anchor::BottomCenter).child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        tooltip_bubble()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .child(label.into()),
                    )
                    .child(icon("tooltip-arrow-down", 12., TOOLTIP_BG).h(rpx(6.))),
            ),
        ))
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
        .bg(rgb(RED))
        .border_2()
        .border_color(rgb(ring))
        .text_size(rpx(11.))
        .font_weight(FontWeight::BOLD)
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
        .bg(rgb(SELECTED))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(STRONG))
        .children(badge.map(|path| img(path).size(rpx(12.)).flex_none()))
        .child(tag.text.clone())
}

pub fn icon_button(id: impl Into<ElementId>, name: &str, size: f32, color: u32) -> Stateful<Div> {
    icon_button_with_hover(id, name, size, color, rgb(SELECTED).into())
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
