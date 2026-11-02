use gpui_component::scroll::ScrollableElement as _;
use std::{
    cell::Cell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_tokio::Tokio;

use crate::{
    app::{DiscordApp, emoji_picker},
    discord::{
        gifs::{Gif, GifCategories, GifCategory, GifProvider},
        settings_proto::{self, FavoriteGif},
    },
    media::animated,
    media::images::ImageSpec,
    settings,
    theme::{self, rpx},
};

const WIDTH: f32 = emoji_picker::PANEL_WIDTH;
const HEIGHT: f32 = emoji_picker::HEIGHT;
const PADDING: f32 = 14.;
const GAP: f32 = 14.;
const COLUMN: f32 = (WIDTH - 2. - 2. * PADDING - GAP) / 2.;
const TILE: f32 = 124.;
/// Children need their own radius: `overflow_hidden` clips to the tile's rectangle, not its corners.
const RADIUS: f32 = 4.;
const TAB: f32 = 32.;
const FIELD: f32 = 42.;
const HEADER_PADDING: f32 = 12.;
const HEADER_GAP: f32 = 12.;
const MIN_HEIGHT: f32 = 240.;
const SEARCH_DELAY: Duration = Duration::from_millis(300);
/// Tiles this far outside the viewport still render, so a fast scroll doesn't flash empty cells.
const OVERSCAN: f32 = 240.;
const PICKER_RISE: f32 = 8.;
const PICKER_ENTER: Duration = Duration::from_millis(150);
const PICKER_EXIT: Duration = Duration::from_millis(100);
/// A press that closed the picker via mouse-down-outside must not reopen it from
/// the toggle button handling the same press.
const REOPEN_GUARD: Duration = Duration::from_millis(150);
/// `FavoriteGIF.format` in FrecencyUserSettings: 1 = IMAGE, 2 = VIDEO.
const FORMAT_IMAGE: u32 = 1;
const FORMAT_VIDEO: u32 = 2;

#[derive(Clone)]
enum Load<T> {
    Loading,
    Ready(T),
    Failed,
}

#[derive(Clone, PartialEq)]
enum View {
    Home,
    Trending,
    Favorites,
    Search(String),
}

#[derive(Default)]
pub(crate) struct Gifs {
    picker: Option<GifPicker>,
    closing: Option<Task<()>>,
    animation: u64,
    closed_at: Option<Instant>,
    categories: HashMap<GifProvider, Load<Rc<GifCategories>>>,
    trending: HashMap<GifProvider, Load<Rc<Vec<Gif>>>>,
    frecency_loading: bool,
    favorite_writes: u64,
}

struct GifPicker {
    position: Point<Pixels>,
    search: Entity<InputState>,
    view: View,
    results: Load<Rc<Vec<Gif>>>,
    searches: u64,
    pending_search: Option<Task<()>>,
    provider_menu: bool,
    provider_menu_closed_at: Option<Instant>,
    scroll: ScrollHandle,
    closing: bool,
    swapped: bool,
    /// Picks which favorite fills the Favorites tile; rolled once per open, like Discord.
    favorite_pick: usize,
    animation: u64,
    _search_events: Subscription,
}

#[derive(Clone)]
struct GifItem {
    url: String,
    src: String,
    width: u32,
    height: u32,
    video: bool,
    preview: Preview,
}

#[derive(Clone)]
enum Preview {
    Image(String),
    Video(String),
}

impl Preview {
    /// Video decoding runs one clip at a time, so provider videos whose animated-image
    /// rendition sits at a predictable URL show that instead.
    fn of(src: &str, video: bool) -> Self {
        let video = video || is_video_path(src);
        match video.then(|| image_rendition(src)).flatten() {
            Some(image) => Self::Image(image),
            None if video => Self::Video(src.to_owned()),
            None => Self::Image(src.to_owned()),
        }
    }
}

fn is_video_path(src: &str) -> bool {
    let path = src.split(['?', '#']).next().unwrap_or_default();
    let path = path.to_ascii_lowercase();
    path.ends_with(".mp4") || path.ends_with(".webm")
}

/// Tenor names renditions by the last five characters of the first path segment
/// (`…AAAPo/name.mp4` is mp4, `…AAAAM/name.gif` is tinygif); Giphy keeps the file
/// name and swaps the extension (`200.mp4` / `200.webp`). Klipy uses unrelated names.
fn image_rendition(src: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(src).ok()?;
    let host = url.host_str()?.to_owned();
    let path = url.path().to_owned();
    let (stem, _) = path.rsplit_once('.')?;
    let path = if host == "media.tenor.com" {
        let mut segments = stem.trim_start_matches('/').splitn(2, '/');
        let id = segments.next()?;
        let name = segments.next()?;
        let base = id.get(..id.len().checked_sub(5)?)?;
        if !id[base.len()..].starts_with("AAA") {
            return None;
        }
        format!("/{base}AAAAM/{name}.gif")
    } else if host == "i.giphy.com" || (host.starts_with("media") && host.ends_with(".giphy.com")) {
        format!("{stem}.webp")
    } else {
        return None;
    };
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

impl GifItem {
    fn from_gif(gif: &Gif) -> Self {
        Self {
            url: gif.url.clone(),
            src: gif.src.clone(),
            width: gif.width,
            height: gif.height,
            video: false,
            preview: Preview::Image(gif.src.clone()),
        }
    }

    fn from_favorite(favorite: &FavoriteGif) -> Self {
        Self {
            url: favorite.url.clone(),
            src: favorite.src.clone(),
            width: favorite.width,
            height: favorite.height,
            video: favorite.format == FORMAT_VIDEO,
            preview: Preview::of(&favorite.src, favorite.format == FORMAT_VIDEO),
        }
    }
}

enum Tile {
    Favorites(Option<GifItem>),
    Trending(Option<String>),
    Category(GifCategory),
    Gif(GifItem),
    Placeholder,
}

struct Placed {
    x: f32,
    y: f32,
    height: f32,
    tile: Tile,
}

impl DiscordApp {
    pub(crate) fn toggle_gif_picker(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .gifs
            .picker
            .as_ref()
            .is_some_and(|picker| !picker.closing)
        {
            self.close_gif_picker(cx);
            return;
        }
        if self
            .gifs
            .closed_at
            .is_some_and(|at| at.elapsed() < REOPEN_GUARD)
        {
            return;
        }
        self.close_emoji_picker(cx);
        self.open_gif_picker(position, false, window, cx);
    }

    /// `swapped` opens in place of the emoji picker's tab, without the entry animation.
    fn open_gif_picker(
        &mut self,
        position: Point<Pixels>,
        swapped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.gifs.closing = None;
        self.gifs.animation = self.gifs.animation.wrapping_add(1);
        let provider = settings::get(cx).gif_provider;
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(format!("Search {}", provider.label()))
        });
        let events = cx.subscribe_in(&search, window, |app, _, event, _, cx| {
            if let InputEvent::Change = event {
                app.schedule_gif_search(cx);
            }
        });
        search.update(cx, |input, cx| input.focus(window, cx));
        self.gifs.picker = Some(GifPicker {
            position,
            search,
            view: View::Home,
            results: Load::Loading,
            searches: 0,
            pending_search: None,
            provider_menu: false,
            provider_menu_closed_at: None,
            scroll: ScrollHandle::new(),
            closing: false,
            swapped,
            favorite_pick: rand::random(),
            animation: self.gifs.animation,
            _search_events: events,
        });
        self.gifs.closed_at = None;
        self.gifs
            .categories
            .retain(|_, load| !matches!(load, Load::Failed));
        self.gifs
            .trending
            .retain(|_, load| !matches!(load, Load::Failed));
        self.load_gif_categories(provider, cx);
        self.load_frecency(cx);
        cx.notify();
    }

    pub(crate) fn close_gif_picker(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picker) = &mut self.gifs.picker else {
            return false;
        };
        if picker.closing {
            return false;
        }
        picker.closing = true;
        picker.pending_search = None;
        let animation = picker.animation;
        self.gifs.closed_at = Some(Instant::now());
        if cx.reduce_motion() {
            self.gifs.picker = None;
            self.gifs.closing = None;
            cx.notify();
            return true;
        }
        self.gifs.closing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PICKER_EXIT).await;
            this.update(cx, |app, cx| {
                // Only remove the exact picker this close started with; a newer open replaced it.
                let still_closing = app
                    .gifs
                    .picker
                    .as_ref()
                    .is_some_and(|picker| picker.closing && picker.animation == animation);
                if still_closing {
                    app.gifs.picker = None;
                    app.gifs.closing = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
        true
    }

    fn load_gif_categories(&mut self, provider: GifProvider, cx: &mut Context<Self>) {
        if self.gifs.categories.contains_key(&provider) {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.gifs.categories.insert(provider, Load::Loading);
        let request = Tokio::spawn(cx, async move { session.gif_categories(provider).await });
        cx.spawn(async move |this, cx| {
            let load = match request.await {
                Ok(Ok(categories)) => Load::Ready(Rc::new(categories)),
                Ok(Err(error)) => {
                    log::warn!("Could not load {} categories: {error}", provider.label());
                    Load::Failed
                }
                Err(_) => Load::Failed,
            };
            this.update(cx, |app, cx| {
                app.gifs.categories.insert(provider, load);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_gif_trending(&mut self, provider: GifProvider, cx: &mut Context<Self>) {
        if self.gifs.trending.contains_key(&provider) {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.gifs.trending.insert(provider, Load::Loading);
        let request = Tokio::spawn(cx, async move { session.gif_trending(provider).await });
        cx.spawn(async move |this, cx| {
            let load = match request.await {
                Ok(Ok(gifs)) => Load::Ready(Rc::new(gifs)),
                Ok(Err(error)) => {
                    log::warn!("Could not load trending {} GIFs: {error}", provider.label());
                    Load::Failed
                }
                Err(_) => Load::Failed,
            };
            this.update(cx, |app, cx| {
                app.gifs.trending.insert(provider, load);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_frecency(&mut self, cx: &mut Context<Self>) {
        if self.store.frecency().is_some() || self.gifs.frecency_loading {
            return;
        }
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.gifs.frecency_loading = true;
        let request = Tokio::spawn(cx, async move { session.frecency_settings().await });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                app.gifs.frecency_loading = false;
                match result {
                    Ok(Ok(proto)) => {
                        // A gateway update may have landed while this was in flight.
                        if app.store.frecency().is_none() {
                            app.store.set_frecency(proto);
                        }
                    }
                    Ok(Err(error)) => log::warn!("Could not load favorite GIFs: {error}"),
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn schedule_gif_search(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.gifs.picker else {
            return;
        };
        let query = picker.search.read(cx).value().trim().to_owned();
        if query.is_empty() {
            picker.pending_search = None;
            if matches!(picker.view, View::Search(_)) {
                self.show_gif_view(View::Home, cx);
            }
            return;
        }
        picker.pending_search = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            this.update(cx, |app, cx| app.search_gifs(query, cx)).ok();
        }));
    }

    fn search_gifs(&mut self, query: String, cx: &mut Context<Self>) {
        let provider = settings::get(cx).gif_provider;
        let session = self.session.clone().filter(|_| self.live);
        let Some(picker) = &mut self.gifs.picker else {
            return;
        };
        picker.pending_search = None;
        picker.searches += 1;
        let search = picker.searches;
        picker.view = View::Search(query.clone());
        picker.scroll = ScrollHandle::new();
        let Some(session) = session else {
            picker.results = Load::Failed;
            cx.notify();
            return;
        };
        picker.results = Load::Loading;
        cx.notify();
        let request = Tokio::spawn(
            cx,
            async move { session.gif_search(provider, &query).await },
        );
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                let Some(picker) = &mut app.gifs.picker else {
                    return;
                };
                if picker.searches != search {
                    return;
                }
                picker.results = match result {
                    Ok(Ok(gifs)) => Load::Ready(Rc::new(gifs)),
                    Ok(Err(error)) => {
                        log::warn!("{} GIF search failed: {error}", provider.label());
                        Load::Failed
                    }
                    Err(_) => Load::Failed,
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn show_gif_view(&mut self, view: View, cx: &mut Context<Self>) {
        let provider = settings::get(cx).gif_provider;
        if view == View::Trending {
            self.load_gif_trending(provider, cx);
        }
        if let Some(picker) = &mut self.gifs.picker {
            picker.view = view;
            picker.searches += 1;
            picker.scroll = ScrollHandle::new();
        }
        cx.notify();
    }

    fn open_gif_category(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = &self.gifs.picker else {
            return;
        };
        // `set_value` doesn't emit `Change`, so the search starts here instead of after the delay.
        picker
            .search
            .update(cx, |input, cx| input.set_value(name.clone(), window, cx));
        self.search_gifs(name, cx);
    }

    fn leave_gif_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(picker) = &self.gifs.picker {
            picker
                .search
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.show_gif_view(View::Home, cx);
    }

    fn toggle_gif_provider_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(picker) = &mut self.gifs.picker {
            // The menu's outside-press handler closes it before this click lands.
            let just_closed = picker
                .provider_menu_closed_at
                .is_some_and(|at| at.elapsed() < REOPEN_GUARD);
            picker.provider_menu = !picker.provider_menu && !just_closed;
            cx.notify();
        }
    }

    fn select_gif_provider(
        &mut self,
        provider: GifProvider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        settings::update(cx, |settings| settings.gif_provider = provider);
        let Some(picker) = &mut self.gifs.picker else {
            return;
        };
        picker.provider_menu = false;
        let view = picker.view.clone();
        picker.search.update(cx, |input, cx| {
            input.set_placeholder(format!("Search {}", provider.label()), window, cx)
        });
        self.load_gif_categories(provider, cx);
        match view {
            View::Search(query) => self.search_gifs(query, cx),
            view => self.show_gif_view(view, cx),
        }
    }

    fn pick_gif(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        self.close_gif_picker(cx);
        self.send_content(url, Vec::new(), window, cx);
    }

    fn toggle_favorite_gif(&mut self, item: &GifItem, cx: &mut Context<Self>) {
        let Some(previous) = self.store.frecency().map(<[u8]>::to_vec) else {
            return;
        };
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let mut favorites = self.store.favorite_gifs().to_vec();
        if let Some(index) = favorites.iter().position(|gif| gif.url == item.url) {
            favorites.remove(index);
        } else {
            let order = favorites.iter().map(|gif| gif.order).max().unwrap_or(0) + 1;
            favorites.push(FavoriteGif {
                url: item.url.clone(),
                src: item.src.clone(),
                format: if item.video {
                    FORMAT_VIDEO
                } else {
                    FORMAT_IMAGE
                },
                width: item.width,
                height: item.height,
                order,
            });
        }
        let Ok(next) = settings_proto::with_favorite_gifs(&previous, &favorites) else {
            log::warn!("Could not encode favorite GIFs");
            return;
        };
        self.store.set_frecency(next.clone());
        self.gifs.favorite_writes += 1;
        let write = self.gifs.favorite_writes;
        cx.notify();
        let request = Tokio::spawn(
            cx,
            async move { session.update_frecency_settings(&next).await },
        );
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |app, cx| {
                // A newer toggle already moved the optimistic state on; its own reply settles it.
                if app.gifs.favorite_writes != write {
                    return;
                }
                match result {
                    Ok(Ok(state)) => app.store.set_frecency(state),
                    Ok(Err(error)) => {
                        log::warn!("Could not save favorite GIFs: {error}");
                        app.store.set_frecency(previous);
                    }
                    Err(_) => app.store.set_frecency(previous),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn home_tiles(
    categories: Option<&Load<Rc<GifCategories>>>,
    favorite: Option<&FavoriteGif>,
) -> Vec<Tile> {
    let favorite = favorite.map(GifItem::from_favorite);
    let mut tiles = Vec::new();
    match categories {
        Some(Load::Ready(categories)) => {
            tiles.push(Tile::Favorites(favorite.clone()));
            tiles.push(Tile::Trending(categories.trending_preview.clone()));
            tiles.extend(categories.categories.iter().cloned().map(Tile::Category));
        }
        Some(Load::Failed) => {
            tiles.push(Tile::Favorites(favorite.clone()));
            tiles.push(Tile::Trending(None));
        }
        Some(Load::Loading) | None => {
            tiles.push(Tile::Favorites(favorite.clone()));
            tiles.push(Tile::Trending(None));
            tiles.extend((0..6).map(|_| Tile::Placeholder));
        }
    }
    tiles
}

fn random_favorite(favorites: &[FavoriteGif], pick: usize) -> Option<&FavoriteGif> {
    favorites.get(pick.checked_rem(favorites.len())?)
}

fn grid_layout(tiles: Vec<Tile>) -> (Vec<Placed>, f32) {
    let placed: Vec<_> = tiles
        .into_iter()
        .enumerate()
        .map(|(index, tile)| Placed {
            x: (index % 2) as f32 * (COLUMN + GAP),
            y: (index / 2) as f32 * (TILE + GAP),
            height: TILE,
            tile,
        })
        .collect();
    let rows = placed.len().div_ceil(2) as f32;
    (placed, (rows * (TILE + GAP) - GAP).max(0.))
}

fn masonry_layout(items: Vec<GifItem>) -> (Vec<Placed>, f32) {
    let mut columns = [0f32; 2];
    let placed = items
        .into_iter()
        .map(|item| {
            let height = if item.width > 0 && item.height > 0 {
                (COLUMN * item.height as f32 / item.width as f32).clamp(64., 2. * COLUMN)
            } else {
                TILE
            };
            let column = usize::from(columns[1] < columns[0]);
            let y = columns[column];
            columns[column] += height + GAP;
            Placed {
                x: column as f32 * (COLUMN + GAP),
                y,
                height,
                tile: Tile::Gif(item),
            }
        })
        .collect();
    (placed, (columns[0].max(columns[1]) - GAP).max(0.))
}

fn placeholder_layout() -> (Vec<Placed>, f32) {
    grid_layout((0..8).map(|_| Tile::Placeholder).collect())
}

enum Body {
    Tiles(Vec<Placed>, f32),
    Message(&'static str),
}

fn body(app: &DiscordApp, picker: &GifPicker, provider: GifProvider) -> Body {
    let gifs = |load: Option<&Load<Rc<Vec<Gif>>>>, empty: &'static str| match load {
        Some(Load::Ready(gifs)) if gifs.is_empty() => Body::Message(empty),
        Some(Load::Ready(gifs)) => {
            let (placed, height) = masonry_layout(gifs.iter().map(GifItem::from_gif).collect());
            Body::Tiles(placed, height)
        }
        Some(Load::Failed) => Body::Message(unavailable(provider)),
        Some(Load::Loading) | None => {
            let (placed, height) = placeholder_layout();
            Body::Tiles(placed, height)
        }
    };
    match &picker.view {
        View::Home => {
            let (placed, height) = grid_layout(home_tiles(
                app.gifs.categories.get(&provider),
                random_favorite(app.store.favorite_gifs(), picker.favorite_pick),
            ));
            Body::Tiles(placed, height)
        }
        View::Trending => gifs(
            app.gifs.trending.get(&provider),
            "No trending GIFs right now",
        ),
        View::Search(_) => gifs(Some(&picker.results), "No GIFs match your search"),
        View::Favorites => {
            let favorites = app.store.favorite_gifs();
            if favorites.is_empty() {
                return if app.store.frecency().is_none() && app.gifs.frecency_loading {
                    let (placed, height) = placeholder_layout();
                    Body::Tiles(placed, height)
                } else {
                    Body::Message("Click the star on a GIF to save it here")
                };
            }
            let (placed, height) =
                masonry_layout(favorites.iter().map(GifItem::from_favorite).collect());
            Body::Tiles(placed, height)
        }
    }
}

fn unavailable(provider: GifProvider) -> &'static str {
    match provider {
        GifProvider::Klipy => "Klipy is unavailable right now",
        GifProvider::Tenor => "Tenor is unavailable right now — try another provider",
        GifProvider::Giphy => "Giphy is unavailable right now — try another provider",
    }
}

pub(super) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    let picker = app.gifs.picker.as_ref()?;
    let closing = picker.closing;
    let provider = settings::get(cx).gif_provider;
    let close = app.entity.clone();
    let key = if closing {
        "gif-picker-out"
    } else {
        "gif-picker-in"
    };
    let animation = Animation::new(if closing { PICKER_EXIT } else { PICKER_ENTER })
        .with_easing(ease_out_quint());
    let panel = div()
        .id("gif-picker")
        .relative()
        .flex()
        .flex_col()
        .w(rpx(WIDTH))
        .h(panel_height(picker.position, window))
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgba(theme::BORDER))
        .bg(rgb(theme::ELEVATED))
        .shadow_lg()
        .overflow_hidden()
        .when(!closing, |this| {
            // A closing panel stops occluding so clicks fall through until the close task removes it.
            this.occlude().on_mouse_down_out(move |_, _, cx| {
                close.update(cx, |app, cx| app.close_gif_picker(cx)).ok();
            })
        })
        .child(header(app, picker, provider, window, cx))
        .child(grid(
            app,
            picker,
            provider,
            !closing && !cx.reduce_motion(),
            window,
            cx,
        ))
        .when(picker.provider_menu && !closing, |this| {
            this.child(provider_menu(app, provider))
        });
    let panel = if picker.swapped && !closing {
        panel.into_any_element()
    } else {
        panel
            .with_animation((key, picker.animation), animation, move |this, delta| {
                let progress = if closing { 1. - delta } else { delta };
                this.top(px((1. - progress) * PICKER_RISE))
                    .opacity(progress)
            })
            .into_any_element()
    };
    Some(
        deferred(
            anchored()
                .position(picker.position)
                .anchor(Anchor::BottomRight)
                .snap_to_window_with_margin(px(8.))
                .child(panel),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

fn header(
    app: &DiscordApp,
    picker: &GifPicker,
    provider: GifProvider,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    let entity = app.entity.clone();
    let focused = picker.search.read(cx).focus_handle(cx).is_focused(window);
    let title = match picker.view {
        View::Trending => Some("Trending GIFs"),
        View::Favorites => Some("Favorites"),
        View::Home | View::Search(_) => None,
    };
    let field = match title {
        Some(title) => div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(rpx(8.))
            .h(rpx(FIELD))
            .px(rpx(8.))
            .rounded(rpx(8.))
            .bg(rgb(theme::BG))
            .border_1()
            .border_color(rgba(theme::BORDER))
            .child(
                theme::icon_button("gif-back", "arrow-left", 20., theme::MUTED).on_click({
                    let entity = entity.clone();
                    move |_, window, cx| {
                        entity
                            .update(cx, |app, cx| app.leave_gif_view(window, cx))
                            .ok();
                    }
                }),
            )
            .child(
                div()
                    .text_size(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::STRONG))
                    .child(title),
            )
            .into_any_element(),
        None => div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(rpx(8.))
            .h(rpx(FIELD))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgb(theme::BG))
            .border_1()
            .border_color(if focused {
                rgb(theme::FOCUS)
            } else {
                rgba(theme::BORDER)
            })
            .child(theme::icon("search", 18., theme::TEXT))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&picker.search)
                        .appearance(false)
                        .h(rpx(40.))
                        .px_0()
                        .text_size(rpx(16.)),
                ),
            )
            .into_any_element(),
    };
    let open = picker.provider_menu;
    let provider_button = div()
        .id("gif-provider")
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(FIELD))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgba(theme::BORDER))
        .bg(rgb(theme::BG))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::HOVER)))
        .on_click({
            let entity = entity.clone();
            move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.toggle_gif_provider_menu(cx))
                    .ok();
            }
        })
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::STRONG))
                .child(provider.label()),
        )
        .child(theme::icon(
            if open { "chevron-up" } else { "chevron-down" },
            18.,
            theme::TEXT,
        ));
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(rpx(HEADER_GAP))
        .p(rpx(HEADER_PADDING))
        .border_b_1()
        .border_color(rgba(theme::BORDER))
        .child(tabs(app, Tab::Media))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(8.))
                .child(field)
                .child(provider_button),
        )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tab {
    Media,
    Emoji,
}

pub(super) fn tabs(app: &DiscordApp, selected: Tab) -> impl IntoElement {
    let tab = |label: &'static str, tab: Tab| {
        let active = tab == selected;
        let entity = app.entity.clone();
        div()
            .id(label)
            .flex()
            .items_center()
            .h(rpx(TAB))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .text_size(rpx(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(if active { theme::STRONG } else { theme::TEXT }))
            .when(active, |this| this.bg(rgba(theme::SELECTED)))
            .when(!active, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(rgba(theme::HOVER)).text_color(rgb(theme::STRONG)))
                    .on_click(move |_, window, cx| {
                        entity
                            .update(cx, |app, cx| app.switch_picker_tab(tab, window, cx))
                            .ok();
                    })
            })
            .child(label)
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .child(tab("Media", Tab::Media))
        .child(tab("Emoji", Tab::Emoji))
}

/// Fits the picker between the window top and its anchor when the window is short or zoomed.
pub(super) fn panel_height(position: Point<Pixels>, window: &Window) -> Pixels {
    let rem = window.rem_size();
    let full = rpx(HEIGHT).to_pixels(rem);
    let available = position.y - px(8.);
    full.min(available)
        .max(rpx(MIN_HEIGHT).to_pixels(rem).min(full))
}

impl DiscordApp {
    fn switch_picker_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        match tab {
            Tab::Media => {
                let Some(position) = self.take_emoji_picker() else {
                    return;
                };
                self.open_gif_picker(position, true, window, cx);
            }
            Tab::Emoji => {
                let Some(position) = self.gifs.picker.take().map(|picker| picker.position) else {
                    return;
                };
                self.gifs.closing = None;
                self.open_emoji_tab(position, window, cx);
            }
        }
    }
}

fn provider_menu(app: &DiscordApp, selected: GifProvider) -> impl IntoElement {
    let close = app.entity.clone();
    div()
        .id("gif-provider-menu")
        .absolute()
        .top(rpx(HEADER_PADDING + TAB + HEADER_GAP + FIELD + 6.))
        .right(rpx(HEADER_PADDING))
        .w(rpx(110.))
        .p(rpx(6.))
        .flex()
        .flex_col()
        .gap(rpx(2.))
        .rounded(rpx(8.))
        .border_1()
        .border_color(rgba(theme::BORDER))
        .bg(rgb(theme::ELEVATED))
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(move |_, _, cx| {
            close
                .update(cx, |app, cx| {
                    if let Some(picker) = &mut app.gifs.picker {
                        picker.provider_menu = false;
                        picker.provider_menu_closed_at = Some(Instant::now());
                        cx.notify();
                    }
                })
                .ok();
        })
        .children(GifProvider::ALL.into_iter().map(|provider| {
            let entity = app.entity.clone();
            let active = provider == selected;
            div()
                .id(provider.label())
                .flex()
                .items_center()
                .justify_between()
                .h(rpx(40.))
                .px(rpx(10.))
                .rounded(rpx(6.))
                .cursor_pointer()
                .hover(|style| style.bg(rgba(theme::SELECTED)))
                .when(active, |this| this.bg(rgba(theme::SELECTED)))
                .text_size(rpx(16.))
                .text_color(rgb(if active { theme::STRONG } else { theme::MUTED }))
                .on_click(move |_, window, cx| {
                    entity
                        .update(cx, |app, cx| app.select_gif_provider(provider, window, cx))
                        .ok();
                })
                .child(provider.label())
                .when(active, |this| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(rpx(20.))
                            .rounded_full()
                            .bg(rgb(theme::BLURPLE))
                            .child(theme::icon("check", 14., 0xFFFFFF)),
                    )
                })
        }))
}

fn grid(
    app: &DiscordApp,
    picker: &GifPicker,
    provider: GifProvider,
    animate: bool,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    let (placed, height) = match body(app, picker, provider) {
        Body::Tiles(placed, height) => (placed, height),
        Body::Message(message) => {
            return div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px(rpx(24.))
                .text_size(rpx(14.))
                .text_color(rgb(theme::MUTED))
                .child(message)
                .into_any_element();
        }
    };
    let rem = window.rem_size();
    let top = to_css(-picker.scroll.offset().y, rem);
    let viewport = match to_css(picker.scroll.bounds().size.height, rem) {
        height if height > 0. => height,
        _ => HEIGHT,
    };
    let visible = (top - OVERSCAN)..(top + viewport + OVERSCAN);
    let band = Rc::new(Cell::new((top / OVERSCAN) as i64));
    let scroll = picker.scroll.clone();
    let entity = app.entity.clone();
    let grid = div()
        .id(("gif-grid", picker.searches))
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&picker.scroll)
        .on_scroll_wheel(move |_, _, cx| {
            // Tiles outside the viewport aren't rendered, so crossing a band needs a re-render.
            let next = (to_css(-scroll.offset().y, rem) / OVERSCAN) as i64;
            if band.replace(next) != next {
                entity.update(cx, |_, cx| cx.notify()).ok();
            }
        })
        .child(
            div()
                .relative()
                .mx(rpx(PADDING))
                .my(rpx(PADDING))
                .h(rpx(height))
                .children(
                    placed
                        .into_iter()
                        .enumerate()
                        .filter(|(_, placed)| {
                            placed.y + placed.height >= visible.start && placed.y <= visible.end
                        })
                        .map(|(index, placed)| tile(app, index, placed, animate, cx)),
                ),
        );
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(grid)
        .vertical_scrollbar(&picker.scroll)
        .into_any_element()
}

fn to_css(pixels: Pixels, rem: Pixels) -> f32 {
    pixels.as_f32() * theme::BASE_FONT_SIZE / rem.as_f32()
}

fn tile(app: &DiscordApp, index: usize, placed: Placed, animate: bool, cx: &App) -> AnyElement {
    let entity = app.entity.clone();
    let frame = div()
        .id(("gif-tile", index))
        .group("gif-tile")
        .absolute()
        .left(rpx(placed.x))
        .top(rpx(placed.y))
        .w(rpx(COLUMN))
        .h(rpx(placed.height))
        .rounded(rpx(RADIUS))
        .overflow_hidden()
        .bg(rgba(theme::BORDER));
    match placed.tile {
        Tile::Placeholder => frame.into_any_element(),
        Tile::Favorites(favorite) => frame
            .bg(rgb(theme::BLURPLE))
            .cursor_pointer()
            .children(favorite.map(|item| {
                media(
                    &item.preview,
                    (item.width, item.height),
                    animate,
                    placed.height,
                    cx,
                )
            }))
            .child(scrim(0x5865F2B3, 0x5865F299))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.show_gif_view(View::Favorites, cx))
                    .ok();
            })
            .child(label(None, "Favorites"))
            .into_any_element(),
        Tile::Trending(preview) => frame
            .cursor_pointer()
            .children(
                preview.map(|src| media(&Preview::Image(src), (0, 0), animate, placed.height, cx)),
            )
            .child(scrim(0x000000A6, 0x00000073))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.show_gif_view(View::Trending, cx))
                    .ok();
            })
            .child(label(Some("trending-up"), "Trending GIFs"))
            .into_any_element(),
        Tile::Category(category) => {
            let name = category.name.clone();
            frame
                .cursor_pointer()
                .child(media(
                    &Preview::Image(category.src.clone()),
                    (0, 0),
                    animate,
                    placed.height,
                    cx,
                ))
                .child(scrim(0x000000A6, 0x00000073))
                .on_click(move |_, window, cx| {
                    let name = name.clone();
                    entity
                        .update(cx, |app, cx| app.open_gif_category(name, window, cx))
                        .ok();
                })
                .child(label(None, &category.name))
                .into_any_element()
        }
        Tile::Gif(item) => {
            let favorite = app.store.is_favorite_gif(&item.url);
            let can_favorite = app.store.frecency().is_some() && app.live;
            let url = item.url.clone();
            let star = can_favorite.then(|| {
                let entity = entity.clone();
                let item = item.clone();
                div()
                    .id("gif-favorite")
                    .absolute()
                    .top(rpx(6.))
                    .right(rpx(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(32.))
                    .rounded(rpx(6.))
                    .bg(rgba(0x000000A0))
                    .invisible()
                    .group_hover("gif-tile", |style| style.visible())
                    .hover(|style| style.bg(rgba(0x000000D0)))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        entity
                            .update(cx, |app, cx| app.toggle_favorite_gif(&item, cx))
                            .ok();
                    })
                    .child(theme::icon(
                        if favorite { "star-filled" } else { "star" },
                        18.,
                        if favorite { theme::YELLOW } else { 0xFFFFFF },
                    ))
            });
            frame
                .cursor_pointer()
                .border_2()
                .border_color(transparent_black())
                .hover(|style| style.border_color(rgb(theme::BLURPLE)))
                .child(media(
                    &item.preview,
                    (item.width, item.height),
                    animate,
                    placed.height,
                    cx,
                ))
                .on_click(move |_, window, cx| {
                    let url = url.clone();
                    entity
                        .update(cx, |app, cx| app.pick_gif(url, window, cx))
                        .ok();
                })
                .children(star)
                .into_any_element()
        }
    }
}

fn media(
    preview: &Preview,
    (width, height): (u32, u32),
    animate: bool,
    tile_height: f32,
    cx: &App,
) -> AnyElement {
    let pixels = theme::image_px(COLUMN.max(tile_height));
    let (src, video) = match preview {
        Preview::Image(src) => (src, false),
        Preview::Video(src) => (src, true),
    };
    let key = {
        let mut hasher = std::hash::DefaultHasher::new();
        std::hash::Hash::hash(src, &mut hasher);
        std::hash::Hasher::finish(&hasher)
    };
    let frames = if video {
        let spec = ImageSpec::Video {
            url: src.clone(),
            width,
            height,
        };
        crate::media::video::resolve(&spec, pixels, cx)
    } else if animate {
        let spec = ImageSpec::Media {
            url: src.clone(),
            width,
            height,
            animated: true,
        };
        animated::resolve(&spec, pixels, cx)
    } else {
        None
    };
    if let Some(frames) = frames {
        return animated::image(frames)
            .absolute()
            .inset_0()
            .size_full()
            .object_fit(ObjectFit::Cover)
            .rounded(rpx(RADIUS))
            .id(("gif-media", key))
            .into_any_element();
    }
    if !video {
        let spec = ImageSpec::Media {
            url: src.clone(),
            width,
            height,
            animated: true,
        };
        return animated::poster(&spec, pixels, cx)
            .map(|image| {
                image
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded(rpx(RADIUS))
                    .id(("gif-still", key))
                    .into_any_element()
            })
            .or_else(|| {
                let fallback = animated::fallback(&spec, pixels, cx)?;
                crate::media::images::resolve(&fallback, pixels, cx).map(|path| {
                    img(path)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .rounded(rpx(RADIUS))
                        .into_any_element()
                })
            })
            .unwrap_or_else(|| div().into_any_element());
    }
    // Discord's media proxy renders a still frame for its own videos while the clip decodes.
    let still = ImageSpec::VideoPoster {
        id: key,
        url: src.clone(),
        width,
        height,
    };
    match crate::media::images::resolve(&still, pixels, cx) {
        Some(path) => img(path)
            .absolute()
            .inset_0()
            .size_full()
            .object_fit(ObjectFit::Cover)
            .rounded(rpx(RADIUS))
            .into_any_element(),
        None => div().into_any_element(),
    }
}

fn scrim(color: u32, hovered: u32) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .rounded(rpx(RADIUS))
        .bg(rgba(color))
        .group_hover("gif-tile", move |style| style.bg(rgba(hovered)))
}

fn label(icon: Option<&'static str>, text: &str) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .gap(rpx(6.))
        .px(rpx(12.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(0xFFFFFF))
        .children(icon.map(|icon| theme::icon(icon, 20., 0xFFFFFF)))
        .child(div().truncate().child(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{
        FORMAT_IMAGE, FavoriteGif, Preview, image_rendition, is_video_path, random_favorite,
    };

    #[test]
    fn provider_videos_map_to_animated_images() {
        assert_eq!(
            image_rendition("https://media.tenor.com/WBMoHT5SvJAAAAPo/shia-labeouf-clapping.mp4")
                .as_deref(),
            Some("https://media.tenor.com/WBMoHT5SvJAAAAAM/shia-labeouf-clapping.gif")
        );
        assert_eq!(
            image_rendition("https://media4.giphy.com/media/v1.abc/sampleGiphyOne/giphy.mp4?cid=1")
                .as_deref(),
            Some("https://media4.giphy.com/media/v1.abc/sampleGiphyOne/giphy.webp")
        );
        for unmapped in [
            "https://static.klipy.com/ii/f87f/84/09/sample-unmapped-video.mp4",
            "https://media.tenor.com/short/name.mp4",
            "https://cdn.discordapp.com/attachments/1/2/clip.mp4",
            "https://media.giphy.com.evil.test/a/giphy.mp4",
        ] {
            assert_eq!(image_rendition(unmapped), None, "{unmapped}");
        }
    }

    #[test]
    fn favorite_previews_prefer_images_and_keep_unmapped_videos() {
        let tenor = "https://media.tenor.com/WBMoHT5SvJAAAAPo/clap.mp4";
        assert!(
            matches!(Preview::of(tenor, false), Preview::Image(url) if url.ends_with("AAAAM/clap.gif"))
        );
        let klipy = "https://static.klipy.com/ii/a/b/c/clip.mp4";
        assert!(matches!(Preview::of(klipy, true), Preview::Video(url) if url == klipy));
        let gif = "https://media.tenor.com/WBMoHT5SvJAAAAAC/clap.gif";
        assert!(matches!(Preview::of(gif, false), Preview::Image(url) if url == gif));
        assert!(is_video_path("https://x.test/a.WEBM?x=1"));
    }

    #[test]
    fn random_favorite_wraps_and_handles_empty() {
        let favorite = |order| FavoriteGif {
            url: format!("https://tenor.com/view/{order}"),
            src: String::new(),
            format: FORMAT_IMAGE,
            width: 1,
            height: 1,
            order,
        };
        let favorites = [favorite(2), favorite(1)];
        assert_eq!(random_favorite(&favorites, 5).map(|gif| gif.order), Some(1));
        assert!(random_favorite(&[], 5).is_none());
    }
}
