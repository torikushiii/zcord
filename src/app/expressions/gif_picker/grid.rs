use super::view::{Body, body};
use super::*;

const COLUMN: f32 = (WIDTH - 2. - 2. * PADDING - GAP) / 2.;
const TILE: f32 = 124.;
const RADIUS: f32 = 4.;
const OVERSCAN: f32 = 240.;

pub(super) fn home_tiles(
    categories: Option<&Load<Rc<GifCategories>>>,
    favorite: Option<Option<&FavoriteGif>>,
) -> Vec<Tile> {
    let mut tiles: Vec<Tile> = favorite
        .map(|favorite| Tile::Favorites(favorite.map(GifItem::from_favorite)))
        .into_iter()
        .collect();
    match categories {
        Some(Load::Ready(categories)) => {
            tiles.push(Tile::Trending(categories.trending_preview.clone()));
            tiles.extend(categories.categories.iter().cloned().map(Tile::Category));
        }
        Some(Load::Failed) => {
            tiles.push(Tile::Trending(None));
        }
        Some(Load::Loading) | None => {
            tiles.push(Tile::Trending(None));
            tiles.extend((0..6).map(|_| Tile::Placeholder));
        }
    }
    tiles
}

pub(super) fn random_favorite(favorites: &[FavoriteGif], pick: usize) -> Option<&FavoriteGif> {
    favorites.get(pick.checked_rem(favorites.len())?)
}

pub(super) fn grid_layout(tiles: Vec<Tile>) -> (Vec<Placed>, f32) {
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

pub(super) fn masonry_layout(items: Vec<GifItem>) -> (Vec<Placed>, f32) {
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

pub(super) fn placeholder_layout() -> (Vec<Placed>, f32) {
    grid_layout((0..8).map(|_| Tile::Placeholder).collect())
}

pub(super) fn grid(
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
                .text_color(rgb(theme::muted()))
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

pub(super) fn tile(
    app: &DiscordApp,
    index: usize,
    placed: Placed,
    animate: bool,
    cx: &App,
) -> AnyElement {
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
        .bg(rgba(theme::border()));
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
            let (url, src) = (item.url.clone(), item.src.clone());
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
                    let (url, src) = (url.clone(), src.clone());
                    entity
                        .update(cx, |app, cx| app.pick_gif(url, src, window, cx))
                        .ok();
                })
                .children(star)
                .into_any_element()
        }
    }
}

pub(super) fn media(
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

pub(crate) fn category_previews(app: &DiscordApp, count: usize, cx: &App) -> Vec<String> {
    match app.gifs.categories.get(&settings::get(cx).gif_provider) {
        Some(Load::Ready(categories)) => categories
            .categories
            .iter()
            .take(count)
            .map(|category| category.src.clone())
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn gif_cell(src: &str, size: f32, [tl, tr, br, bl]: [f32; 4], cx: &App) -> AnyElement {
    let pixels = theme::image_px(size);
    let spec = ImageSpec::Media {
        url: src.to_owned(),
        width: 0,
        height: 0,
        animated: true,
    };
    fn fill<E: Styled>(this: E, [tl, tr, br, bl]: [f32; 4]) -> E {
        this.absolute()
            .inset_0()
            .size_full()
            .rounded_tl(rpx(tl))
            .rounded_tr(rpx(tr))
            .rounded_br(rpx(br))
            .rounded_bl(rpx(bl))
    }
    let corners = [tl, tr, br, bl];
    if let Some(frames) = animated::resolve(&spec, pixels, cx) {
        let key = {
            let mut hasher = std::hash::DefaultHasher::new();
            std::hash::Hash::hash(src, &mut hasher);
            std::hash::Hasher::finish(&hasher)
        };
        return fill(
            animated::image(frames).object_fit(ObjectFit::Cover),
            corners,
        )
        .id(("gif-cell", key))
        .into_any_element();
    }
    animated::poster(&spec, pixels, cx)
        .map(|image| fill(image.object_fit(ObjectFit::Cover), corners).into_any_element())
        .or_else(|| {
            let fallback = animated::fallback(&spec, pixels, cx)?;
            crate::media::images::resolve(&fallback, pixels, cx).map(|path| {
                fill(img(path).object_fit(ObjectFit::Cover), corners).into_any_element()
            })
        })
        .unwrap_or_else(|| div().into_any_element())
}

fn scrim(color: u32, hovered: u32) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .rounded(rpx(RADIUS))
        .bg(rgba(color))
        .group_hover("gif-tile", move |style| style.bg(rgba(hovered)))
}

pub(super) fn label(icon: Option<&'static str>, text: &str) -> impl IntoElement {
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
