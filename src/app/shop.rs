mod art;
mod browse;
mod card;
mod controls;
mod detail;
mod featured;
mod header;
mod price;

pub(crate) use detail::overlay;
pub(crate) use header::header;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};
use gpui_component::input::{InputEvent, InputState};
use gpui_tokio::Tokio;

use crate::{
    app::{DiscordApp, guilds::channel_list, guilds::guild_rail},
    discord::model::{ShopCollection, ShopInfo, ShopKind, ShopPage, ShopProduct, ShopPromo},
    theme::{self, rpx},
};

const REFRESH_AFTER: Duration = Duration::from_secs(10 * 60);
const HYDRATE_BATCH: usize = 60;
const CONTENT_MAX_WIDTH: f32 = 1080.;
const PAGE_PADDING_X: f32 = 32.;
const GRID_GAP: f32 = 24.;
const CARD_MIN_WIDTH: f32 = 210.;
const CARD_HEIGHT: f32 = 246.;
const MAX_COLUMNS: usize = 4;
const LIST_OVERDRAW: f32 = 600.;
const MENU_REOPEN_GUARD: Duration = Duration::from_millis(150);
const BROWSE_HOVER_GRACE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub(crate) enum ShopTab {
    #[default]
    Featured,
    Browse,
    Orbs,
    Collection(u64),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum PageKind {
    Home,
    Orbs,
}

impl PageKind {
    fn tab(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Orbs => "orbs",
        }
    }
}

#[derive(Default)]
struct PageState {
    page: Option<Arc<ShopPage>>,
    loading: bool,
    failed: bool,
    loaded_at: Option<Instant>,
}

#[derive(Default)]
struct CollectionState {
    loading: bool,
    failed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Browse,
    GridSort,
    BrowseSort,
}

#[derive(Clone, Copy)]
enum BrowseHover {
    Tab,
    Menu,
}

#[derive(Clone)]
struct Section {
    title: String,
    collection: Option<u64>,
    button: Option<String>,
    skus: Vec<u64>,
    key: u32,
}

#[derive(Clone)]
enum Row {
    BrowseBar,
    Message(&'static str, &'static str),
    Loading(f32),
    Hero(PageKind),
    Promos(Vec<ShopPromo>),
    Section(Section),
    Info(ShopInfo),
    GridHeader(PageKind),
    Cards {
        section: u32,
        index: usize,
        skus: Vec<Option<u64>>,
    },
    GridFooter,
    CollectionHero(u64),
    CollectionBanner(u64),
    Pager {
        page: u32,
        pages: u32,
    },
    Gap(f32),
}

pub(crate) struct Shop {
    owner: u64,
    tab: ShopTab,
    epoch: u64,
    pages: HashMap<PageKind, PageState>,
    products: HashMap<u64, Arc<ShopProduct>>,
    collections: HashMap<u64, Arc<ShopCollection>>,
    collection_loads: HashMap<u64, CollectionState>,
    pending: HashSet<u64>,
    missing: HashSet<u64>,
    owned: Option<HashSet<u64>>,
    balance: Option<u64>,
    country: Option<Option<String>>,
    extras_loading: bool,
    browse: browse::Browse,
    grid: featured::Grid,
    detail: Option<detail::Detail>,
    detail_closing: Option<Task<()>>,
    detail_shown: u64,
    notice: Option<u64>,
    notice_closing: Option<Task<()>>,
    notice_shown: u64,
    hovered: Option<(u32, u64)>,
    entered: u64,
    left: Option<((u32, u64), u64)>,
    hover_changes: u64,
    card_clock: RefCell<Option<Instant>>,
    detail_clock: RefCell<Option<Instant>>,
    carousels: HashMap<u32, usize>,
    menu: Option<Menu>,
    menu_closed: Option<Instant>,
    browse_hover: [bool; 2],
    browse_hover_close: Option<Task<()>>,
    caret: Cell<(bool, u64)>,
    rows: ListState,
    rows_shape: Cell<Option<(ShopTab, u64, usize, Vec<u64>)>>,
    search: Option<Entity<InputState>>,
    _search_events: Option<Subscription>,
}

impl Default for Shop {
    fn default() -> Self {
        Self {
            owner: 0,
            tab: ShopTab::default(),
            epoch: 0,
            pages: HashMap::new(),
            products: HashMap::new(),
            collections: HashMap::new(),
            collection_loads: HashMap::new(),
            pending: HashSet::new(),
            missing: HashSet::new(),
            owned: None,
            balance: None,
            country: None,
            extras_loading: false,
            browse: browse::Browse::default(),
            grid: featured::Grid::default(),
            detail: None,
            detail_closing: None,
            detail_shown: 0,
            notice: None,
            notice_closing: None,
            notice_shown: 0,
            hovered: None,
            entered: 0,
            left: None,
            hover_changes: 0,
            card_clock: RefCell::new(None),
            detail_clock: RefCell::new(None),
            carousels: HashMap::new(),
            menu: None,
            menu_closed: None,
            browse_hover: [false; 2],
            browse_hover_close: None,
            caret: Cell::new((false, 0)),
            rows: ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW)),
            rows_shape: Cell::new(None),
            search: None,
            _search_events: None,
        }
    }
}

impl Shop {
    fn product(&self, sku_id: u64) -> Option<&Arc<ShopProduct>> {
        self.products.get(&sku_id)
    }

    fn page(&self, kind: PageKind) -> Option<&Arc<ShopPage>> {
        self.pages.get(&kind).and_then(|state| state.page.as_ref())
    }

    fn owns(&self, product: &ShopProduct) -> bool {
        let Some(owned) = &self.owned else {
            return false;
        };
        owned.contains(&product.sku_id)
            || (!product.bundled.is_empty()
                && product
                    .bundled
                    .iter()
                    .all(|bundled| owned.contains(&bundled.sku_id)))
    }

    fn hidden(&self, sku_id: u64) -> bool {
        self.missing.contains(&sku_id)
            || self
                .product(sku_id)
                .is_some_and(|product| product.kind == ShopKind::Other && self.owns(product))
    }

    fn remember_collection(&mut self, collection: ShopCollection) {
        for product in &collection.products {
            self.products
                .insert(product.sku_id, Arc::new(product.clone()));
        }
        self.collections.insert(collection.id, Arc::new(collection));
    }
}

fn nitro(app: &DiscordApp) -> bool {
    price::has_nitro_pricing(app.store.premium_type())
}

impl DiscordApp {
    pub(crate) fn open_shop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_home_page(super::direct::home::HomePage::Shop, window, cx);
        if self.home.shop.search.is_none() {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search the Shop"));
            self.home.shop._search_events =
                Some(cx.subscribe_in(&search, window, |app, _, event, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        app.submit_shop_search(cx);
                    }
                }));
            self.home.shop.search = Some(search);
        }
        self.load_shop(cx);
    }

    fn load_shop(&mut self, cx: &mut Context<Self>) {
        let owner = self.store.current_user_id;
        if self.home.shop.owner != owner {
            let search = self.home.shop.search.take();
            let events = self.home.shop._search_events.take();
            self.home.shop = Shop {
                owner,
                search,
                _search_events: events,
                ..Shop::default()
            };
        }
        self.load_shop_extras(cx);
        match self.home.shop.tab {
            ShopTab::Featured => self.load_shop_page(PageKind::Home, cx),
            ShopTab::Orbs => self.load_shop_page(PageKind::Orbs, cx),
            ShopTab::Browse => self.load_browse(cx),
            ShopTab::Collection(id) => self.load_shop_collection(id, cx),
        }
        let shop = &self.home.shop;
        let unresolved = match shop.tab {
            ShopTab::Featured => featured::wanted_skus(shop, PageKind::Home),
            ShopTab::Orbs => featured::wanted_skus(shop, PageKind::Orbs),
            ShopTab::Browse => shop.browse.result_skus(),
            ShopTab::Collection(id) => shop
                .collections
                .get(&id)
                .map(|collection| collection.product_skus.clone())
                .unwrap_or_default(),
        };
        self.hydrate_shop(unresolved, cx);
    }

    fn shop_session(&self) -> Option<crate::discord::Session> {
        self.session.clone().filter(|_| self.live)
    }

    fn shop_country(&self) -> Option<String> {
        self.home.shop.country.clone().flatten()
    }

    fn load_shop_extras(&mut self, cx: &mut Context<Self>) {
        let shop = &self.home.shop;
        if shop.extras_loading || (shop.country.is_some() && shop.owned.is_some()) {
            return;
        }
        let Some(session) = self.shop_session() else {
            return;
        };
        let owner = self.home.shop.owner;
        self.home.shop.extras_loading = true;
        let request = Tokio::spawn(cx, async move {
            tokio::join!(
                session.billing_country(),
                session.owned_shop_skus(),
                session.orb_balance()
            )
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let shop = &mut app.home.shop;
                if shop.owner != owner {
                    return;
                }
                shop.extras_loading = false;
                let Ok((country, owned, balance)) = result else {
                    return;
                };
                shop.country = Some(country.unwrap_or_else(|error| {
                    log::debug!("Could not load the billing country: {error}");
                    None
                }));
                match owned {
                    Ok(owned) => shop.owned = Some(owned.into_iter().collect()),
                    Err(error) => log::debug!("Could not load owned collectibles: {error}"),
                }
                match balance {
                    Ok(balance) => shop.balance = Some(balance),
                    Err(error) => log::debug!("Could not load the Orbs balance: {error}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_shop_page(&mut self, kind: PageKind, cx: &mut Context<Self>) {
        let Some(session) = self.shop_session() else {
            return;
        };
        let state = self.home.shop.pages.entry(kind).or_default();
        let fresh = state
            .loaded_at
            .is_some_and(|at| at.elapsed() < REFRESH_AFTER);
        if state.loading || fresh {
            return;
        }
        state.loading = true;
        let owner = self.home.shop.owner;
        let request = Tokio::spawn(cx, async move { session.shop_page(kind.tab()).await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let shop = &mut app.home.shop;
                if shop.owner != owner {
                    return;
                }
                let state = shop.pages.entry(kind).or_default();
                state.loading = false;
                match result {
                    Ok(Ok(page)) => {
                        state.failed = false;
                        state.loaded_at = Some(Instant::now());
                        let collections = page.collections.clone();
                        state.page = Some(Arc::new(page));
                        for collection in collections {
                            shop.remember_collection(collection);
                        }
                        shop.epoch += 1;
                        let wanted = featured::wanted_skus(shop, kind);
                        app.hydrate_shop(wanted, cx);
                    }
                    Ok(Err(error)) => {
                        log::warn!("Could not load the Shop: {error}");
                        state.failed = true;
                    }
                    Err(error) => {
                        log::warn!("Shop request task failed: {error}");
                        state.failed = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn hydrate_shop(&mut self, skus: Vec<u64>, cx: &mut Context<Self>) {
        let shop = &self.home.shop;
        let mut wanted: Vec<u64> = Vec::new();
        for sku in skus {
            if shop.products.contains_key(&sku)
                || shop.pending.contains(&sku)
                || shop.missing.contains(&sku)
                || wanted.contains(&sku)
            {
                continue;
            }
            wanted.push(sku);
        }
        if wanted.is_empty() {
            return;
        }
        let Some(session) = self.shop_session() else {
            return;
        };
        let owner = shop.owner;
        let country = self.shop_country();
        for batch in wanted.chunks(HYDRATE_BATCH) {
            let batch = batch.to_vec();
            self.home.shop.pending.extend(batch.iter().copied());
            let session = session.clone();
            let country = country.clone();
            let skus = batch.clone();
            let request = Tokio::spawn(cx, async move {
                session.shop_products(&skus, country.as_deref()).await
            });
            cx.spawn(async move |this, cx| {
                let result = request.await;
                this.update(cx, |app, cx| {
                    let shop = &mut app.home.shop;
                    if shop.owner != owner {
                        return;
                    }
                    for sku in &batch {
                        shop.pending.remove(sku);
                    }
                    match result {
                        Ok(Ok(products)) => {
                            for product in products {
                                shop.products.insert(product.sku_id, Arc::new(product));
                            }
                            let unknown: Vec<u64> = batch
                                .iter()
                                .filter(|sku| !shop.products.contains_key(sku))
                                .copied()
                                .collect();
                            shop.missing.extend(unknown);
                        }
                        Ok(Err(error)) => log::warn!("Could not load Shop items: {error}"),
                        Err(error) => log::warn!("Shop item task failed: {error}"),
                    }
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    fn load_shop_collection(&mut self, collection_id: u64, cx: &mut Context<Self>) {
        if self.home.shop.collections.contains_key(&collection_id) {
            return;
        }
        let Some(session) = self.shop_session() else {
            return;
        };
        let country = self.shop_country();
        let owner = self.home.shop.owner;
        let state = self
            .home
            .shop
            .collection_loads
            .entry(collection_id)
            .or_default();
        if state.loading {
            return;
        }
        state.loading = true;
        state.failed = false;
        let request = Tokio::spawn(cx, async move {
            session
                .shop_collection(collection_id, country.as_deref())
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let shop = &mut app.home.shop;
                if shop.owner != owner {
                    return;
                }
                let failed = match result {
                    Ok(Ok(Some(collection))) => {
                        shop.remember_collection(collection);
                        shop.epoch += 1;
                        false
                    }
                    Ok(Ok(None)) => true,
                    Ok(Err(error)) => {
                        log::warn!("Could not load a Shop collection: {error}");
                        true
                    }
                    Err(error) => {
                        log::warn!("Shop collection task failed: {error}");
                        true
                    }
                };
                let state = shop.collection_loads.entry(collection_id).or_default();
                state.loading = false;
                state.failed = failed;
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn select_shop_tab(&mut self, tab: ShopTab, cx: &mut Context<Self>) {
        let shop = &mut self.home.shop;
        shop.menu = None;
        shop.hovered = None;
        if shop.tab != tab {
            shop.tab = tab;
            shop.rows_shape.set(None);
        }
        self.load_shop(cx);
        cx.notify();
    }

    fn open_shop_collection(&mut self, collection_id: u64, cx: &mut Context<Self>) {
        self.close_shop_detail(cx);
        self.select_shop_tab(ShopTab::Collection(collection_id), cx);
    }

    fn hover_shop_card(&mut self, key: (u32, u64), hovered: bool, cx: &mut Context<Self>) {
        let shop = &mut self.home.shop;
        let next = if hovered {
            Some(key)
        } else if shop.hovered == Some(key) {
            None
        } else {
            return;
        };
        if shop.hovered != next {
            shop.hover_changes += 1;
            if let Some(previous) = shop.hovered {
                shop.left = Some((previous, shop.hover_changes));
            }
            if next.is_some() {
                shop.entered = shop.hover_changes;
            }
            shop.hovered = next;
            shop.card_clock.replace(None);
            cx.notify();
        }
    }

    fn scroll_shop_carousel(&mut self, section: u32, offset: usize, cx: &mut Context<Self>) {
        self.home.shop.carousels.insert(section, offset);
        cx.notify();
    }

    fn toggle_shop_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        let shop = &mut self.home.shop;
        let guarded = shop
            .menu_closed
            .is_some_and(|at| at.elapsed() < MENU_REOPEN_GUARD);
        shop.menu = if shop.menu == Some(menu) || guarded {
            None
        } else {
            Some(menu)
        };
        cx.notify();
    }

    fn hover_shop_browse(&mut self, part: BrowseHover, hovered: bool, cx: &mut Context<Self>) {
        let shop = &mut self.home.shop;
        let open = shop.menu == Some(Menu::Browse);
        if !open {
            shop.browse_hover = [false; 2];
        }
        shop.browse_hover[part as usize] = hovered;
        if hovered {
            shop.browse_hover_close = None;
            if !open {
                shop.menu = Some(Menu::Browse);
                cx.notify();
            }
            return;
        }
        if !open || shop.browse_hover.contains(&true) {
            return;
        }
        shop.browse_hover_close = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BROWSE_HOVER_GRACE).await;
            this.update(cx, |app, cx| {
                let shop = &mut app.home.shop;
                shop.browse_hover_close = None;
                if shop.menu == Some(Menu::Browse) && !shop.browse_hover.contains(&true) {
                    app.close_shop_menu(cx);
                }
            })
            .ok();
        }));
    }

    fn close_shop_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.home.shop.menu.take().is_none() {
            return false;
        }
        self.home.shop.menu_closed = Some(Instant::now());
        cx.notify();
        true
    }

    pub(crate) fn close_shop_layers(&mut self, cx: &mut Context<Self>) -> bool {
        self.close_shop_notice(cx) || self.close_shop_detail(cx) || self.close_shop_menu(cx)
    }

    fn submit_shop_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = &self.home.shop.search else {
            return;
        };
        let text = search.read(cx).value().trim().to_owned();
        self.home.shop.browse.search_for(text);
        self.home.shop.tab = ShopTab::Browse;
        self.home.shop.rows_shape.set(None);
        self.load_browse(cx);
        cx.notify();
    }
}

fn column_width(window: &Window) -> f32 {
    let unit = rpx(1.).to_pixels(window.rem_size());
    window.viewport_size().width / unit - guild_rail::WIDTH - channel_list::WIDTH
}

fn content_width(shop: &Shop, window: &Window) -> f32 {
    let sidebar = if shop.tab == ShopTab::Browse && shop.browse.filters_shown {
        browse::SIDEBAR_WIDTH
    } else {
        0.
    };
    (column_width(window) - sidebar - PAGE_PADDING_X * 2.).clamp(CARD_MIN_WIDTH, CONTENT_MAX_WIDTH)
}

fn columns(width: f32) -> (usize, f32) {
    let columns =
        (((width + GRID_GAP) / (CARD_MIN_WIDTH + GRID_GAP)).floor() as usize).clamp(1, MAX_COLUMNS);
    let card = (width - GRID_GAP * (columns - 1) as f32) / columns as f32;
    (columns, card.max(CARD_MIN_WIDTH))
}

fn rows_of(section: u32, skus: impl IntoIterator<Item = Option<u64>>, columns: usize) -> Vec<Row> {
    let skus: Vec<Option<u64>> = skus.into_iter().collect();
    skus.chunks(columns.max(1))
        .enumerate()
        .map(|(index, chunk)| Row::Cards {
            section,
            index,
            skus: chunk.to_vec(),
        })
        .collect()
}

fn row_key(shop: &Shop, row: &Row) -> u64 {
    let mut hasher = DefaultHasher::new();
    std::mem::discriminant(row).hash(&mut hasher);
    match row {
        Row::BrowseBar | Row::GridFooter => {}
        Row::Message(title, _) => title.hash(&mut hasher),
        Row::Loading(height) | Row::Gap(height) => height.to_bits().hash(&mut hasher),
        Row::Hero(kind) => (kind, featured::hero_has_cards(shop, *kind)).hash(&mut hasher),
        Row::Promos(promos) => {
            for promo in promos {
                promo.collection_id.hash(&mut hasher);
            }
        }
        Row::Section(section) => section.key.hash(&mut hasher),
        Row::Info(info) => info.title.hash(&mut hasher),
        Row::GridHeader(kind) => kind.hash(&mut hasher),
        Row::Cards { section, index, .. } => (section, index).hash(&mut hasher),
        Row::CollectionHero(id) | Row::CollectionBanner(id) => id.hash(&mut hasher),
        Row::Pager { page, pages } => (page, pages).hash(&mut hasher),
    }
    hasher.finish()
}

fn render_row(app: &DiscordApp, row: &Row, window: &Window, cx: &App) -> AnyElement {
    let shop = &app.home.shop;
    let width = content_width(shop, window);
    let body = match row {
        Row::BrowseBar => browse::bar(app, width),
        Row::Message(title, detail) => featured::message(title, detail),
        Row::Loading(height) => featured::loading(*height, cx),
        Row::Hero(kind) => featured::hero(app, *kind, width, window, cx),
        Row::Promos(promos) => featured::promos(app, promos, width, cx),
        Row::Section(section) => featured::section(app, section, width, cx),
        Row::Info(info) => featured::info(info, width, cx),
        Row::GridHeader(kind) => featured::grid_header(app, *kind),
        Row::Cards {
            section,
            index,
            skus,
        } => card::row(app, *section, *index, skus, width, cx),
        Row::GridFooter => featured::grid_footer(app),
        Row::CollectionHero(id) => featured::collection_hero(app, *id, width, cx),
        Row::CollectionBanner(id) => browse::collection_banner(app, *id, width, cx),
        Row::Pager { page, pages } => browse::pager(app, *page, *pages),
        Row::Gap(height) => div().h(rpx(*height)).into_any_element(),
    };
    let full_bleed = matches!(row, Row::Hero(_) | Row::CollectionHero(_));
    div()
        .w_full()
        .flex()
        .justify_center()
        .when(!full_bleed, |this| this.px(rpx(PAGE_PADDING_X)))
        .child(body)
        .into_any_element()
}

pub(crate) fn page(app: &DiscordApp, window: &Window) -> AnyElement {
    let shop = &app.home.shop;
    let rows: Vec<Row> = match shop.tab {
        ShopTab::Featured => featured::rows(shop, PageKind::Home, window),
        ShopTab::Orbs => featured::rows(shop, PageKind::Orbs, window),
        ShopTab::Browse => browse::rows(shop, window),
        ShopTab::Collection(id) => featured::collection_rows(shop, id, window),
    };
    let width = column_width(window).round() as usize;
    let keys: Vec<u64> = rows.iter().map(|row| row_key(shop, row)).collect();
    let previous = shop
        .rows_shape
        .replace(Some((shop.tab, shop.epoch, width, keys.clone())));
    match previous {
        Some((tab, epoch, previous_width, old))
            if (tab, epoch, previous_width) == (shop.tab, shop.epoch, width) =>
        {
            let prefix = old
                .iter()
                .zip(&keys)
                .take_while(|(old, new)| old == new)
                .count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(keys[prefix..].iter().rev())
                .take_while(|(old, new)| old == new)
                .count();
            if prefix + suffix < old.len().max(keys.len()) {
                shop.rows
                    .splice(prefix..old.len() - suffix, keys.len() - prefix - suffix);
            }
        }
        _ => shop.rows.reset(rows.len()),
    }
    let rows = Rc::new(rows);
    let entity = app.entity.clone();
    let body = list(shop.rows.clone(), move |index, window, cx| {
        let Some(row) = rows.get(index) else {
            return div().into_any_element();
        };
        let Some(entity) = entity.upgrade() else {
            return div().into_any_element();
        };
        render_row(entity.read(cx), row, window, cx)
    })
    .size_full();
    div()
        .id("shop-page")
        .flex()
        .size_full()
        .bg(rgb(theme::bg()))
        .child(div().flex_1().min_w_0().h_full().child(body))
        .when(
            shop.tab == ShopTab::Browse && shop.browse.filters_shown,
            |this| this.child(browse::sidebar(app)),
        )
        .into_any_element()
}
