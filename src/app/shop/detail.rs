use super::controls::{BORDER_SUBTLE, Tone, art as asset, button, on_app};
use super::*;
use crate::{
    app::{DIALOG_EXIT, direct::home::HomePage},
    discord::model::CollectibleItem,
};

const WIDTH: f32 = 960.;
const HEIGHT: f32 = 620.;
const LEFT: f32 = 360.;
const PREVIEW: f32 = 300.;
const POP_ENTER: Duration = Duration::from_millis(200);

pub(super) struct Detail {
    sku: u64,
    item: usize,
}

impl DiscordApp {
    pub(super) fn open_shop_detail(&mut self, sku: u64, cx: &mut Context<Self>) {
        let shop = &mut self.home.shop;
        shop.detail = Some(Detail { sku, item: 0 });
        shop.detail_clock.replace(None);
        shop.detail_closing = None;
        shop.detail_shown += 1;
        shop.hovered = None;
        shop.menu = None;
        cx.notify();
    }

    pub(super) fn close_shop_detail(&mut self, cx: &mut Context<Self>) -> bool {
        let shop = &mut self.home.shop;
        if shop.detail.is_none() || shop.detail_closing.is_some() {
            return false;
        }
        shop.detail_closing = Some(remove_after_exit(cx, |shop| {
            shop.detail = None;
            shop.detail_closing = None;
        }));
        cx.notify();
        true
    }

    fn select_shop_detail(&mut self, sku: u64, item: usize, cx: &mut Context<Self>) {
        let shop = &mut self.home.shop;
        shop.detail = Some(Detail { sku, item });
        shop.detail_clock.replace(None);
        if !shop.products.contains_key(&sku) {
            self.hydrate_shop(vec![sku], cx);
        }
        cx.notify();
    }

    pub(super) fn show_shop_notice(&mut self, sku: u64, cx: &mut Context<Self>) {
        let shop = &mut self.home.shop;
        shop.notice = Some(sku);
        shop.notice_closing = None;
        shop.notice_shown += 1;
        cx.notify();
    }

    pub(super) fn close_shop_notice(&mut self, cx: &mut Context<Self>) -> bool {
        let shop = &mut self.home.shop;
        if shop.notice.is_none() || shop.notice_closing.is_some() {
            return false;
        }
        shop.notice_closing = Some(remove_after_exit(cx, |shop| {
            shop.notice = None;
            shop.notice_closing = None;
        }));
        cx.notify();
        true
    }
}

fn remove_after_exit(cx: &mut Context<DiscordApp>, remove: fn(&mut Shop)) -> Task<()> {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(DIALOG_EXIT).await;
        this.update(cx, |app, cx| {
            remove(&mut app.home.shop);
            cx.notify();
        })
        .ok();
    })
}

fn pop(
    name: &'static str,
    shown: u64,
    closing: bool,
    backdrop: Stateful<Div>,
    panel: Stateful<Div>,
) -> AnyElement {
    let (phase, duration) = if closing {
        ("out", DIALOG_EXIT)
    } else {
        ("in", POP_ENTER)
    };
    let key = SharedString::from(format!("{name}-{phase}-{shown}"));
    let animation = Animation::new(duration).with_easing(ease_out_quint());
    let progress = move |delta: f32| if closing { 1. - delta } else { delta };
    backdrop
        .child(panel.relative().with_animation(
            key.clone(),
            animation.clone(),
            move |panel, delta| {
                let shown = progress(delta);
                panel.opacity(shown).top(rpx(16. * (1. - shown)))
            },
        ))
        .with_animation(key, animation, move |backdrop, delta| {
            backdrop.bg(rgba(theme::SCRIM).opacity(progress(delta)))
        })
        .into_any_element()
}

fn kind_label(item: &CollectibleItem) -> &'static str {
    ShopProduct::item_kind(item).label()
}

fn preview_item(product: &ShopProduct, index: usize) -> Option<&CollectibleItem> {
    product.items.get(index).or_else(|| product.items.first())
}

fn item_name(product: &ShopProduct, item: &CollectibleItem) -> String {
    product
        .bundled
        .iter()
        .find(|bundled| bundled.sku_id == item.sku_id)
        .map(|bundled| bundled.name.clone())
        .or_else(|| item.title.clone())
        .unwrap_or_else(|| product.name.clone())
}

fn summary(product: &ShopProduct) -> String {
    if product.kind == ShopKind::Bundle {
        return format!("Bundle includes {} items", product.items.len());
    }
    if product.summary.contains("{joinedItems}") {
        return String::new();
    }
    product.summary.clone()
}

fn user_avatar(
    app: &DiscordApp,
    size: f32,
    item: Option<&CollectibleItem>,
    cx: &App,
) -> AnyElement {
    let user_id = app.store.current_user_id;
    let user = app.store.user(user_id);
    let decoration = item
        .filter(|item| ShopProduct::item_kind(item) == ShopKind::Decoration)
        .and_then(art::decoration_of)
        .or_else(|| user.and_then(|user| user.decoration.clone()));
    div()
        .id(SharedString::from(format!("shop-detail-avatar-{size}")))
        .flex_none()
        .child(theme::preview_avatar(
            user,
            user_id,
            size,
            None,
            None,
            None,
            decoration.as_ref(),
            cx,
        ))
        .into_any_element()
}

fn message_mock(app: &DiscordApp, item: &CollectibleItem, cx: &App) -> AnyElement {
    let name = app
        .store
        .user(app.store.current_user_id)
        .map_or_else(String::new, |user| user.name.clone());
    div()
        .flex()
        .gap(rpx(12.))
        .w(rpx(PREVIEW))
        .p(rpx(16.))
        .rounded(rpx(8.))
        .bg(rgb(theme::bg()))
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .shadow_lg()
        .child(user_avatar(app, 40., Some(item), cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(2.))
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child(name),
                )
                .child(
                    div()
                        .text_size(rpx(16.))
                        .text_color(rgb(theme::text()))
                        .child("Look at my beautiful decoration"),
                ),
        )
        .into_any_element()
}

fn stage(app: &DiscordApp, product: &ShopProduct, index: usize, cx: &App) -> AnyElement {
    let clock = &app.home.shop.detail_clock;
    let Some(item) = preview_item(product, index) else {
        return art::other_card(product, 120.);
    };
    match ShopProduct::item_kind(item) {
        ShopKind::Decoration => div()
            .flex()
            .flex_col()
            .items_center()
            .gap(rpx(32.))
            .child(user_avatar(app, 120., Some(item), cx))
            .child(message_mock(app, item, cx))
            .into_any_element(),
        ShopKind::Nameplate => {
            let plate = art::nameplate_of(item);
            div()
                .flex()
                .flex_col()
                .gap(rpx(6.))
                .p(rpx(12.))
                .rounded(rpx(8.))
                .bg(rgb(theme::bg()))
                .shadow_lg()
                .child(
                    div()
                        .opacity(0.5)
                        .child(art::plate_row(app, None, PREVIEW, false, false, cx)),
                )
                .child(art::plate_row(app, plate.as_ref(), PREVIEW, true, true, cx))
                .child(
                    div()
                        .opacity(0.5)
                        .child(art::plate_row(app, None, PREVIEW, false, false, cx)),
                )
                .into_any_element()
        }
        ShopKind::Effect => {
            let width = PREVIEW * 0.9;
            let height = width * 1.35;
            div()
                .relative()
                .w(rpx(width))
                .h(rpx(height))
                .rounded(rpx(art::MOCK_RADIUS))
                .overflow_hidden()
                .shadow_lg()
                .child(art::profile_mock(width, height))
                .children(art::effect(item, clock, width, art::MOCK_RADIUS, true, cx))
                .into_any_element()
        }
        ShopKind::Frame => art::framed_mock(item, PREVIEW * 0.8, PREVIEW, cx),
        ShopKind::Bundle | ShopKind::Other => art::other_card(product, 120.),
    }
}

fn thumbnail(
    app: &DiscordApp,
    product: &ShopProduct,
    index: usize,
    selected: bool,
    cx: &App,
) -> AnyElement {
    let sku = product.sku_id;
    let item = &product.items[index];
    let id = SharedString::from(format!("shop-detail-item-{sku}-{index}"));
    let art: AnyElement = match ShopProduct::item_kind(item) {
        ShopKind::Decoration => art::decorated_placeholder(id.clone(), item, 30., false, cx),
        ShopKind::Nameplate => art::nameplate_of(item)
            .and_then(|plate| {
                crate::app::members::member_list::rounded_nameplate_layer(
                    &plate,
                    app.store.current_user_id,
                    52.,
                    false,
                    6.,
                    cx,
                )
            })
            .unwrap_or_else(|| div().into_any_element()),
        ShopKind::Effect => item
            .thumbnail_preview_src
            .as_deref()
            .and_then(|url| asset(url, 52., cx))
            .map(|image| {
                image
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .into_any_element()
            })
            .unwrap_or_else(|| div().into_any_element()),
        _ => theme::icon("store", 24., theme::muted()).into_any_element(),
    };
    div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(56.))
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(rgb(theme::surface_high()))
        .border_2()
        .border_color(if selected {
            rgb(theme::BLURPLE)
        } else {
            rgba(BORDER_SUBTLE)
        })
        .cursor_pointer()
        .on_click(on_app(app, move |app, _, cx| {
            app.select_shop_detail(sku, index, cx)
        }))
        .child(art)
        .into_any_element()
}

fn purchase(app: &DiscordApp, product: &ShopProduct) -> AnyElement {
    let shop = &app.home.shop;
    let nitro = nitro(app);
    let sku = product.sku_id;
    if shop.owns(product) {
        return div()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .h(rpx(40.))
            .text_size(rpx(16.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme::strong()))
            .child(theme::icon("check", 18., theme::GREEN))
            .child("You own this item")
            .into_any_element();
    }
    let money = product.prices.local(nitro);
    let orbs = product.prices.orbs();
    let price_line = money.map(|money| {
        div()
            .flex()
            .items_center()
            .gap(rpx(6.))
            .text_size(rpx(20.))
            .line_height(rpx(24.))
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(theme::strong()))
            .when(nitro && !product.prices.nitro.is_empty(), |this| {
                this.child(theme::icon("nitro", 20., theme::strong()))
            })
            .child(price::money(money, false))
            .children(price::discount(product, money).map(|percent| {
                div()
                    .text_color(rgb(theme::GREEN))
                    .child(format!("(-{percent}%)"))
            }))
    });
    let nitro_line = price::nitro_offer(product, nitro).map(|offer| {
        div()
            .flex()
            .items_center()
            .gap(rpx(6.))
            .text_size(rpx(14.))
            .text_color(rgb(theme::muted()))
            .child(theme::icon("nitro", 14., theme::muted()))
            .child(format!("{offer} with Nitro"))
    });
    let orbs_line = orbs.map(|orbs| {
        div()
            .flex()
            .items_center()
            .justify_between()
            .text_size(rpx(14.))
            .text_color(rgb(theme::text()))
            .child("Using Orbs")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .text_size(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .child(theme::icon("orb", 16., theme::strong()))
                    .child(price::grouped(orbs)),
            )
    });
    let redeem = orbs.map(|orbs| {
        button(
            "shop-detail-redeem",
            format!("Redeem {} Orbs", price::grouped(orbs)),
            Tone::Brand,
        )
        .w_full()
        .on_click(on_app(app, move |app, _, cx| app.show_shop_notice(sku, cx)))
    });
    let buy = money.map(|money| {
        div()
            .flex()
            .gap(rpx(8.))
            .w_full()
            .child(
                button(
                    "shop-detail-buy",
                    format!("Buy for {}", price::money(money, true)),
                    if orbs.is_some() {
                        Tone::Subtle
                    } else {
                        Tone::Brand
                    },
                )
                .flex_1()
                .on_click(on_app(app, move |app, _, cx| app.show_shop_notice(sku, cx))),
            )
            .child(
                button("shop-detail-gift", "", Tone::Brand)
                    .w(rpx(40.))
                    .px_0()
                    .child(theme::icon("gift", 20., 0xFFFFFF))
                    .on_click(on_app(app, move |app, _, cx| app.show_shop_notice(sku, cx))),
            )
    });
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .children(price_line)
        .children(nitro_line)
        .children(orbs_line)
        .child(div().h(rpx(4.)))
        .children(redeem)
        .children(buy)
        .into_any_element()
}

fn modal(app: &DiscordApp, detail: &Detail, window: &Window, cx: &App) -> Option<AnyElement> {
    let shop = &app.home.shop;
    let product = shop.product(detail.sku)?;
    let selected = detail.item.min(product.items.len().saturating_sub(1));
    let collection = product
        .collection_id
        .and_then(|id| shop.collections.get(&id).map(|collection| (id, collection)));
    let viewport = window.viewport_size();
    let unit = rpx(1.).to_pixels(window.rem_size());
    let width = WIDTH.min(viewport.width / unit - 48.);
    let height = HEIGHT.min(viewport.height / unit - 48.);
    let styles = collection
        .map(|(_, collection)| collection.styles.background.clone())
        .filter(|colors| colors.len() >= 2)
        .unwrap_or_else(|| product.styles.background.clone());
    let backdrop_art = collection
        .and_then(|(_, collection)| collection.detail_background.as_deref())
        .and_then(|url| asset(url, width - LEFT, cx))
        .map(|image| {
            div()
                .absolute()
                .inset_0()
                .child(image.size_full().object_fit(ObjectFit::Cover))
        });
    let background = match styles.as_slice() {
        [top, bottom, ..] => linear_gradient(
            180.,
            linear_color_stop(rgb(*top), 0.),
            linear_color_stop(rgb(*bottom), 1.),
        ),
        _ => solid_background(rgb(theme::surface_high())),
    };
    let item = preview_item(product, selected);
    let items = (product.kind == ShopKind::Bundle && product.items.len() > 1).then(|| {
        div()
            .flex()
            .flex_col()
            .gap(rpx(8.))
            .child(
                div().flex().gap(rpx(8.)).children(
                    (0..product.items.len())
                        .map(|index| thumbnail(app, product, index, index == selected, cx)),
                ),
            )
            .children(item.map(|item| {
                div()
                    .flex()
                    .gap(rpx(4.))
                    .text_size(rpx(14.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::strong()))
                            .child(item_name(product, item)),
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::muted()))
                            .child(format!("- {}", kind_label(item))),
                    )
            }))
    });
    let variants = (product.variants.len() > 1).then(|| {
        div()
            .flex()
            .flex_wrap()
            .gap(rpx(8.))
            .children(product.variants.iter().enumerate().map(|(index, variant)| {
                let target = variant.sku_id;
                let current = target == product.sku_id;
                div()
                    .id(("shop-detail-variant", index))
                    .size(rpx(28.))
                    .rounded_full()
                    .cursor_pointer()
                    .border_2()
                    .border_color(if current {
                        white()
                    } else {
                        transparent_black()
                    })
                    .bg(rgb(variant.swatch.unwrap_or(0x5C5E66)))
                    .on_click(on_app(app, move |app, _, cx| {
                        app.select_shop_detail(target, 0, cx)
                    }))
                    .child(theme::hover_tooltip(
                        SharedString::from(format!("shop-detail-variant-tip-{index}")),
                        variant.label.clone(),
                    ))
            }))
    });
    let left = div()
        .flex()
        .flex_col()
        .flex_none()
        .w(rpx(LEFT))
        .h_full()
        .p(rpx(24.))
        .gap(rpx(16.))
        .bg(rgb(theme::bg()))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(4.))
                .child(
                    div()
                        .text_size(rpx(24.))
                        .line_height(rpx(30.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(theme::strong()))
                        .child(product.name.clone()),
                )
                .child(
                    div()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::text()))
                        .child(summary(product)),
                ),
        )
        .children(items)
        .children(variants)
        .child(div().flex_1())
        .child(purchase(app, product));
    let chip = collection.map(|(id, collection)| {
        div()
            .id("shop-detail-collection")
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(32.))
            .px(rpx(10.))
            .rounded(rpx(8.))
            .bg(rgba(0x000000A3))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(0x000000CC)))
            .text_size(rpx(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(white())
            .on_click(on_app(app, move |app, _, cx| {
                app.open_shop_collection(id, cx)
            }))
            .child(theme::icon("store", 16., 0xFFFFFF))
            .child(collection.name.clone())
    });
    let close = div()
        .id("shop-detail-close")
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(32.))
        .rounded(rpx(8.))
        .bg(rgba(0x000000A3))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(0x000000CC)))
        .on_click(on_app(app, |app, _, cx| {
            app.close_shop_detail(cx);
        }))
        .child(theme::icon("x", 18., 0xFFFFFF));
    let right = div()
        .relative()
        .flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .bg(background)
        .children(backdrop_art)
        .child(div().relative().child(stage(app, product, selected, cx)))
        .child(
            div()
                .absolute()
                .top(rpx(16.))
                .left(rpx(16.))
                .right(rpx(16.))
                .flex()
                .justify_between()
                .child(div().children(chip))
                .child(close),
        );
    let close_backdrop = app.entity.clone();
    let backdrop = div()
        .id("shop-detail-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_click(move |_, _, cx| {
            close_backdrop
                .update(cx, |app, cx| {
                    app.close_shop_detail(cx);
                })
                .ok();
        })
        .child(theme::selection_guard());
    let panel = div()
        .id("shop-detail")
        .flex()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(12.))
        .overflow_hidden()
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
        .occlude()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(left)
        .child(right);
    Some(pop(
        "shop-detail",
        shop.detail_shown,
        shop.detail_closing.is_some(),
        backdrop,
        panel,
    ))
}

fn notice(app: &DiscordApp, sku: u64) -> AnyElement {
    let name = app.home.shop.product(sku).map_or_else(
        || "this item".to_owned(),
        |product| card::display_name(product),
    );
    let dismiss = app.entity.clone();
    let backdrop = div()
        .id("shop-notice-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_click(move |_, _, cx| {
            dismiss
                .update(cx, |app, cx| {
                    app.close_shop_notice(cx);
                })
                .ok();
        });
    let panel = div()
        .id("shop-notice")
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .w(rpx(440.))
        .p(rpx(24.))
        .rounded(rpx(12.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .occlude()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(theme::strong()))
                .child("Buy it in Discord"),
        )
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .text_color(rgb(theme::muted()))
                .child(format!(
                    "zcord can only browse the Shop. To buy, gift or redeem {name}, \
                             open it in the official Discord app or on discord.com."
                )),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .gap(rpx(8.))
                .mt(rpx(16.))
                .child(
                    button("shop-notice-close", "Close", Tone::Subtle).on_click(on_app(
                        app,
                        |app, _, cx| {
                            app.close_shop_notice(cx);
                        },
                    )),
                )
                .child(
                    button("shop-notice-open", "Open on discord.com", Tone::Brand)
                        .child(theme::icon("external-link", 16., 0xFFFFFF))
                        .on_click(on_app(app, move |app, _, cx| {
                            app.close_shop_notice(cx);
                            cx.open_url(&format!("https://discord.com/shop#itemSkuId={sku}"));
                        })),
                ),
        );
    let shop = &app.home.shop;
    pop(
        "shop-notice",
        shop.notice_shown,
        shop.notice_closing.is_some(),
        backdrop,
        panel,
    )
}

pub(crate) fn overlay(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    if !(app.showing_home_page() && app.home.page == HomePage::Shop) {
        return None;
    }
    let shop = &app.home.shop;
    let modal = shop
        .detail
        .as_ref()
        .and_then(|detail| modal(app, detail, window, cx));
    let notice = shop.notice.map(|sku| notice(app, sku));
    if modal.is_none() && notice.is_none() {
        return None;
    }
    Some(
        div()
            .absolute()
            .inset_0()
            .children(modal)
            .children(notice)
            .into_any_element(),
    )
}
