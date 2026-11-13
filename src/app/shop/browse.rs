use gpui_component::scroll::ScrollableElement as _;

use super::controls::{Choice, Tone, art, button, on_app, select};
use super::*;
use crate::{
    app::menu,
    discord::model::{ShopQuery, ShopSort},
};

pub(super) const SIDEBAR_WIDTH: f32 = 240.;
const COLLECTIONS_PER_PAGE: u32 = 5;
const RESULTS_PER_PAGE: u32 = 20;
const BANNER_HEIGHT: f32 = 166.;

pub(super) const TYPES: [(ShopKind, &str); 5] = [
    (ShopKind::Decoration, "Avatar Decorations"),
    (ShopKind::Nameplate, "Nameplates"),
    (ShopKind::Effect, "Profile Effects"),
    (ShopKind::Frame, "Profile Frames"),
    (ShopKind::Bundle, "Bundles"),
];

const COLORS: [(&str, &str, u32); 10] = [
    ("COLLECTIBLES_COLOR_PURPLE", "Purple", 0x8E5BD6),
    ("COLLECTIBLES_COLOR_BLUE", "Blue", 0x2F86D6),
    ("COLLECTIBLES_COLOR_GREEN", "Green", 0x2DA453),
    ("COLLECTIBLES_COLOR_BROWN", "Brown", 0x8A4B2C),
    ("COLLECTIBLES_COLOR_YELLOW", "Yellow", 0xE3B505),
    ("COLLECTIBLES_COLOR_ORANGE", "Orange", 0xE37A1E),
    ("COLLECTIBLES_COLOR_RED", "Red", 0xD43B33),
    ("COLLECTIBLES_COLOR_PINK", "Pink", 0xD8487E),
    ("COLLECTIBLES_COLOR_WHITE", "White", 0xF2F2F2),
    ("COLLECTIBLES_COLOR_BLACK", "Black", 0x26272B),
];

const THEMES: [(&str, &str); 10] = [
    ("COLLECTIBLES_THEME_ANIME", "Anime"),
    ("COLLECTIBLES_THEME_GAMING", "Gaming"),
    ("COLLECTIBLES_THEME_CUTE_COZY", "Cute & Cozy"),
    ("COLLECTIBLES_THEME_SCI_FI", "Sci-Fi"),
    ("COLLECTIBLES_THEME_FOOD_DRINKS", "Food & Drinks"),
    ("COLLECTIBLES_THEME_FANTASY", "Fantasy"),
    ("COLLECTIBLES_THEME_ANIMALS_PETS", "Animals & Pets"),
    ("COLLECTIBLES_THEME_NATURE", "Nature"),
    ("COLLECTIBLES_THEME_MOVIES_TV_SHOWS", "Movies & TV"),
    ("COLLECTIBLES_THEME_DARK_MOODY", "Dark & Moody"),
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Sort {
    #[default]
    Recent,
    Oldest,
    Popular,
    PriceLow,
    PriceHigh,
    Relevance,
}

impl Sort {
    const ALL: [(Self, &'static str); 6] = [
        (Self::Relevance, "Relevance"),
        (Self::Recent, "Recently Added"),
        (Self::Oldest, "Oldest Added"),
        (Self::Popular, "Popular"),
        (Self::PriceLow, "Price: Low to High"),
        (Self::PriceHigh, "Price: High to Low"),
    ];

    fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(sort, _)| *sort == self)
            .map_or("Recently Added", |(_, label)| label)
    }

    fn query(self) -> ShopSort {
        match self {
            Self::Recent => ShopSort::Recent,
            Self::Oldest => ShopSort::Oldest,
            Self::Popular => ShopSort::Popular,
            Self::PriceLow => ShopSort::PriceLow,
            Self::PriceHigh => ShopSort::PriceHigh,
            Self::Relevance => ShopSort::Relevance,
        }
    }
}

enum Results {
    Collections { ids: Vec<u64>, total: u32 },
    Search { skus: Vec<u64>, total: u32 },
}

pub(super) struct Browse {
    kinds: Vec<ShopKind>,
    colors: Vec<&'static str>,
    themes: Vec<&'static str>,
    orbs_only: bool,
    collabs_only: bool,
    search: String,
    sort: Sort,
    page: u32,
    pub(super) filters_shown: bool,
    results: Option<Results>,
    failed: bool,
    generation: u64,
    loading: Option<u64>,
}

impl Default for Browse {
    fn default() -> Self {
        Self {
            kinds: Vec::new(),
            colors: Vec::new(),
            themes: Vec::new(),
            orbs_only: false,
            collabs_only: false,
            search: String::new(),
            sort: Sort::default(),
            page: 0,
            filters_shown: true,
            results: None,
            failed: false,
            generation: 0,
            loading: None,
        }
    }
}

impl Browse {
    fn filtered(&self) -> bool {
        !self.kinds.is_empty()
            || !self.colors.is_empty()
            || !self.themes.is_empty()
            || self.orbs_only
            || self.collabs_only
    }

    fn by_collection(&self) -> bool {
        !self.filtered() && self.search.is_empty() && self.sort == Sort::Recent
    }

    fn changed(&mut self) {
        self.generation += 1;
        self.page = 0;
        self.results = None;
        self.failed = false;
    }

    fn clear_filters(&mut self) {
        self.kinds.clear();
        self.colors.clear();
        self.themes.clear();
        self.orbs_only = false;
        self.collabs_only = false;
        self.changed();
    }

    pub(super) fn reset(&mut self) {
        self.clear_filters();
        self.search_for(String::new());
    }

    pub(super) fn only(&mut self, kind: Option<ShopKind>, collabs: bool) {
        self.reset();
        self.kinds.extend(kind);
        self.collabs_only = collabs;
    }

    pub(super) fn search_for(&mut self, text: String) {
        if self.search != text {
            if !text.is_empty() {
                self.sort = Sort::Relevance;
            } else if self.sort == Sort::Relevance {
                self.sort = Sort::default();
            }
            self.search = text;
            self.changed();
        }
    }

    fn query(&self) -> ShopQuery {
        ShopQuery {
            item_types: self.kinds.clone(),
            colors: self.colors.clone(),
            themes: self.themes.clone(),
            orbs_only: self.orbs_only,
            collabs_only: self.collabs_only,
            search: self.search.clone(),
            sort: self.sort.query(),
            shuffle: false,
            offset: self.page * RESULTS_PER_PAGE,
            limit: RESULTS_PER_PAGE,
        }
    }

    pub(super) fn result_skus(&self) -> Vec<u64> {
        match &self.results {
            Some(Results::Search { skus, .. }) => skus.clone(),
            _ => Vec::new(),
        }
    }

    fn pages(&self) -> u32 {
        match &self.results {
            Some(Results::Collections { total, .. }) => total.div_ceil(COLLECTIONS_PER_PAGE),
            Some(Results::Search { total, .. }) => total.div_ceil(RESULTS_PER_PAGE),
            None => 0,
        }
    }
}

fn toggle<T: PartialEq>(values: &mut Vec<T>, value: T) {
    match values.iter().position(|existing| *existing == value) {
        Some(index) => {
            values.remove(index);
        }
        None => values.push(value),
    }
}

impl DiscordApp {
    pub(super) fn load_browse(&mut self, cx: &mut Context<Self>) {
        let browse = &self.home.shop.browse;
        let generation = browse.generation;
        if browse.loading == Some(generation) || browse.results.is_some() {
            return;
        }
        let Some(session) = self.shop_session() else {
            return;
        };
        let owner = self.home.shop.owner;
        let country = self.shop_country();
        let by_collection = browse.by_collection();
        let page = browse.page;
        let query = browse.query();
        self.home.shop.browse.loading = Some(generation);
        if by_collection {
            let request = Tokio::spawn(cx, async move {
                session
                    .shop_collections(
                        page * COLLECTIONS_PER_PAGE,
                        COLLECTIONS_PER_PAGE,
                        country.as_deref(),
                    )
                    .await
            });
            cx.spawn(async move |this, cx| {
                let result = request.await;
                this.update(cx, |app, cx| {
                    let shop = &mut app.home.shop;
                    if shop.owner != owner || shop.browse.generation != generation {
                        return;
                    }
                    shop.browse.loading = None;
                    match result {
                        Ok(Ok(page)) => {
                            let ids = page.collections.iter().map(|c| c.id).collect();
                            for collection in page.collections {
                                shop.remember_collection(collection);
                            }
                            shop.browse.results = Some(Results::Collections {
                                ids,
                                total: page.total,
                            });
                            shop.epoch += 1;
                        }
                        Ok(Err(error)) => {
                            log::warn!("Could not load Shop collections: {error}");
                            shop.browse.failed = true;
                        }
                        Err(error) => {
                            log::warn!("Shop collections task failed: {error}");
                            shop.browse.failed = true;
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        } else {
            let request = Tokio::spawn(cx, async move { session.search_shop(&query).await });
            cx.spawn(async move |this, cx| {
                let result = request.await;
                this.update(cx, |app, cx| {
                    let shop = &mut app.home.shop;
                    if shop.owner != owner || shop.browse.generation != generation {
                        return;
                    }
                    shop.browse.loading = None;
                    match result {
                        Ok(Ok(page)) => {
                            let skus = page.skus.clone();
                            shop.browse.results = Some(Results::Search {
                                skus: page.skus,
                                total: page.pagination.total,
                            });
                            shop.epoch += 1;
                            app.hydrate_shop(skus, cx);
                        }
                        Ok(Err(error)) => {
                            log::warn!("Could not search the Shop: {error}");
                            shop.browse.failed = true;
                        }
                        Err(error) => {
                            log::warn!("Shop search task failed: {error}");
                            shop.browse.failed = true;
                        }
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    fn update_browse(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Browse)) {
        change(&mut self.home.shop.browse);
        self.home.shop.browse.changed();
        self.load_browse(cx);
        cx.notify();
    }

    fn set_browse_page(&mut self, page: u32, cx: &mut Context<Self>) {
        let browse = &mut self.home.shop.browse;
        if browse.page == page {
            return;
        }
        browse.generation += 1;
        browse.page = page;
        browse.results = None;
        browse.failed = false;
        self.load_browse(cx);
        cx.notify();
    }

    pub(super) fn browse_only(
        &mut self,
        kind: Option<ShopKind>,
        collabs: bool,
        cx: &mut Context<Self>,
    ) {
        self.home.shop.browse.only(kind, collabs);
        self.select_shop_tab(ShopTab::Browse, cx);
    }
}

pub(super) fn rows(shop: &Shop, window: &Window) -> Vec<Row> {
    let browse = &shop.browse;
    let (columns, _) = columns(content_width(shop, window));
    let mut rows = vec![Row::BrowseBar];
    match &browse.results {
        None if browse.failed => rows.push(Row::Message(
            "The Shop didn't load",
            "Something went wrong getting the Shop. Try again later.",
        )),
        None => {
            rows.push(Row::Loading(BANNER_HEIGHT));
            rows.push(Row::Gap(24.));
            rows.push(Row::Loading(CARD_HEIGHT));
        }
        Some(Results::Collections { ids, .. }) => {
            for id in ids {
                let Some(collection) = shop.collections.get(id) else {
                    continue;
                };
                rows.push(Row::CollectionBanner(*id));
                rows.push(Row::Gap(24.));
                let skus = collection
                    .product_skus
                    .iter()
                    .copied()
                    .filter(|sku| !shop.hidden(*sku))
                    .map(Some);
                rows.extend(rows_of(*id as u32, skus, columns));
                rows.push(Row::Gap(24.));
            }
        }
        Some(Results::Search { skus, .. }) if skus.is_empty() => rows.push(Row::Message(
            "No items match",
            "Try removing a filter or searching for something else.",
        )),
        Some(Results::Search { skus, .. }) => {
            let skus = skus
                .iter()
                .copied()
                .filter(|sku| !shop.hidden(*sku))
                .map(|sku| shop.product(sku).map(|_| sku));
            rows.extend(rows_of(u32::MAX - 1, skus, columns));
        }
    }
    let pages = browse.pages();
    if pages > 1 {
        rows.push(Row::Pager {
            page: browse.page + 1,
            pages,
        });
    }
    rows.push(Row::Gap(48.));
    rows
}

pub(super) fn bar(app: &DiscordApp, width: f32) -> AnyElement {
    let browse = &app.home.shop.browse;
    let searching = !browse.search.is_empty();
    let choices = Sort::ALL
        .iter()
        .filter(|(sort, _)| searching || *sort != Sort::Relevance)
        .map(|&(sort, label)| Choice {
            label,
            selected: browse.sort == sort,
            pick: Rc::new(move |app: &mut DiscordApp, cx: &mut Context<DiscordApp>| {
                app.update_browse(cx, |browse| browse.sort = sort)
            }),
        })
        .collect();
    let heading = (!browse.search.is_empty()).then(|| {
        div()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .min_w_0()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(rpx(20.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .child(format!("Results for \u{201c}{}\u{201d}", browse.search)),
            )
            .child(
                div()
                    .id("shop-search-clear")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(24.))
                    .rounded_full()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgba(theme::hover())))
                    .on_click(on_app(app, |app, _, cx| {
                        app.update_browse(cx, |browse| browse.search_for(String::new()))
                    }))
                    .child(theme::icon("x", 16., theme::muted())),
            )
    });
    let shown = browse.filters_shown;
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rpx(16.))
        .w(rpx(width))
        .pt(rpx(16.))
        .pb(rpx(24.))
        .child(div().flex_1().min_w_0().children(heading))
        .child(
            div()
                .flex()
                .flex_none()
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
                    "shop-browse-sort",
                    Menu::BrowseSort,
                    190.,
                    browse.sort.label(),
                    choices,
                ))
                .child(
                    button(
                        "shop-filters-toggle",
                        if shown {
                            "Hide Filters"
                        } else {
                            "Show Filters"
                        },
                        Tone::Subtle,
                    )
                    .child(theme::icon("sliders-horizontal", 16., theme::strong()))
                    .on_click(on_app(app, |app, _, cx| {
                        let browse = &mut app.home.shop.browse;
                        browse.filters_shown = !browse.filters_shown;
                        app.home.shop.rows_shape.set(None);
                        cx.notify();
                    })),
                ),
        )
        .into_any_element()
}

pub(super) fn collection_banner(
    app: &DiscordApp,
    collection_id: u64,
    width: f32,
    cx: &App,
) -> AnyElement {
    let Some(collection) = app.home.shop.collections.get(&collection_id) else {
        return div().into_any_element();
    };
    let image = collection
        .catalog_banner
        .as_deref()
        .or(collection.hero_banner.as_deref())
        .and_then(|url| art(url, width, cx))
        .map(|image| {
            div()
                .absolute()
                .inset_0()
                .child(image.size_full().object_fit(ObjectFit::Cover))
        });
    div()
        .id(("shop-collection-banner", collection_id as usize))
        .relative()
        .flex()
        .items_end()
        .w(rpx(width))
        .h(rpx(BANNER_HEIGHT))
        .p(rpx(16.))
        .rounded(rpx(16.))
        .overflow_hidden()
        .bg(rgb(theme::surface_high()))
        .cursor_pointer()
        .on_click(on_app(app, move |app, _, cx| {
            app.open_shop_collection(collection_id, cx)
        }))
        .when(image.is_none(), |this| {
            this.child(
                div()
                    .relative()
                    .text_size(rpx(24.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(theme::strong()))
                    .child(collection.name.clone()),
            )
        })
        .children(image)
        .into_any_element()
}

fn pager_slots(page: u32, pages: u32) -> Vec<Option<u32>> {
    if pages <= 9 {
        return (1..=pages).map(Some).collect();
    }
    let mut slots = vec![Some(1)];
    let (start, end) = if page <= 5 {
        (2, 7)
    } else if page + 4 >= pages {
        (pages - 6, pages - 1)
    } else {
        (page - 2, page + 2)
    };
    if start > 2 {
        slots.push(None);
    }
    slots.extend((start..=end).map(Some));
    if end < pages - 1 {
        slots.push(None);
    }
    slots.push(Some(pages));
    slots
}

pub(super) fn pager(app: &DiscordApp, page: u32, pages: u32) -> AnyElement {
    let step = |id: &'static str, label: &'static str, target: u32, enabled: bool, back: bool| {
        let color = if enabled {
            theme::strong()
        } else {
            theme::faint()
        };
        let chevron = theme::icon(
            if back {
                "chevron-left"
            } else {
                "chevron-right"
            },
            16.,
            color,
        );
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(rpx(4.))
            .h(rpx(36.))
            .px(rpx(10.))
            .rounded(rpx(8.))
            .text_size(rpx(16.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(color))
            .map(|this| {
                if back {
                    this.child(chevron).child(label)
                } else {
                    this.child(label).child(chevron)
                }
            })
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(rgba(theme::hover())))
                    .on_click(on_app(app, move |app, _, cx| {
                        app.set_browse_page(target.saturating_sub(1), cx)
                    }))
            })
    };
    let numbers = pager_slots(page, pages)
        .into_iter()
        .enumerate()
        .map(|(slot, number)| {
            let Some(number) = number else {
                return div()
                    .w(rpx(28.))
                    .text_center()
                    .text_color(rgb(theme::muted()))
                    .child("\u{2026}")
                    .into_any_element();
            };
            let current = number == page;
            div()
                .id(("shop-page", slot))
                .flex()
                .items_center()
                .justify_center()
                .h(rpx(28.))
                .min_w(rpx(28.))
                .px(rpx(6.))
                .rounded_full()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .map(|this| {
                    if current {
                        this.bg(rgb(theme::BLURPLE)).text_color(white())
                    } else {
                        this.text_color(rgb(theme::strong()))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgba(theme::hover())))
                            .on_click(on_app(app, move |app, _, cx| {
                                app.set_browse_page(number - 1, cx)
                            }))
                    }
                })
                .child(number.to_string())
                .into_any_element()
        });
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(rpx(16.))
        .w_full()
        .pt(rpx(24.))
        .border_t_1()
        .border_color(rgba(theme::border_subtle()))
        .child(step(
            "shop-page-back",
            "Previous",
            page.saturating_sub(1),
            page > 1,
            true,
        ))
        .child(div().flex().items_center().gap(rpx(4.)).children(numbers))
        .child(step(
            "shop-page-next",
            "Next",
            page + 1,
            page < pages,
            false,
        ))
        .into_any_element()
}

fn heading(text: &'static str) -> Div {
    div()
        .mt(rpx(16.))
        .mb(rpx(8.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()))
        .child(text)
}

fn check_row(
    app: &DiscordApp,
    id: impl Into<ElementId>,
    label: &'static str,
    checked: bool,
    change: impl Fn(&mut Browse) + 'static,
) -> Stateful<Div> {
    let change = Rc::new(change);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(rpx(10.))
        .h(rpx(32.))
        .cursor_pointer()
        .text_size(rpx(16.))
        .text_color(rgb(theme::text()))
        .on_click(on_app(app, move |app, _, cx| {
            let change = change.clone();
            app.update_browse(cx, move |browse| change(browse))
        }))
        .child(menu::checkbox(checked))
        .child(label)
}

pub(super) fn sidebar(app: &DiscordApp) -> AnyElement {
    let browse = &app.home.shop.browse;
    let types = TYPES.iter().enumerate().map(|(index, (kind, label))| {
        let kind = *kind;
        check_row(
            app,
            ("shop-filter-type", index),
            label,
            browse.kinds.contains(&kind),
            move |browse| toggle(&mut browse.kinds, kind),
        )
    });
    let colors = COLORS
        .iter()
        .enumerate()
        .map(|(index, (value, label, color))| {
            let value = *value;
            let selected = browse.colors.contains(&value);
            div()
                .id(("shop-filter-color", index))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(32.))
                .rounded_full()
                .cursor_pointer()
                .border_2()
                .border_color(if selected {
                    rgb(theme::strong())
                } else {
                    rgba(0)
                })
                .on_click(on_app(app, move |app, _, cx| {
                    app.update_browse(cx, |browse| toggle(&mut browse.colors, value))
                }))
                .child(div().size(rpx(26.)).rounded_full().bg(rgb(*color)))
                .child(theme::hover_tooltip(
                    SharedString::from(format!("shop-filter-color-tip-{index}")),
                    *label,
                ))
        });
    let themes = THEMES.iter().enumerate().map(|(index, (value, label))| {
        let value = *value;
        let selected = browse.themes.contains(&value);
        div()
            .id(("shop-filter-theme", index))
            .flex()
            .items_center()
            .h(rpx(32.))
            .px(rpx(10.))
            .rounded(rpx(8.))
            .cursor_pointer()
            .border_1()
            .border_color(rgba(controls::BORDER_SUBTLE))
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .map(|this| {
                if selected {
                    this.bg(rgb(theme::BLURPLE)).text_color(white())
                } else {
                    this.bg(rgba(controls::MOD_SUBTLE))
                        .text_color(rgb(theme::strong()))
                        .hover(|style| style.bg(rgba(theme::selected())))
                }
            })
            .on_click(on_app(app, move |app, _, cx| {
                app.update_browse(cx, |browse| toggle(&mut browse.themes, value))
            }))
            .child(*label)
    });
    div()
        .id("shop-filters")
        .flex()
        .flex_col()
        .flex_none()
        .w(rpx(SIDEBAR_WIDTH))
        .h_full()
        .px(rpx(16.))
        .pb(rpx(24.))
        .border_l_1()
        .border_color(rgba(theme::border_subtle()))
        .overflow_y_scrollbar()
        .child(heading("Item Type"))
        .children(types)
        .child(heading("Show Only"))
        .child(check_row(
            app,
            "shop-filter-orbs",
            "Orbs Eligible",
            browse.orbs_only,
            |browse| browse.orbs_only = !browse.orbs_only,
        ))
        .child(check_row(
            app,
            "shop-filter-collabs",
            "Collabs",
            browse.collabs_only,
            |browse| browse.collabs_only = !browse.collabs_only,
        ))
        .child(heading("Color"))
        .child(div().flex().flex_wrap().gap(rpx(8.)).children(colors))
        .child(heading("Theme"))
        .child(div().flex().flex_wrap().gap(rpx(8.)).children(themes))
        .when(browse.filtered(), |this| {
            this.child(
                button("shop-filters-clear", "Clear all filters", Tone::Subtle)
                    .mt(rpx(24.))
                    .on_click(on_app(app, |app, _, cx| {
                        app.update_browse(cx, Browse::clear_filters)
                    })),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::pager_slots;

    #[test]
    fn pager_keeps_the_ends_and_a_window_around_the_page() {
        assert_eq!(pager_slots(1, 3), [Some(1), Some(2), Some(3)]);
        let near_start = pager_slots(1, 20);
        assert_eq!(
            near_start,
            [
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                None,
                Some(20)
            ]
        );
        assert_eq!(
            pager_slots(10, 20),
            [
                Some(1),
                None,
                Some(8),
                Some(9),
                Some(10),
                Some(11),
                Some(12),
                None,
                Some(20)
            ]
        );
        assert_eq!(
            pager_slots(19, 20),
            [
                Some(1),
                None,
                Some(14),
                Some(15),
                Some(16),
                Some(17),
                Some(18),
                Some(19),
                Some(20)
            ]
        );
    }
}
