use std::cell::RefCell;

use super::controls::art as asset;
use super::*;
use crate::{
    app::{members::member_list, profile},
    discord::model::CollectibleItem,
    media::{animated, images},
    store::{Decoration, Nameplate},
};

pub(super) const MOCK_RADIUS: f32 = 8.;
const DECORATION_SCALE: f32 = 1.2;
const PLATE_HEIGHT: f32 = 42.;
const SKELETON: u32 = 0x2E2F34;
const SKELETON_STRONG: u32 = 0x3F4046;

pub(super) fn decoration_of(item: &CollectibleItem) -> Option<Decoration> {
    Some(Decoration {
        asset: item.asset.clone()?,
        sku_id: Some(item.sku_id),
    })
}

pub(super) fn nameplate_of(item: &CollectibleItem) -> Option<Nameplate> {
    Some(Nameplate {
        asset: item.asset.clone()?,
        palette: item.palette.clone(),
        label: item.label.clone(),
    })
}

fn kind_of(item: &CollectibleItem) -> ShopKind {
    ShopProduct::item_kind(item)
}

pub(super) fn default_avatar(size: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(size))
        .rounded_full()
        .bg(rgb(0x5C5E66))
        .child(theme::icon("discord", size * 0.6, 0x3B3D44))
}

pub(super) fn decoration_layer(
    id: impl Into<ElementId>,
    decoration: &Decoration,
    size: f32,
    animate: bool,
    cx: &App,
) -> Option<AnyElement> {
    let frame = size * DECORATION_SCALE;
    let pixels = theme::image_px(frame);
    let animated = (animate && decoration.asset.starts_with("a_"))
        .then(|| animated::resolve(&decoration.image_spec(true), pixels, cx))
        .flatten();
    let image = match animated {
        Some(image) => animated::image(image).id(id).size_full().into_any_element(),
        None => img(images::resolve(&decoration.image_spec(false), pixels, cx)?)
            .size_full()
            .into_any_element(),
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

pub(super) fn decoration_card(
    app: &DiscordApp,
    id: &SharedString,
    item: &CollectibleItem,
    size: f32,
    hovered: bool,
    cx: &App,
) -> AnyElement {
    let decoration = decoration_of(item).filter(|_| hovered);
    match decoration {
        Some(decoration) => {
            let user_id = app.store.current_user_id;
            div()
                .id(SharedString::from(format!("{id}-own")))
                .flex_none()
                .child(theme::preview_avatar(
                    app.store.user(user_id),
                    user_id,
                    size,
                    None,
                    None,
                    None,
                    Some(&decoration),
                    cx,
                ))
                .into_any_element()
        }
        None => decorated_placeholder(id.clone(), item, size, false, cx),
    }
}

pub(super) fn decorated_placeholder(
    id: impl Into<ElementId>,
    item: &CollectibleItem,
    size: f32,
    animate: bool,
    cx: &App,
) -> AnyElement {
    div()
        .relative()
        .flex_none()
        .size(rpx(size))
        .child(default_avatar(size))
        .children(
            decoration_of(item)
                .and_then(|decoration| decoration_layer(id, &decoration, size, animate, cx)),
        )
        .into_any_element()
}

fn bar(width: f32, height: f32, color: u32) -> Div {
    div()
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(height / 2.))
        .bg(rgb(color))
}

pub(super) fn profile_mock(width: f32, height: f32) -> Div {
    let avatar = width * 0.3;
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(rgb(theme::base_lowest()))
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .child(div().w_full().h(rpx(width * 0.36)).bg(rgb(SKELETON)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(width * 0.04))
                .px(rpx(width * 0.08))
                .child(
                    div()
                        .size(rpx(avatar))
                        .mt(rpx(-avatar * 0.55))
                        .rounded_full()
                        .border(rpx(width * 0.025))
                        .border_color(rgb(theme::base_lowest()))
                        .bg(rgb(SKELETON_STRONG)),
                )
                .child(bar(width * 0.5, width * 0.05, SKELETON_STRONG))
                .child(bar(width * 0.35, width * 0.04, SKELETON))
                .child(bar(width * 0.75, width * 0.04, SKELETON))
                .child(bar(width * 0.6, width * 0.04, SKELETON)),
        )
}

pub(super) fn effect(
    item: &CollectibleItem,
    clock: &RefCell<Option<Instant>>,
    width: f32,
    corner: f32,
    animate: bool,
    cx: &App,
) -> Vec<AnyElement> {
    if animate && !cx.reduce_motion() {
        let layers = profile::effect_item_layers(item, clock, width, corner, false, cx);
        if !layers.is_empty() {
            return layers;
        }
    }
    item.thumbnail_preview_src
        .as_deref()
        .or(item.reduced_motion_src.as_deref())
        .and_then(|url| asset(url, width, cx))
        .map(|image| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .child(image.w_full().rounded_t(rpx(corner)))
                .into_any_element()
        })
        .into_iter()
        .collect()
}

pub(super) fn effect_card(
    item: &CollectibleItem,
    clock: &RefCell<Option<Instant>>,
    width: f32,
    height: f32,
    animate: bool,
    cx: &App,
) -> AnyElement {
    div()
        .relative()
        .flex()
        .justify_center()
        .w(rpx(width))
        .h(rpx(height))
        .overflow_hidden()
        .child(profile_mock(width * 0.62, height * 1.2).mt(rpx(height * 0.12)))
        .children(effect(item, clock, width, card::INNER_RADIUS, animate, cx))
        .into_any_element()
}

pub(super) fn framed_mock(item: &CollectibleItem, width: f32, height: f32, cx: &App) -> AnyElement {
    let overflow = profile::item_overflow(item, width);
    div()
        .flex_none()
        .pt(rpx(overflow.top))
        .pb(rpx(overflow.bottom))
        .child(profile::framed_with(
            Some((item.sku_id, item)),
            width,
            profile_mock(width, height).into_any_element(),
            false,
            cx,
        ))
        .into_any_element()
}

pub(super) fn frame_card(item: &CollectibleItem, width: f32, height: f32, cx: &App) -> AnyElement {
    let card_width = (width * 0.5).min(height * 0.62);
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(rpx(width))
        .h(rpx(height))
        .overflow_hidden()
        .child(framed_mock(item, card_width, card_width * 1.25, cx))
        .into_any_element()
}

pub(super) fn plate_row(
    app: &DiscordApp,
    plate: Option<&Nameplate>,
    width: f32,
    animate: bool,
    own: bool,
    cx: &App,
) -> AnyElement {
    let user_id = app.store.current_user_id;
    let user = app.store.user(user_id);
    let content = if own {
        div()
            .flex()
            .items_center()
            .gap(rpx(10.))
            .min_w_0()
            .child(theme::preview_avatar(
                user, user_id, 32., None, None, None, None, cx,
            ))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(rpx(15.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .child(user.map_or_else(String::new, |user| user.name.clone())),
            )
    } else if plate.is_some() {
        div()
            .flex()
            .items_center()
            .gap(rpx(12.))
            .child(default_avatar(32.))
            .child(bar(123., 14., 0x706F74).opacity(0.5))
    } else {
        div()
            .flex()
            .items_center()
            .gap(rpx(12.))
            .child(div().size(rpx(32.)).rounded_full().bg(rgb(SKELETON)))
            .child(bar(width * 0.55, 14., SKELETON))
    };
    div()
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(PLATE_HEIGHT))
        .rounded(rpx(8.))
        .overflow_hidden()
        .children(plate.and_then(|plate| {
            member_list::rounded_nameplate_layer(plate, user_id, PLATE_HEIGHT, animate, 8., cx)
        }))
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .px(rpx(8.))
                .child(content),
        )
        .into_any_element()
}

pub(super) fn nameplate_card(
    app: &DiscordApp,
    item: &CollectibleItem,
    width: f32,
    height: f32,
    hovered: bool,
    cx: &App,
) -> AnyElement {
    let plate = nameplate_of(item);
    let row_width = width - 16.;
    let filler = |opacity: f32| {
        div()
            .opacity(opacity)
            .child(plate_row(app, None, row_width, false, false, cx))
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rpx(4.))
        .w(rpx(width))
        .h(rpx(height))
        .overflow_hidden()
        .child(filler(0.7))
        .child(filler(0.85))
        .child(plate_row(
            app,
            plate.as_ref(),
            row_width,
            hovered,
            hovered,
            cx,
        ))
        .child(filler(0.85))
        .child(filler(0.7))
        .into_any_element()
}

pub(super) fn other_card(product: &ShopProduct, size: f32) -> AnyElement {
    let icon = if product.name.to_ascii_lowercase().contains("nitro") {
        "nitro"
    } else {
        "orb"
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(size))
        .rounded_full()
        .bg(linear_gradient(
            135.,
            linear_color_stop(rgb(0x7C4DFF), 0.),
            linear_color_stop(rgb(theme::BLURPLE), 1.),
        ))
        .child(theme::icon(icon, size * 0.5, 0xFFFFFF))
        .into_any_element()
}

pub(super) fn poster_layer(url: &str, width: f32, cx: &App) -> Option<Div> {
    asset(url, width * 2.2, cx).map(|image| {
        div()
            .absolute()
            .inset_0()
            .child(image.size_full().object_fit(ObjectFit::Cover))
    })
}

pub(super) fn bundle_card(
    app: &DiscordApp,
    id: SharedString,
    product: &ShopProduct,
    clock: &RefCell<Option<Instant>>,
    [width, height]: [f32; 2],
    animate: bool,
    cx: &App,
) -> AnyElement {
    let item = |kind: ShopKind| product.items.iter().find(|item| kind_of(item) == kind);
    let side = item(ShopKind::Effect)
        .map(|effect_item| {
            let card_width = width * 0.36;
            div()
                .relative()
                .flex_none()
                .w(rpx(card_width))
                .h(rpx(height * 0.9))
                .rounded(rpx(MOCK_RADIUS))
                .overflow_hidden()
                .child(profile_mock(card_width, height * 0.9))
                .children(effect(
                    effect_item,
                    clock,
                    card_width,
                    MOCK_RADIUS,
                    animate,
                    cx,
                ))
                .into_any_element()
        })
        .or_else(|| {
            item(ShopKind::Frame).map(|frame| framed_mock(frame, width * 0.3, height * 0.62, cx))
        });
    let avatar = (height * 0.42).min(width * 0.3);
    let left = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .items_center()
        .justify_center()
        .gap(rpx(height * 0.08))
        .children(item(ShopKind::Decoration).map(|decoration| {
            decorated_placeholder(
                SharedString::from(format!("{id}-decoration")),
                decoration,
                avatar,
                animate,
                cx,
            )
        }))
        .children(item(ShopKind::Nameplate).map(|plate| {
            plate_row(
                app,
                nameplate_of(plate).as_ref(),
                (width * if side.is_some() { 0.5 } else { 0.8 }).min(220.),
                animate,
                true,
                cx,
            )
        }));
    div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .w(rpx(width))
        .h(rpx(height))
        .px(rpx(12.))
        .overflow_hidden()
        .child(left)
        .children(side)
        .into_any_element()
}
