use super::controls::{Choice, Tone, art, button, on_app, pulse, select};
use super::*;
use crate::discord::model::{ShopBlock, ShopGrid, ShopHero, ShopQuery};

const GRID_LIMIT: usize = 96;
const SHUFFLE_COUNT: u32 = 48;
const HERO_TOP: f32 = 387.;
const BLOCK_GAP: f32 = 48.;
const PROMO_HEIGHT: f32 = 236.;
const COLLECTION_HERO: f32 = 360.;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum GridSort {
    #[default]
    ForYou,
    Popular,
}

#[derive(Default)]
pub(super) struct Grid {
    sort: GridSort,
    shuffled: HashMap<PageKind, Vec<u64>>,
    shuffling: bool,
    generation: u64,
}

fn section_key(kind: PageKind, index: usize) -> u32 {
    let base = match kind {
        PageKind::Home => 0,
        PageKind::Orbs => 1 << 16,
    };
    base | index as u32
}

pub(super) fn hero_has_cards(shop: &Shop, kind: PageKind) -> bool {
    shop.page(kind)
        .and_then(|page| hero_block(page))
        .is_some_and(|hero| !shown(shop, &hero.skus).is_empty())
}

fn hero_block(page: &ShopPage) -> Option<&ShopHero> {
    page.blocks.iter().find_map(|block| match block {
        ShopBlock::Hero(hero) => Some(hero),
        _ => None,
    })
}

fn grid_skus(shop: &Shop, kind: PageKind, grid: &ShopGrid) -> Vec<u64> {
    if let Some(shuffled) = shop.grid.shuffled.get(&kind) {
        return shuffled.clone();
    }
    let source = match shop.grid.sort {
        GridSort::Popular if !grid.popular.is_empty() => &grid.popular,
        _ => &grid.recommended,
    };
    source.iter().copied().take(GRID_LIMIT).collect()
}

pub(super) fn wanted_skus(shop: &Shop, kind: PageKind) -> Vec<u64> {
    let Some(page) = shop.page(kind) else {
        return Vec::new();
    };
    let mut skus = Vec::new();
    for block in &page.blocks {
        match block {
            ShopBlock::Hero(hero) => skus.extend(&hero.skus),
            ShopBlock::Row(row) => skus.extend(&row.skus),
            ShopBlock::Grid(grid) => skus.extend(grid_skus(shop, kind, grid)),
            ShopBlock::Promos(_) | ShopBlock::Info(_) => {}
        }
    }
    skus
}

fn shown(shop: &Shop, skus: &[u64]) -> Vec<u64> {
    skus.iter()
        .copied()
        .filter(|sku| shop.product(*sku).is_some() && !shop.hidden(*sku))
        .collect()
}

pub(super) fn rows(shop: &Shop, kind: PageKind, window: &Window) -> Vec<Row> {
    let state = shop.pages.get(&kind);
    let Some(page) = shop.page(kind) else {
        return if state.is_some_and(|state| state.failed) {
            vec![Row::Message(
                "The Shop didn't load",
                "Something went wrong getting the Shop. Try again later.",
            )]
        } else {
            vec![
                Row::Loading(HERO_TOP + CARD_HEIGHT),
                Row::Gap(BLOCK_GAP),
                Row::Loading(CARD_HEIGHT),
            ]
        };
    };
    let (columns, _) = columns(content_width(shop, window));
    let mut rows = Vec::new();
    let mut hero_shown = false;
    for (index, block) in page.blocks.iter().enumerate() {
        let key = section_key(kind, index);
        match block {
            ShopBlock::Hero(_) if !hero_shown => {
                hero_shown = true;
                rows.push(Row::Hero(kind));
            }
            ShopBlock::Hero(_) => {}
            ShopBlock::Promos(promos) if !promos.is_empty() => {
                rows.push(Row::Promos(promos.clone()));
                rows.push(Row::Gap(BLOCK_GAP));
            }
            ShopBlock::Promos(_) => {}
            ShopBlock::Row(row) => {
                let skus = shown(shop, &row.skus);
                if skus.is_empty() {
                    continue;
                }
                rows.push(Row::Section(Section {
                    title: row.title.clone(),
                    collection: row.collection_id,
                    button: row.button.clone(),
                    skus,
                    key,
                }));
                rows.push(Row::Gap(BLOCK_GAP));
            }
            ShopBlock::Info(info) => {
                rows.push(Row::Info(info.clone()));
                rows.push(Row::Gap(BLOCK_GAP));
            }
            ShopBlock::Grid(grid) => {
                rows.push(Row::GridHeader(kind));
                let skus: Vec<Option<u64>> = grid_skus(shop, kind, grid)
                    .into_iter()
                    .filter(|sku| !shop.hidden(*sku))
                    .map(|sku| shop.product(sku).map(|_| sku))
                    .collect();
                rows.extend(rows_of(key, skus, columns));
                if kind == PageKind::Home {
                    rows.push(Row::GridFooter);
                }
                rows.push(Row::Gap(BLOCK_GAP));
            }
        }
    }
    rows
}

pub(super) fn collection_rows(shop: &Shop, collection_id: u64, window: &Window) -> Vec<Row> {
    let Some(collection) = shop.collections.get(&collection_id) else {
        let failed = shop
            .collection_loads
            .get(&collection_id)
            .is_some_and(|state| state.failed);
        return if failed {
            vec![Row::Message(
                "This collection isn't available",
                "It may have left the Shop. Try another collection.",
            )]
        } else {
            vec![
                Row::Loading(COLLECTION_HERO),
                Row::Gap(BLOCK_GAP),
                Row::Loading(CARD_HEIGHT),
            ]
        };
    };
    let (columns, _) = columns(content_width(shop, window));
    let skus: Vec<Option<u64>> = collection
        .product_skus
        .iter()
        .copied()
        .filter(|sku| !shop.hidden(*sku))
        .map(Some)
        .collect();
    let mut rows = vec![Row::CollectionHero(collection_id), Row::Gap(32.)];
    rows.extend(rows_of(u32::MAX, skus, columns));
    rows.push(Row::Gap(BLOCK_GAP));
    rows
}

pub(super) fn message(title: &'static str, detail: &'static str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(rpx(8.))
        .pt(rpx(96.))
        .text_center()
        .child(
            div()
                .text_size(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(title),
        )
        .child(
            div()
                .max_w(rpx(480.))
                .text_size(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child(detail),
        )
        .into_any_element()
}

pub(super) fn loading(height: f32, cx: &App) -> AnyElement {
    pulse(
        SharedString::from(format!("shop-loading-{height}")),
        div()
            .w_full()
            .max_w(rpx(CONTENT_MAX_WIDTH))
            .h(rpx(height))
            .mt(rpx(24.))
            .rounded(rpx(16.)),
        cx,
    )
}

fn fade(color: u32, height: f32) -> Div {
    div()
        .absolute()
        .left_0()
        .bottom_0()
        .w_full()
        .h(rpx(height))
        .bg(linear_gradient(
            0.,
            linear_color_stop(rgb(color), 0.),
            linear_color_stop(rgba(color << 8), 1.),
        ))
}

fn backdrop(colors: &[u32]) -> Background {
    match colors {
        [top, bottom, ..] => linear_gradient(
            180.,
            linear_color_stop(rgb(*top), 0.),
            linear_color_stop(rgb(*bottom), 1.),
        ),
        _ => rgb(theme::bg()).into(),
    }
}

pub(super) fn hero(
    app: &DiscordApp,
    kind: PageKind,
    width: f32,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let shop = &app.home.shop;
    let Some(hero) = shop.page(kind).and_then(|page| hero_block(page)) else {
        return div().into_any_element();
    };
    let collection = hero.collection_id.and_then(|id| shop.collections.get(&id));
    let full = column_width(window);
    let banner = hero
        .banner
        .as_deref()
        .or_else(|| collection.and_then(|c| c.hero_banner.as_deref()))
        .and_then(|url| art(url, full, cx))
        .map(|image| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(rpx(HERO_TOP + 174.))
                .child(image.size_full().object_fit(ObjectFit::Cover))
        });
    let logo = hero
        .logo
        .as_deref()
        .or_else(|| collection.and_then(|c| c.hero_logo.as_deref()))
        .and_then(|url| art(url, 323., cx))
        .map(|image| {
            image
                .max_w(rpx(323.))
                .h(rpx(90.))
                .object_fit(ObjectFit::Contain)
        });
    let heading = match logo {
        Some(logo) => div().flex().items_center().child(logo),
        None => div()
            .flex()
            .flex_col()
            .gap(rpx(12.))
            .max_w(rpx(560.))
            .child(
                div()
                    .text_size(rpx(32.))
                    .line_height(rpx(40.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(white())
                    .child(hero.title.clone()),
            )
            .when(!hero.summary.is_empty(), |this| {
                this.child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .text_color(rgba(0xFFFFFFCC))
                        .child(hero.summary.clone()),
                )
            }),
    };
    let action = match kind {
        PageKind::Orbs => button("shop-hero-quests", "Go to Quest Home", Tone::White)
            .on_click(on_app(app, |app, window, cx| app.open_quests(window, cx))),
        PageKind::Home => {
            let collection_id = hero.collection_id;
            button("shop-hero-collection", "Shop the Collection", Tone::White).on_click(on_app(
                app,
                move |app, _, cx| {
                    if let Some(id) = collection_id {
                        app.open_shop_collection(id, cx);
                    }
                },
            ))
        }
    };
    let skus = shown(shop, &hero.skus);
    let colors = collection
        .map(|c| c.styles.background.clone())
        .unwrap_or_default();
    let key = section_key(kind, usize::MAX >> 48);
    div()
        .relative()
        .w_full()
        .overflow_hidden()
        .bg(backdrop(&colors))
        .children(banner)
        .child(fade(theme::bg(), 240.))
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .w_full()
                .px(rpx(PAGE_PADDING_X))
                .pb(rpx(24.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(rpx(24.))
                        .w(rpx(width))
                        .h(rpx(HERO_TOP))
                        .child(heading)
                        .child(action),
                )
                .when(!skus.is_empty(), |this| {
                    this.child(
                        div()
                            .w(rpx(width))
                            .child(card::carousel(app, key, &skus, width, cx)),
                    )
                }),
        )
        .into_any_element()
}

pub(super) fn promos(app: &DiscordApp, promos: &[ShopPromo], width: f32, cx: &App) -> AnyElement {
    let count = promos.len().clamp(1, 2);
    let tile_width = (width - GRID_GAP * (count - 1) as f32) / count as f32;
    let tiles =
        promos.iter().take(2).enumerate().map(|(index, promo)| {
            let collection_id = promo.collection_id;
            let image = promo
                .art
                .as_deref()
                .and_then(|url| art(url, tile_width, cx))
                .map(|image| {
                    div().absolute().top_0().left_0().child(
                        image
                            .h(rpx(PROMO_HEIGHT))
                            .min_w(rpx(tile_width))
                            .object_fit(ObjectFit::Cover),
                    )
                });
            div()
                .id(("shop-promo", index))
                .relative()
                .flex()
                .flex_col()
                .justify_end()
                .w(rpx(tile_width))
                .h(rpx(PROMO_HEIGHT))
                .p(rpx(24.))
                .rounded(rpx(16.))
                .overflow_hidden()
                .cursor_pointer()
                .on_click(on_app(app, move |app, _, cx| {
                    app.open_shop_collection(collection_id, cx)
                }))
                .children(image)
                .child(div().relative().flex().child(
                    button(("shop-promo-go", index), "Take me there", Tone::White).on_click(
                        on_app(app, move |app, _, cx| {
                            app.open_shop_collection(collection_id, cx)
                        }),
                    ),
                ))
        });
    div()
        .flex()
        .gap(rpx(GRID_GAP))
        .w(rpx(width))
        .children(tiles)
        .into_any_element()
}

pub(super) fn section(app: &DiscordApp, section: &Section, width: f32, cx: &App) -> AnyElement {
    let collection = section.collection;
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rpx(16.))
        .mb(rpx(16.))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(section.title.clone()),
        )
        .children(section.button.clone().zip(collection).map(|(label, id)| {
            button(("shop-section-all", section.key), label, Tone::Subtle)
                .on_click(on_app(app, move |app, _, cx| {
                    app.open_shop_collection(id, cx)
                }))
        }));
    div()
        .flex()
        .flex_col()
        .w(rpx(width))
        .child(header)
        .child(card::carousel(app, section.key, &section.skus, width, cx))
        .into_any_element()
}

pub(super) fn info(info: &ShopInfo, width: f32, cx: &App) -> AnyElement {
    let banner = info
        .banner
        .as_deref()
        .and_then(|url| art(url, width, cx))
        .map(|image| {
            div()
                .absolute()
                .inset_0()
                .child(image.size_full().object_fit(ObjectFit::Cover))
        });
    div()
        .relative()
        .flex()
        .flex_col()
        .justify_center()
        .gap(rpx(8.))
        .w(rpx(width))
        .h(rpx(170.))
        .px(rpx(24.))
        .rounded(rpx(24.))
        .overflow_hidden()
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .bg(rgb(theme::surface_high()))
        .children(banner)
        .child(
            div()
                .relative()
                .max_w(rpx(width * 0.5))
                .text_size(rpx(24.))
                .line_height(rpx(30.))
                .font_weight(FontWeight::BOLD)
                .text_color(white())
                .child(info.title.clone()),
        )
        .child(
            div()
                .relative()
                .max_w(rpx(width * 0.45))
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .text_color(rgba(0xFFFFFFCC))
                .child(info.body.clone()),
        )
        .into_any_element()
}

impl DiscordApp {
    fn set_shop_grid_sort(&mut self, sort: GridSort, cx: &mut Context<Self>) {
        let grid = &mut self.home.shop.grid;
        grid.sort = sort;
        grid.shuffled.clear();
        grid.generation += 1;
        self.home.shop.epoch += 1;
        for kind in [PageKind::Home, PageKind::Orbs] {
            let wanted = wanted_skus(&self.home.shop, kind);
            self.hydrate_shop(wanted, cx);
        }
        cx.notify();
    }

    fn shuffle_shop_grid(&mut self, kind: PageKind, cx: &mut Context<Self>) {
        let Some(session) = self.shop_session() else {
            return;
        };
        let grid = &mut self.home.shop.grid;
        if grid.shuffling {
            return;
        }
        grid.shuffling = true;
        grid.generation += 1;
        let generation = grid.generation;
        let owner = self.home.shop.owner;
        let query = ShopQuery {
            orbs_only: kind == PageKind::Orbs,
            shuffle: true,
            limit: SHUFFLE_COUNT,
            ..ShopQuery::default()
        };
        let request = Tokio::spawn(cx, async move { session.search_shop(&query).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let shop = &mut app.home.shop;
                if shop.owner != owner {
                    return;
                }
                shop.grid.shuffling = false;
                if shop.grid.generation != generation {
                    return;
                }
                match result {
                    Ok(Ok(page)) => {
                        shop.grid.shuffled.insert(kind, page.skus.clone());
                        shop.epoch += 1;
                        app.hydrate_shop(page.skus, cx);
                    }
                    Ok(Err(error)) => log::warn!("Could not shuffle the Shop: {error}"),
                    Err(error) => log::warn!("Shop shuffle task failed: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

pub(super) fn grid_header(app: &DiscordApp, kind: PageKind) -> AnyElement {
    let shop = &app.home.shop;
    let sort = shop.grid.sort;
    let choice = |label: &'static str, value: GridSort| Choice {
        label,
        selected: sort == value,
        pick: Rc::new(move |app: &mut DiscordApp, cx: &mut Context<DiscordApp>| {
            app.set_shop_grid_sort(value, cx)
        }),
    };
    let label = match sort {
        GridSort::ForYou => "For You",
        GridSort::Popular => "Popular",
    };
    let title = match kind {
        PageKind::Home => "Find your style",
        PageKind::Orbs => "Orbs-Worthy Popular Picks",
    };
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rpx(16.))
        .w_full()
        .max_w(rpx(CONTENT_MAX_WIDTH))
        .mb(rpx(24.))
        .child(
            div()
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(title),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .child(
                    div()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::text()))
                        .child("Sort by"),
                )
                .child(select(
                    app,
                    "shop-grid-sort",
                    Menu::GridSort,
                    260.,
                    label,
                    vec![
                        choice("For You", GridSort::ForYou),
                        choice("Popular", GridSort::Popular),
                    ],
                ))
                .child(
                    button("shop-grid-shuffle", "Shuffle!", Tone::Subtle)
                        .when(shop.grid.shuffling, |this| this.opacity(0.6))
                        .on_click(on_app(app, move |app, _, cx| {
                            app.shuffle_shop_grid(kind, cx)
                        })),
                ),
        )
        .into_any_element()
}

pub(super) fn grid_footer(app: &DiscordApp) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(rpx(16.))
        .w_full()
        .pt(rpx(40.))
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("There's so much more"),
        )
        .child(
            button("shop-grid-all", "Shop All", Tone::Brand)
                .w(rpx(320.))
                .on_click(on_app(app, |app, _, cx| {
                    app.home.shop.browse.reset();
                    app.select_shop_tab(ShopTab::Browse, cx);
                })),
        )
        .into_any_element()
}

pub(super) fn collection_hero(
    app: &DiscordApp,
    collection_id: u64,
    width: f32,
    cx: &App,
) -> AnyElement {
    let shop = &app.home.shop;
    let Some(collection) = shop.collections.get(&collection_id) else {
        return div().into_any_element();
    };
    let banner = collection
        .hero_banner
        .as_deref()
        .or(collection.catalog_banner.as_deref())
        .and_then(|url| art(url, width + PAGE_PADDING_X * 2., cx))
        .map(|image| {
            div()
                .absolute()
                .inset_0()
                .child(image.size_full().object_fit(ObjectFit::Cover))
        });
    let logo = collection
        .hero_logo
        .as_deref()
        .or(collection.logo.as_deref())
        .and_then(|url| art(url, 323., cx))
        .map(|image| {
            image
                .max_w(rpx(323.))
                .h(rpx(90.))
                .object_fit(ObjectFit::Contain)
        });
    let crumb = div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(28.))
        .px(rpx(8.))
        .rounded(rpx(6.))
        .bg(rgba(0x000000A3))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .child(
            div()
                .id("shop-crumb-all")
                .cursor_pointer()
                .text_color(rgba(0xFFFFFFB3))
                .hover(|style| style.text_color(white()))
                .on_click(on_app(app, |app, _, cx| {
                    app.home.shop.browse.reset();
                    app.select_shop_tab(ShopTab::Browse, cx);
                }))
                .child("Shop All"),
        )
        .child(div().text_color(rgba(0xFFFFFF80)).child("/"))
        .child(div().text_color(white()).child(collection.name.clone()));
    div()
        .relative()
        .w_full()
        .h(rpx(COLLECTION_HERO))
        .overflow_hidden()
        .bg(backdrop(&collection.styles.background))
        .children(banner)
        .child(fade(theme::bg(), 120.))
        .child(div().absolute().top(rpx(16.)).left(rpx(16.)).child(crumb))
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .justify_end()
                .gap(rpx(12.))
                .size_full()
                .px(rpx(PAGE_PADDING_X))
                .pb(rpx(32.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(12.))
                        .w(rpx(width))
                        .mx_auto()
                        .child(match logo {
                            Some(logo) => logo.into_any_element(),
                            None => div()
                                .text_size(rpx(32.))
                                .line_height(rpx(40.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(white())
                                .child(collection.name.clone())
                                .into_any_element(),
                        })
                        .when(!collection.summary.is_empty(), |this| {
                            this.child(
                                div()
                                    .max_w(rpx(420.))
                                    .text_size(rpx(16.))
                                    .line_height(rpx(20.))
                                    .text_color(white())
                                    .child(collection.summary.clone()),
                            )
                        }),
                ),
        )
        .into_any_element()
}
