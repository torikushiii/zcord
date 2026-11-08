mod grid;
mod view;

pub(crate) use grid::{category_previews, gif_cell};
pub(crate) use view::{Tab, panel_height, render, tabs};

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
    app::{DiscordApp, expressions::emoji_picker, profile::styles::ImageTarget},
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
const SEARCH_DELAY: Duration = Duration::from_millis(300);
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
    /// The picker belongs to the thread pane's composer rather than the main one.
    pub(crate) pane: bool,
    /// The picker is the profile editor's Choose GIF dialog, picking this image.
    style_target: Option<ImageTarget>,
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
        self.toggle_gif_picker_for(false, position, window, cx);
    }

    pub(crate) fn toggle_gif_picker_for(
        &mut self,
        pane: bool,
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
        self.take_sticker_picker();
        self.gifs.pane = pane;
        self.gifs.style_target = None;
        self.open_gif_picker(position, false, window, cx);
    }

    /// Opens the picker as a centered dialog whose pick becomes the draft avatar or banner.
    pub(crate) fn open_style_gif_picker(
        &mut self,
        target: ImageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_emoji_picker(cx);
        self.take_sticker_picker();
        self.gifs.pane = false;
        self.gifs.style_target = Some(target);
        self.open_gif_picker(Point::default(), false, window, cx);
    }

    /// Starts loading the provider's categories, whose previews tile the Choose GIF option.
    pub(crate) fn preload_gif_categories(&mut self, cx: &mut Context<Self>) {
        let provider = settings::get(cx).gif_provider;
        self.gifs
            .categories
            .retain(|_, load| !matches!(load, Load::Failed));
        self.load_gif_categories(provider, cx);
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

    fn pick_gif(&mut self, url: String, src: String, window: &mut Window, cx: &mut Context<Self>) {
        self.close_gif_picker(cx);
        if let Some(target) = self.gifs.style_target {
            self.pick_style_gif(target, src, cx);
        } else if self.gifs.pane {
            self.send_pane_content(url, window, cx);
        } else {
            self.send_content(url, Vec::new(), window, cx);
        }
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

#[cfg(test)]
mod tests {
    use super::{
        FORMAT_IMAGE, FavoriteGif, Preview, grid::random_favorite, image_rendition, is_video_path,
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
