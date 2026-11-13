use gpui_component::input::Input;

use super::controls::on_app;
use super::*;
use crate::{app::chat, discord::model::ShopBlock};

const CARET_FLIP: Duration = Duration::from_millis(200);
const MENU_GAP: f32 = 4.;

fn underlined(id: &'static str, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h_full()
        .pt(rpx(2.))
        .border_b_2()
        .border_color(if active {
            rgb(theme::strong()).into()
        } else {
            transparent_black()
        })
        .when(!active, |this| {
            this.hover(|style| style.border_color(rgb(theme::BLURPLE)))
        })
        .cursor_pointer()
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::text()))
}

fn tab(
    app: &DiscordApp,
    id: &'static str,
    label: impl Into<SharedString>,
    active: bool,
    target: ShopTab,
) -> Stateful<Div> {
    underlined(id, active)
        .gap(rpx(4.))
        .on_click(on_app(app, move |app, _, cx| {
            app.select_shop_tab(target, cx)
        }))
        .child(label.into())
}

fn caret(shop: &Shop, open: bool, cx: &App) -> AnyElement {
    let (was_open, flips) = shop.caret.get();
    let flips = flips + u64::from(was_open != open);
    shop.caret.set((open, flips));
    let icon = theme::icon("chevron-down", 18., theme::text());
    let turn = |icon: Svg, level: f32| {
        icon.with_transformation(Transformation::rotate(radians(
            std::f32::consts::PI * level,
        )))
    };
    if flips == 0 || cx.reduce_motion() {
        return turn(icon, if open { 1. } else { 0. }).into_any_element();
    }
    icon.with_animation(
        SharedString::from(format!("shop-browse-caret-{flips}")),
        Animation::new(CARET_FLIP).with_easing(ease_out_quint()),
        move |icon, delta| turn(icon, if open { delta } else { 1. - delta }),
    )
    .into_any_element()
}

fn browse_menu(app: &DiscordApp) -> Option<AnyElement> {
    if app.home.shop.menu != Some(Menu::Browse) {
        return None;
    }
    let close = app.entity.clone();
    let hover = app.entity.clone();
    let item = |index: usize, label: &'static str, kind: Option<ShopKind>, collabs: bool| {
        div()
            .id(("shop-browse-menu-item", index))
            .flex()
            .items_center()
            .h(rpx(32.))
            .px(rpx(8.))
            .rounded(rpx(4.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::hover())))
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::strong()))
            .on_click(on_app(app, move |app, _, cx| {
                app.home.shop.menu = None;
                app.browse_only(kind, collabs, cx);
            }))
            .child(label)
    };
    let mut items: Vec<Stateful<Div>> = browse::TYPES
        .iter()
        .enumerate()
        .map(|(index, (kind, label))| item(index, label, Some(*kind), false))
        .collect();
    items.push(item(browse::TYPES.len(), "Collabs", None, true));
    items.push(item(browse::TYPES.len() + 1, "Shop All", None, false));
    Some(
        div()
            .absolute()
            .top(rpx(chat::HEADER_HEIGHT + MENU_GAP))
            .left_0()
            .size_0()
            .child(
                deferred(
                    anchored().anchor(Anchor::TopLeft).child(
                        div()
                            .id("shop-browse-menu")
                            .flex()
                            .flex_col()
                            .w(rpx(160.))
                            .p(rpx(6.))
                            .rounded(rpx(8.))
                            .bg(rgb(theme::surface_high()))
                            .border_1()
                            .border_color(rgba(theme::outline()))
                            .shadow_lg()
                            .occlude()
                            .on_hover(move |hovered, _, cx| {
                                hover
                                    .update(cx, |app, cx| {
                                        app.hover_shop_browse(BrowseHover::Menu, *hovered, cx)
                                    })
                                    .ok();
                            })
                            .on_mouse_down_out(move |_, _, cx| {
                                close
                                    .update(cx, |app, cx| {
                                        app.close_shop_menu(cx);
                                    })
                                    .ok();
                            })
                            .children(items),
                    ),
                )
                .with_priority(2),
            )
            .into_any_element(),
    )
}

fn promo_link(app: &DiscordApp) -> Option<Stateful<Div>> {
    let shop = &app.home.shop;
    let promo = shop
        .page(PageKind::Home)?
        .blocks
        .iter()
        .find_map(|block| match block {
            ShopBlock::Promos(promos) => promos.first(),
            _ => None,
        })?;
    let collection_id = promo.collection_id;
    let active = shop.tab == ShopTab::Collection(collection_id);
    let name = promo.name.replace('/', " / ");
    Some(
        tab(
            app,
            "shop-tab-promo",
            name,
            active,
            ShopTab::Collection(collection_id),
        )
        .child(theme::icon("tag", 18., theme::strong()))
        .flex_row_reverse(),
    )
}

fn search_box(app: &DiscordApp) -> Option<Div> {
    let search = app.home.shop.search.as_ref()?;
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(6.))
            .w(rpx(224.))
            .h(rpx(30.))
            .px(rpx(8.))
            .rounded(rpx(6.))
            .bg(rgb(theme::input_bg()))
            .border_1()
            .border_color(rgb(theme::input_border()))
            .child(theme::icon("search", 16., theme::muted()))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(search)
                        .appearance(false)
                        .px_0()
                        .text_size(rpx(14.)),
                ),
            ),
    )
}

fn balance(app: &DiscordApp) -> Option<Div> {
    let orbs = app.home.shop.balance?;
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(8.))
            .h(rpx(32.))
            .px(rpx(10.))
            .rounded(rpx(8.))
            .bg(rgba(controls::MOD_SUBTLE))
            .border_1()
            .border_color(rgba(controls::BORDER_SUBTLE))
            .text_size(rpx(16.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::strong()))
            .child(theme::icon("orb", 18., theme::strong()))
            .child(price::grouped(orbs)),
    )
}

pub(crate) fn header(app: &DiscordApp, cx: &App) -> AnyElement {
    let shop = &app.home.shop;
    let selected = shop.tab;
    let browse_open = shop.menu == Some(Menu::Browse);
    let hover = app.entity.clone();
    let browse = underlined("shop-tab-browse-group", selected == ShopTab::Browse)
        .relative()
        .on_hover(move |hovered, _, cx| {
            hover
                .update(cx, |app, cx| {
                    app.hover_shop_browse(BrowseHover::Tab, *hovered, cx)
                })
                .ok();
        })
        .child(
            div()
                .id("shop-tab-browse")
                .flex()
                .items_center()
                .h_full()
                .pr(rpx(2.))
                .on_click(on_app(app, |app, _, cx| {
                    app.select_shop_tab(ShopTab::Browse, cx)
                }))
                .child("Browse"),
        )
        .child(
            div()
                .id("shop-tab-browse-menu")
                .flex()
                .items_center()
                .h_full()
                .pl(rpx(2.))
                .on_click(on_app(app, |app, _, cx| {
                    app.hover_shop_browse(BrowseHover::Tab, true, cx)
                }))
                .child(caret(shop, browse_open, cx)),
        )
        .children(browse_menu(app));
    div()
        .flex()
        .items_center()
        .gap(rpx(32.))
        .w_full()
        .h(rpx(chat::HEADER_HEIGHT))
        .flex_none()
        .px_4()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(theme::icon("store", 22., theme::strong()))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(32.))
                .h_full()
                .min_w_0()
                .child(tab(
                    app,
                    "shop-tab-featured",
                    "Featured",
                    selected == ShopTab::Featured,
                    ShopTab::Featured,
                ))
                .child(browse)
                .child(tab(
                    app,
                    "shop-tab-orbs",
                    "Orbs Shop",
                    selected == ShopTab::Orbs,
                    ShopTab::Orbs,
                ))
                .children(promo_link(app)),
        )
        .child(div().flex_1())
        .children(search_box(app))
        .children(balance(app))
        .into_any_element()
}
