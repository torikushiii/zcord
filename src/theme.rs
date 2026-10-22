use gpui::{prelude::*, *};

use std::{path::Path, sync::Arc};

use crate::{
    data::{Status, User},
    images,
};

pub const BG: u32 = 0x08080A;
pub const ELEVATED: u32 = 0x121214;
pub const BORDER: u32 = 0x202024;
pub const HOVER: u32 = 0x151518;
pub const SELECTED: u32 = 0x24242A;
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
    cx: &App,
) -> impl IntoElement + use<> {
    let image = user.and_then(|user| images::resolve(&user.avatar_spec(), image_px(size), cx));
    avatar(
        user.map_or("?", |user| &user.initials),
        user_color(user_id),
        size,
        status,
        image,
    )
}

pub fn avatar(
    initials: &str,
    color: Hsla,
    size: f32,
    status: Option<(Status, u32)>,
    image: Option<Arc<Path>>,
) -> impl IntoElement + use<> {
    let dot = size * 0.3;
    div()
        .relative()
        .flex_none()
        .size(rpx(size))
        .child(match image {
            Some(path) => img(path).size_full().rounded_full().into_any_element(),
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(color)
                .text_color(rgb(BG))
                .text_size(rpx(size * 0.36))
                .font_weight(FontWeight::SEMIBOLD)
                .child(initials.to_owned())
                .into_any_element(),
        })
        .children(status.map(|(status, ring)| {
            div()
                .absolute()
                .right(rpx(-2.))
                .bottom(rpx(-2.))
                .size(rpx(dot + 4.))
                .rounded_full()
                .border_2()
                .border_color(rgb(ring))
                .bg(rgb(status_color(status)))
        }))
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

pub fn guild_tag(tag: &str) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(3.))
        .px(rpx(5.))
        .rounded(rpx(4.))
        .bg(rgb(SELECTED))
        .text_size(rpx(10.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(STRONG))
        .child(div().text_color(rgb(0xC77DFF)).child("♥"))
        .child(tag.to_owned())
}

pub fn icon_button(id: impl Into<ElementId>, name: &str, size: f32, color: u32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(size + 8.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(SELECTED)))
        .child(icon(name, size, color))
}
