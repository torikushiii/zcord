#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod assets;
mod channel_list;
mod chat;
mod data;
mod discord;
mod guild_rail;
mod login;
mod member_list;
mod theme;
mod titlebar;
mod token_store;

use gpui::{
    App, AppContext, Global, KeyBinding, TitlebarOptions, WindowBounds, WindowDecorations,
    WindowOptions, actions, px, size,
};
use gpui_component::{Root, Theme, ThemeMode};

actions!(zcord, [ZoomIn, ZoomOut, ResetZoom]);

struct Zoom(usize);

impl Global for Zoom {}

const DEFAULT_ZOOM: usize = 5;

fn set_zoom(index: usize, cx: &mut App) {
    let index = index.min(theme::ZOOM_LEVELS.len() - 1);
    cx.set_global(Zoom(index));
    Theme::global_mut(cx).font_size = px(theme::BASE_FONT_SIZE * theme::ZOOM_LEVELS[index]);
    cx.refresh_windows();
}

fn zoom_index(cx: &App) -> usize {
    cx.try_global::<Zoom>().map_or(DEFAULT_ZOOM, |zoom| zoom.0)
}

fn main() {
    env_logger::Builder::from_default_env()
        .filter_module("tungstenite", log::LevelFilter::Off)
        .filter_module("tokio_tungstenite", log::LevelFilter::Off)
        .filter_module("reqwest", log::LevelFilter::Off)
        .filter_module("hyper", log::LevelFilter::Off)
        .filter_module("hyper_util", log::LevelFilter::Off)
        .filter_module("h2", log::LevelFilter::Off)
        .filter_module("rustls", log::LevelFilter::Off)
        .filter_module("keyring", log::LevelFilter::Off)
        .init();
    gpui_platform::application()
        .with_assets(assets::AppAssets)
        .run(|cx| {
            gpui_component::init(cx);
            gpui_tokio::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            set_zoom(DEFAULT_ZOOM, cx);
            cx.bind_keys([
                KeyBinding::new("secondary-=", ZoomIn, None),
                KeyBinding::new("secondary-+", ZoomIn, None),
                KeyBinding::new("secondary--", ZoomOut, None),
                KeyBinding::new("secondary-0", ResetZoom, None),
            ]);
            cx.on_action(|_: &ZoomIn, cx| set_zoom(zoom_index(cx) + 1, cx));
            cx.on_action(|_: &ZoomOut, cx| set_zoom(zoom_index(cx).saturating_sub(1), cx));
            cx.on_action(|_: &ResetZoom, cx| set_zoom(DEFAULT_ZOOM, cx));
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1440.), px(900.)), cx)),
                window_min_size: Some(size(px(940.), px(560.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("zcord".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                window_decorations: cfg!(target_os = "linux").then_some(WindowDecorations::Client),
                ..Default::default()
            };

            cx.spawn(async move |cx| {
                cx.open_window(options, |window, cx| {
                    let view = cx.new(|cx| app::DiscordApp::new(window, cx));
                    cx.new(|cx| Root::new(view, window, cx))
                })
                .expect("Failed to open the Discord window");
            })
            .detach();
        });
}
