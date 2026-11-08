use super::art::app_asset;
use super::*;

pub(in crate::app::profile) fn wishlist_panel(
    app: &DiscordApp,
    card: &ProfileCard,
    cx: &App,
) -> AnyElement {
    let palette = card.palette();
    let Some(wishlist) = card.wishlist.as_deref() else {
        return div().into_any_element();
    };
    div()
        .flex()
        .flex_col()
        .gap(rpx(WISHLIST_GAP))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.text))
                        .child(format!(
                            "{} {}",
                            wishlist.items.len(),
                            if wishlist.items.len() == 1 {
                                "Item"
                            } else {
                                "Items"
                            }
                        )),
                )
                .child(shop_menu(app, card, wishlist, palette, cx)),
        )
        .child(
            div().grid().grid_cols(3).gap(rpx(WISHLIST_GAP)).children(
                wishlist
                    .items
                    .iter()
                    .map(|item| wishlist_tile(app, card, wishlist, item, palette, cx)),
            ),
        )
        .into_any_element()
}

fn shop_menu(
    app: &DiscordApp,
    card: &ProfileCard,
    wishlist: &Wishlist,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement {
    let open = card.shop_menu_open;
    let entity = app.entity.clone();
    let button = div()
        .id("profile-wishlist-shop")
        .flex()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(36.))
        .px(rpx(14.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(if open { palette.raised } else { palette.button })
        .hover(move |style| style.bg(palette.raised))
        .text_size(rpx(15.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(palette.strong))
        .child("Browse the Shop")
        .child(theme::icon(
            if open { "x" } else { "chevron-down" },
            16.,
            palette.strong,
        ))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.set_shop_menu(!open, cx))
                .ok();
        });
    div().relative().child(button).when(open, |this| {
        let entity = app.entity.clone();
        let mut entries = vec![(
            SharedString::from("Discord Shop"),
            None,
            "https://discord.com/shop".to_owned(),
        )];
        entries.extend(wishlist.applications.iter().map(|application| {
            let icon = application
                .icon
                .as_deref()
                .and_then(|icon| app_asset(application.id, icon, 20., cx));
            (
                SharedString::from(format!("{} Shop", application.name)),
                Some(icon),
                format!(
                    "https://discord.com/application-directory/{}/store",
                    application.id
                ),
            )
        }));
        let close = entity.clone();
        let menu = div()
            .id("profile-wishlist-shop-menu")
            .flex()
            .flex_col()
            .min_w(rpx(220.))
            .p(rpx(8.))
            .rounded(rpx(8.))
            .bg(rgb(theme::elevated()))
            .border_1()
            .border_color(rgb(theme::menu_border()).opacity(0.06))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(move |_, _, cx| {
                close
                    .update(cx, |app, cx| app.set_shop_menu(false, cx))
                    .ok();
            })
            .children(
                entries
                    .into_iter()
                    .enumerate()
                    .map(|(index, (name, icon, url))| {
                        let entity = entity.clone();
                        div()
                            .id(("profile-wishlist-shop-entry", index))
                            .flex()
                            .items_center()
                            .gap(rpx(10.))
                            .h(rpx(40.))
                            .px(rpx(10.))
                            .rounded(rpx(4.))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgba(theme::selected())))
                            .text_size(rpx(15.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::strong()))
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .size(rpx(20.))
                                    .map(|this| match icon {
                                        None => {
                                            this.child(theme::icon("store", 20., theme::strong()))
                                        }
                                        Some(Some(path)) => {
                                            this.child(img(path).size_full().rounded(rpx(4.)))
                                        }
                                        Some(None) => {
                                            this.rounded(rpx(4.)).bg(rgba(theme::selected()))
                                        }
                                    }),
                            )
                            .child(name)
                            .on_click(move |_, _, cx| {
                                cx.open_url(&url);
                                entity
                                    .update(cx, |app, cx| app.set_shop_menu(false, cx))
                                    .ok();
                            })
                    }),
            );
        this.child(
            div().absolute().top(relative(1.)).right_0().child(
                deferred(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .child(menu.with_animation(
                            "profile-wishlist-shop-menu-enter",
                            Animation::new(SHOP_MENU_ENTER).with_easing(ease_out_quint()),
                            |menu, delta| menu.opacity(delta).mt(rpx(2. + 6. * delta)),
                        )),
                )
                .with_priority(2),
            ),
        )
    })
}

fn wishlist_tile(
    app: &DiscordApp,
    card: &ProfileCard,
    wishlist: &Wishlist,
    item: &WishlistItem,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement {
    let store_art = |asset: Option<u64>, pixels: u32| {
        let spec = ImageSpec::StoreAsset {
            application_id: item.application_id?,
            asset_id: asset?,
        };
        images::resolve(&spec, pixels, cx)
    };
    let layers: Vec<_> = match (
        store_art(item.card_background_asset_id, 1280),
        store_art(item.card_image_asset_id, 640),
    ) {
        (None, None) => store_art(item.thumbnail_asset_id, 640)
            .into_iter()
            .collect(),
        (background, card) => background.into_iter().chain(card).collect(),
    };
    let application = item.application_id.and_then(|id| {
        wishlist
            .applications
            .iter()
            .find(|application| application.id == id)
    });
    let badge = application
        .and_then(|application| app_asset(application.id, application.icon.as_deref()?, 20., cx));
    let tip = match (&item.price, item.is_owned) {
        (_, true) => format!("{} · Owned", item.name),
        (Some(price), false) => format!("{} · {}", item.name, sku_price(price)),
        (None, false) => item.name.clone(),
    };
    let decoration = item
        .collectible
        .as_ref()
        .filter(|collectible| collectible.kind == 0)
        .and_then(|collectible| collectible.asset.clone())
        .map(|asset| {
            let pixels = theme::image_px(WISHLIST_AVATAR * DECORATION_SCALE);
            let animated = asset.starts_with("a_").then(|| {
                let spec = ImageSpec::AvatarDecoration {
                    asset: asset.clone(),
                    animated: true,
                };
                animated::resolve(&spec, pixels, cx)
            });
            let art = match animated.flatten() {
                Some(frames) => Some(
                    animated::image(frames)
                        .id(("profile-wishlist-decoration", item.sku_id))
                        .size_full()
                        .into_any_element(),
                ),
                None => {
                    let spec = ImageSpec::AvatarDecoration {
                        asset,
                        animated: false,
                    };
                    images::resolve(&spec, pixels, cx)
                        .map(|path| img(path).size_full().into_any_element())
                }
            };
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .relative()
                        .size(rpx(WISHLIST_AVATAR))
                        .child(theme::undecorated_member_avatar(
                            &app.store,
                            card.guild_id.unwrap_or(0),
                            card.user_id,
                            WISHLIST_AVATAR,
                            cx,
                        ))
                        .children(art.map(|art| {
                            let frame = WISHLIST_AVATAR * DECORATION_SCALE;
                            div()
                                .absolute()
                                .left(rpx((WISHLIST_AVATAR - frame) / 2.))
                                .top(rpx((WISHLIST_AVATAR - frame) / 2.))
                                .size(rpx(frame))
                                .child(art)
                        })),
                )
        });
    div()
        .id(("profile-wishlist-item", item.sku_id))
        .relative()
        .w_full()
        .aspect_ratio(1.)
        .rounded(rpx(12.))
        .bg(palette.hover)
        .child(theme::hover_tooltip(
            ("profile-wishlist-tip", item.sku_id),
            tip,
        ))
        .children(decoration)
        .children(layers.into_iter().map(|path| {
            img(path)
                .absolute()
                .inset_0()
                .size_full()
                .rounded(rpx(12.))
                .object_fit(ObjectFit::Cover)
        }))
        .children(badge.map(|path| {
            div()
                .absolute()
                .top(rpx(8.))
                .left(rpx(8.))
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(WISHLIST_BADGE))
                .rounded(rpx(6.))
                .bg(hsla(0., 0., 0., 0.6))
                .child(img(path).size(rpx(18.)).rounded(rpx(4.)))
        }))
}

pub(super) fn sku_price(price: &SkuPrice) -> String {
    let amount = price.amount as f64 / 10f64.powi(price.exponent as i32);
    let digits = price.exponent as usize;
    match price.currency.as_str() {
        "usd" => format!("${amount:.digits$}"),
        "eur" => format!("€{amount:.digits$}"),
        "gbp" => format!("£{amount:.digits$}"),
        currency => format!("{amount:.digits$} {}", currency.to_uppercase()),
    }
}
