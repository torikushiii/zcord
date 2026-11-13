use chrono::{DateTime, Utc};

use super::controls::{BORDER_SUBTLE, Tone, button, on_app, pulse};
use super::*;
use crate::media::images::ImageSpec;

const FOOTER: f32 = 68.;
const RADIUS: f32 = 8.;
pub(super) const INNER_RADIUS: f32 = RADIUS - 2.;
const LIFT: f32 = 6.;
const ZOOM: f32 = 0.08;
const BUTTONS: f32 = 46.;
const HOVER: Duration = Duration::from_millis(180);
const LAST_DAY: i64 = 24 * 60 * 60;

pub(super) fn ends_soon(product: &ShopProduct) -> bool {
    product
        .unpublished_at
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .is_some_and(|at| {
            let left = at.with_timezone(&Utc) - Utc::now();
            left.num_seconds() > 0 && left.num_seconds() <= LAST_DAY
        })
}

pub(super) fn orbs_only(product: &ShopProduct) -> bool {
    product.prices.orbs().is_some() && product.prices.regular_local().is_none()
}

pub(super) fn display_name(product: &ShopProduct) -> String {
    if product.kind == ShopKind::Bundle && product.items.len() > 1 {
        format!("{}pc {}", product.items.len(), product.name)
    } else {
        product.name.clone()
    }
}

fn badge(product: &ShopProduct) -> Option<Div> {
    let chip = |text: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(rpx(4.))
            .h(rpx(20.))
            .px(rpx(6.))
            .rounded(rpx(4.))
            .text_size(rpx(12.))
            .font_weight(FontWeight::BOLD)
            .child(text)
    };
    if ends_soon(product) {
        return Some(chip("LAST DAY TO BUY").bg(white()).text_color(black()));
    }
    orbs_only(product).then(|| {
        chip("ORBS EXCLUSIVE")
            .bg(rgba(0x000000B3))
            .border_1()
            .border_color(rgba(0xFFFFFF66))
            .text_color(white())
            .child(theme::icon("orb", 12., 0xFFFFFF))
            .flex_row_reverse()
    })
}

pub(super) fn price_line(product: &ShopProduct, nitro: bool, owned: bool, size: f32) -> Div {
    let line = div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .min_w_0()
        .text_size(rpx(size))
        .line_height(rpx(size + 4.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()));
    if owned {
        return line
            .child(theme::icon("check", size, theme::GREEN))
            .child("Owned");
    }
    let Some(shown) = price::card(product, nitro) else {
        return line.child(div().text_color(rgb(theme::muted())).child("Unavailable"));
    };
    line.when(shown.orbs, |this| {
        this.child(theme::icon("orb", size, theme::strong()))
    })
    .when(shown.nitro, |this| {
        this.child(theme::icon("nitro", size, theme::strong()))
    })
    .child(div().min_w_0().truncate().child(shown.text))
    .children(shown.discount.map(|percent| {
        div()
            .flex_none()
            .text_color(rgb(theme::GREEN))
            .child(format!("(-{percent}%)"))
    }))
}

fn swatches(product: &ShopProduct) -> Option<Div> {
    if product.variants.len() < 2 {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(2.))
            .children(product.variants.iter().take(4).map(|variant| {
                let current = variant.sku_id == product.sku_id;
                div()
                    .size(rpx(16.))
                    .rounded_full()
                    .bg(rgb(variant.swatch.unwrap_or(0x5C5E66)))
                    .when(current, |this| this.border_2().border_color(white()))
            })),
    )
}

pub(super) fn purchase_label(product: &ShopProduct, nitro: bool) -> String {
    if let Some(price) = product.prices.local(nitro) {
        return format!("Buy for {}", price::money(price, true));
    }
    match product.prices.orbs() {
        Some(orbs) => format!("Redeem for {} Orbs", price::grouped(orbs)),
        None => "Buy".to_owned(),
    }
}

fn actions(app: &DiscordApp, key: &SharedString, product: &ShopProduct) -> Div {
    let sku = product.sku_id;
    div()
        .flex()
        .gap(rpx(8.))
        .w_full()
        .child(
            button(
                SharedString::from(format!("{key}-buy")),
                purchase_label(product, nitro(app)),
                Tone::Brand,
            )
            .flex_1()
            .min_w_0()
            .px(rpx(8.))
            .text_size(rpx(14.))
            .on_click(on_app(app, move |app, _, cx| app.show_shop_notice(sku, cx))),
        )
        .when(product.kind != ShopKind::Other, |this| {
            this.child(
                button(SharedString::from(format!("{key}-gift")), "", Tone::Brand)
                    .w(rpx(40.))
                    .px_0()
                    .child(theme::icon("gift", 20., 0xFFFFFF))
                    .on_click(on_app(app, move |app, _, cx| app.show_shop_notice(sku, cx))),
            )
        })
}

#[derive(Clone, Copy)]
enum Motion {
    Rest,
    In(u64),
    Out(u64),
}

fn motion(shop: &Shop, key: (u32, u64)) -> Motion {
    match shop.left {
        _ if shop.hovered == Some(key) => Motion::In(shop.entered),
        Some((left, changes)) if left == key => Motion::Out(changes),
        _ => Motion::Rest,
    }
}

fn animate(
    element: Div,
    id: &SharedString,
    part: &'static str,
    motion: Motion,
    apply: impl Fn(Div, f32) -> Div + 'static,
) -> AnyElement {
    let (changes, entering) = match motion {
        Motion::Rest => return apply(element, 0.).into_any_element(),
        Motion::In(changes) => (changes, true),
        Motion::Out(changes) => (changes, false),
    };
    element
        .with_animation(
            SharedString::from(format!("{id}-{part}-{changes}")),
            Animation::new(HOVER).with_easing(ease_in_out),
            move |element, delta| apply(element, if entering { delta } else { 1. - delta }),
        )
        .into_any_element()
}

pub(super) fn product_art(
    app: &DiscordApp,
    id: &SharedString,
    product: &ShopProduct,
    [width, height]: [f32; 2],
    hovered: bool,
    cx: &App,
) -> AnyElement {
    let clock = &app.home.shop.card_clock;
    let item = product.items.first();
    match (product.kind, item) {
        (ShopKind::Decoration, Some(item)) => {
            let size = (height * 0.66).min(width * 0.5);
            art::decoration_card(app, id, item, size, hovered, cx)
        }
        (ShopKind::Effect, Some(item)) => art::effect_card(item, clock, width, height, hovered, cx),
        (ShopKind::Nameplate, Some(item)) => {
            art::nameplate_card(app, item, width, height, hovered, cx)
        }
        (ShopKind::Frame, Some(item)) => art::frame_card(item, width, height, cx),
        (ShopKind::Bundle, _) => art::bundle_card(
            app,
            id.clone(),
            product,
            clock,
            [width, height],
            hovered,
            cx,
        ),
        _ => art::other_card(product, (height * 0.5).min(96.)),
    }
}

fn poster(
    product: &ShopProduct,
    id: &SharedString,
    motion: Motion,
    [width, height]: [f32; 2],
    cx: &App,
) -> Option<AnyElement> {
    if product.kind != ShopKind::Bundle {
        return None;
    }
    let foreground = product.art.foreground.as_deref()?;
    let clip = match motion {
        Motion::In(entered) if !cx.reduce_motion() => {
            product.art.video.as_deref().and_then(|url| {
                let spec = ImageSpec::Video {
                    url: url.to_owned(),
                    width: 1024,
                    height: 512,
                };
                let frames = crate::media::video::resolve(&spec, theme::image_px(width * 2.2), cx)?;
                Some(
                    div().absolute().inset_0().child(
                        crate::media::animated::image(frames)
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .id(SharedString::from(format!("{id}-clip-{entered}"))),
                    ),
                )
            })
        }
        _ => None,
    };
    let zoom = div().absolute().w(rpx(width)).h(rpx(height));
    let zoom = match clip {
        Some(clip) => zoom.child(clip),
        None => zoom.children(art::poster_layer(foreground, width, cx)),
    };
    let layer = animate(zoom, id, "zoom", motion, move |layer, level| {
        let grow = ZOOM * level;
        layer
            .left(rpx(-width * grow / 2.))
            .top(rpx(-height * grow / 2.))
            .w(rpx(width * (1. + grow)))
            .h(rpx(height * (1. + grow)))
    });
    Some(
        div()
            .absolute()
            .inset_0()
            .bg(black())
            .children(
                product
                    .art
                    .background
                    .as_deref()
                    .and_then(|url| art::poster_layer(url, width, cx)),
            )
            .child(layer)
            .into_any_element(),
    )
}

pub(super) fn card(app: &DiscordApp, section: u32, sku: u64, width: f32, cx: &App) -> AnyElement {
    let shop = &app.home.shop;
    let id = SharedString::from(format!("shop-card-{section}-{sku}"));
    let Some(product) = shop.product(sku) else {
        return skeleton(id, width, cx);
    };
    let key = (section, sku);
    let hovered = shop.hovered == Some(key);
    let owned = shop.owns(product);
    let buyable = !owned && !product.prices.is_empty();
    let motion = motion(shop, key);
    let inner = [width - 4., CARD_HEIGHT - 4.];
    let art_height = CARD_HEIGHT - FOOTER;
    let poster = poster(product, &id, motion, inner, cx);
    let entity = app.entity.clone();
    let price = div()
        .absolute()
        .top_0()
        .left_0()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(rpx(8.))
        .child(price_line(product, nitro(app), owned, 16.))
        .children(swatches(product));
    let swap = if buyable { motion } else { Motion::Rest };
    let price = animate(price, &id, "price", swap, |price, level| {
        price.opacity(1. - level)
    });
    let buttons = (buyable && hovered).then(|| {
        let buttons = div()
            .absolute()
            .bottom_0()
            .left_0()
            .w_full()
            .child(actions(app, &id, product));
        animate(buttons, &id, "buttons", motion, |buttons, level| {
            buttons.opacity(level)
        })
    });
    let slot = div()
        .relative()
        .w_full()
        .h(rpx(20.))
        .child(price)
        .children(buttons);
    let slot = animate(slot, &id, "slot", swap, |slot, level| {
        slot.h(rpx(20. + (BUTTONS - 20.) * level))
    });
    let footer = div()
        .absolute()
        .left_0()
        .bottom_0()
        .w_full()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .p(rpx(12.))
        .when(poster.is_some(), |this| {
            this.pt(rpx(48.)).bg(linear_gradient(
                0.,
                linear_color_stop(rgba(0x000000EB), 0.),
                linear_color_stop(rgba(0x00000000), 1.),
            ))
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .child(display_name(product)),
        )
        .child(slot);
    let art = poster.unwrap_or_else(|| {
        let area = div()
            .absolute()
            .left_0()
            .w_full()
            .h(rpx(art_height))
            .flex()
            .items_center()
            .justify_center()
            .child(product_art(
                app,
                &id,
                product,
                [inner[0], art_height],
                hovered,
                cx,
            ));
        animate(area, &id, "lift", motion, |area, level| {
            area.top(rpx(-LIFT * level))
        })
    });
    div()
        .id(id.clone())
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(CARD_HEIGHT))
        .rounded(rpx(RADIUS))
        .overflow_hidden()
        .border_2()
        .border_color(rgba(if hovered {
            theme::border_strong()
        } else {
            BORDER_SUBTLE
        }))
        .bg(rgb(theme::base_lowest()))
        .cursor_pointer()
        .on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, cx| app.hover_shop_card(key, *hovered, cx))
                .ok();
        })
        .on_click(on_app(app, move |app, _, cx| app.open_shop_detail(sku, cx)))
        .child(art)
        .children(
            badge(product).map(|badge| div().absolute().top(rpx(12.)).left(rpx(12.)).child(badge)),
        )
        .child(footer)
        .into_any_element()
}

pub(super) fn skeleton(id: SharedString, width: f32, cx: &App) -> AnyElement {
    div()
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(CARD_HEIGHT))
        .rounded(rpx(RADIUS))
        .overflow_hidden()
        .border_2()
        .border_color(rgba(BORDER_SUBTLE))
        .bg(rgb(theme::base_lowest()))
        .child(
            div()
                .absolute()
                .left(rpx(12.))
                .bottom(rpx(12.))
                .flex()
                .flex_col()
                .gap(rpx(8.))
                .child(pulse(
                    SharedString::from(format!("{id}-name")),
                    div().w(rpx(width * 0.5)).h(rpx(18.)).rounded(rpx(9.)),
                    cx,
                ))
                .child(pulse(
                    SharedString::from(format!("{id}-price")),
                    div().w(rpx(width * 0.3)).h(rpx(18.)).rounded(rpx(9.)),
                    cx,
                )),
        )
        .into_any_element()
}

pub(super) fn row(
    app: &DiscordApp,
    section: u32,
    index: usize,
    skus: &[Option<u64>],
    width: f32,
    cx: &App,
) -> AnyElement {
    let (_, card_width) = columns(width);
    div()
        .flex()
        .gap(rpx(GRID_GAP))
        .w(rpx(width))
        .pb(rpx(GRID_GAP))
        .children(skus.iter().enumerate().map(|(slot, sku)| match sku {
            Some(sku) => card(app, section, *sku, card_width, cx),
            None => skeleton(
                SharedString::from(format!("shop-skeleton-{section}-{index}-{slot}")),
                card_width,
                cx,
            ),
        }))
        .into_any_element()
}

fn arrow(app: &DiscordApp, section: u32, target: usize, right: bool) -> Stateful<Div> {
    div()
        .id((
            "shop-carousel-arrow",
            section as usize * 2 + usize::from(right),
        ))
        .absolute()
        .top(rpx((CARD_HEIGHT - 40.) / 2.))
        .when(right, |this| this.right(rpx(-20.)))
        .when(!right, |this| this.left(rpx(-20.)))
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(40.))
        .rounded_full()
        .bg(rgb(theme::surface_high()))
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .shadow_md()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(theme::surface_highest())))
        .on_click(on_app(app, move |app, _, cx| {
            app.scroll_shop_carousel(section, target, cx)
        }))
        .child(theme::icon(
            if right { "arrow-right" } else { "arrow-left" },
            20.,
            theme::strong(),
        ))
}

pub(super) fn carousel(
    app: &DiscordApp,
    section: u32,
    skus: &[u64],
    width: f32,
    cx: &App,
) -> AnyElement {
    let (columns, card_width) = columns(width);
    let last = skus.len().saturating_sub(columns);
    let offset = app
        .home
        .shop
        .carousels
        .get(&section)
        .copied()
        .unwrap_or(0)
        .min(last);
    div()
        .relative()
        .flex()
        .gap(rpx(GRID_GAP))
        .w(rpx(width))
        .h(rpx(CARD_HEIGHT))
        .children(
            skus.iter()
                .skip(offset)
                .take(columns)
                .map(|sku| card(app, section, *sku, card_width, cx)),
        )
        .when(offset > 0, |this| {
            this.child(arrow(app, section, offset.saturating_sub(columns), false))
        })
        .when(offset < last, |this| {
            this.child(arrow(app, section, (offset + columns).min(last), true))
        })
        .into_any_element()
}
