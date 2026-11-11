use super::grid::{
    grid, grid_layout, home_tiles, masonry_layout, placeholder_layout, random_favorite,
};
use super::*;

const TAB: f32 = 32.;
const FIELD: f32 = 42.;
const HEADER_PADDING: f32 = 12.;
const HEADER_GAP: f32 = 12.;
const MIN_HEIGHT: f32 = 240.;
const PICKER_RISE: f32 = 8.;
const MODAL_MARGIN: f32 = 40.;
const PICKER_ENTER: Duration = Duration::from_millis(150);

pub(super) enum Body {
    Tiles(Vec<Placed>, f32),
    Message(&'static str),
}

pub(super) fn body(app: &DiscordApp, picker: &GifPicker, provider: GifProvider) -> Body {
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
            let favorite = app
                .gifs
                .style_target
                .is_none()
                .then(|| random_favorite(app.store.favorite_gifs(), picker.favorite_pick));
            let (placed, height) =
                grid_layout(home_tiles(app.gifs.categories.get(&provider), favorite));
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

pub(crate) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
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
    let modal = app.gifs.style_target.is_some();
    let height = if modal {
        let rem = window.rem_size();
        rpx(HEIGHT)
            .to_pixels(rem)
            .min(window.viewport_size().height - px(2. * MODAL_MARGIN))
    } else {
        panel_height(picker.position, window)
    };
    let panel = div()
        .id("gif-picker")
        .relative()
        .flex()
        .flex_col()
        .w(rpx(WIDTH))
        .h(height)
        .rounded(rpx(if modal { 12. } else { 8. }))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(if modal {
            theme::surface_high()
        } else {
            theme::elevated()
        }))
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
    } else if modal {
        panel
            .with_animation((key, picker.animation), animation, move |this, delta| {
                this.opacity(if closing { 1. - delta } else { delta })
            })
            .into_any_element()
    } else {
        panel
            .with_animation((key, picker.animation), animation, move |this, delta| {
                let progress = if closing { 1. - delta } else { delta };
                this.top(px((1. - progress) * PICKER_RISE))
                    .opacity(progress)
            })
            .into_any_element()
    };
    if modal {
        return Some(
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!closing, |this| this.occlude())
                    .bg(rgba(theme::SCRIM).opacity(if closing { 0. } else { 1. }))
                    .child(theme::selection_guard())
                    .child(panel),
            )
            .with_priority(2)
            .into_any_element(),
        );
    }
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
            .bg(rgb(theme::bg()))
            .border_1()
            .border_color(rgba(theme::border()))
            .child(
                theme::icon_button("gif-back", "arrow-left", 20., theme::muted()).on_click({
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
                    .text_color(rgb(theme::strong()))
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
            .bg(rgb(theme::bg()))
            .border_1()
            .border_color(if focused {
                rgb(theme::focus())
            } else {
                rgba(theme::border())
            })
            .child(theme::icon("search", 18., theme::text()))
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
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::bg()))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::hover())))
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
                .text_color(rgb(theme::strong()))
                .child(provider.label()),
        )
        .child(theme::icon(
            if open { "chevron-up" } else { "chevron-down" },
            18.,
            theme::text(),
        ));
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(rpx(HEADER_GAP))
        .p(rpx(HEADER_PADDING))
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(match app.gifs.style_target {
            Some(_) => div()
                .flex()
                .items_center()
                .justify_between()
                .h(rpx(TAB))
                .child(
                    div()
                        .text_size(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child("Choose GIF"),
                )
                .child(
                    theme::icon_button("gif-dialog-close", "x", 24., theme::muted()).on_click({
                        let entity = entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |app, cx| app.close_gif_picker(cx)).ok();
                        }
                    }),
                )
                .into_any_element(),
            None => tabs(app, Tab::Media).into_any_element(),
        })
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
pub(crate) enum Tab {
    Media,
    Stickers,
    Emoji,
}

pub(crate) fn tabs(app: &DiscordApp, selected: Tab) -> impl IntoElement {
    let tab = |label: &'static str, tab: Tab| {
        let active = tab == selected;
        let entity = app.entity.clone();
        let keyboard = entity.clone();
        div()
            .id(label)
            .role(Role::Tab)
            .aria_selected(active)
            .focusable()
            .tab_index(0)
            .flex()
            .items_center()
            .h(rpx(TAB))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .text_size(rpx(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(if active {
                theme::strong()
            } else {
                theme::text()
            }))
            .when(active, |this| this.bg(rgba(theme::selected())))
            .focus_visible(|style| style.bg(rgba(theme::selected())))
            .when(!active, |this| {
                this.cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(rgba(theme::hover()))
                            .text_color(rgb(theme::strong()))
                    })
                    .on_click(move |_, window, cx| {
                        entity
                            .update(cx, |app, cx| {
                                app.switch_picker_tab(selected, tab, window, cx)
                            })
                            .ok();
                    })
            })
            .on_key_down(move |event, window, cx| {
                if !matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    return;
                }
                keyboard
                    .update(cx, |app, cx| {
                        app.switch_picker_tab(selected, tab, window, cx)
                    })
                    .ok();
                cx.stop_propagation();
            })
            .child(label)
    };
    div()
        .id("expression-picker-tabs")
        .role(Role::TabList)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .child(tab("Media", Tab::Media))
        .child(tab("Stickers", Tab::Stickers))
        .child(tab("Emoji", Tab::Emoji))
}

pub(crate) fn panel_height(position: Point<Pixels>, window: &Window) -> Pixels {
    let rem = window.rem_size();
    let full = rpx(HEIGHT).to_pixels(rem);
    let available = position.y - px(8.);
    full.min(available)
        .max(rpx(MIN_HEIGHT).to_pixels(rem).min(full))
}

impl DiscordApp {
    fn switch_picker_tab(
        &mut self,
        selected: Tab,
        tab: Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if selected == tab {
            return;
        }
        let position = match selected {
            Tab::Media => self.gifs.picker.take().map(|picker| picker.position),
            Tab::Stickers => self.take_sticker_picker(),
            Tab::Emoji => self.take_emoji_picker(),
        };
        let Some(position) = position else {
            return;
        };
        self.gifs.closing = None;
        self.gifs.style_target = None;
        match tab {
            Tab::Media => self.open_gif_picker(position, true, window, cx),
            Tab::Stickers => self.open_sticker_tab(position, window, cx),
            Tab::Emoji => self.open_emoji_tab(position, window, cx),
        }
    }
}

pub(super) fn provider_menu(app: &DiscordApp, selected: GifProvider) -> impl IntoElement {
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
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::elevated()))
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
                .hover(|style| style.bg(rgba(theme::selected())))
                .when(active, |this| this.bg(rgba(theme::selected())))
                .text_size(rpx(16.))
                .text_color(rgb(if active {
                    theme::strong()
                } else {
                    theme::muted()
                }))
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
