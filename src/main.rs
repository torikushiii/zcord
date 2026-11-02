#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod assets;
mod credentials;
mod discord;
mod fonts;
mod games;
mod media;
mod notify;
mod quick_switcher;
mod settings;
mod startup;
mod store;
mod text;
mod theme;

use gpui::{
    AppContext, KeyBinding, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions,
    actions, px, size,
};
use gpui_component::Root;

actions!(zcord, [ZoomIn, ZoomOut, ResetZoom]);

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
    notify::init();
    gpui_platform::application()
        .with_assets(assets::AppAssets)
        .run(|cx| {
            gpui_component::init(cx);
            gpui_tokio::init(cx);
            media::images::init(cx);
            media::animated::init(cx);
            theme::init(cx);
            fonts::init(cx);
            settings::init(cx);
            let saved = settings::get(cx);
            settings::set_clock_24h(cx, saved.clock_24h);
            settings::set_reduce_motion(cx, saved.reduce_motion);
            settings::set_gif_autoplay(cx, !saved.gif_autoplay_off);
            settings::set_smooth_scroll(cx, !saved.smooth_scroll_off);
            settings::set_emoji_animation(cx, !saved.emoji_animation_off);
            settings::set_link_previews(cx, !saved.link_previews_off);
            settings::set_notifications(cx, !saved.notifications_off);
            settings::set_notification_sound(cx, !saved.notification_sound_off);
            settings::set_game_activity(cx, saved.share_game_activity);
            settings::set_chat_font_size(cx, saved.chat_font_size.unwrap_or(16.0));
            theme::set_zoom(saved.zoom.unwrap_or(theme::DEFAULT_ZOOM), cx);
            cx.bind_keys([
                KeyBinding::new("secondary-=", ZoomIn, None),
                KeyBinding::new("secondary-+", ZoomIn, None),
                KeyBinding::new("secondary--", ZoomOut, None),
                KeyBinding::new("secondary-0", ResetZoom, None),
                KeyBinding::new("escape", app::JumpToPresent, None),
                KeyBinding::new("shift-escape", app::MarkGuildRead, None),
                KeyBinding::new("secondary-k", app::QuickSwitch, None),
                KeyBinding::new("secondary-t", app::QuickSwitch, None),
                KeyBinding::new("secondary-f", app::FocusSearch, None),
                KeyBinding::new("up", app::ComposerUp, Some("Input")),
                KeyBinding::new("down", app::ComposerDown, Some("Input")),
                KeyBinding::new("tab", app::ComposerTab, Some("Input")),
            ]);
            cx.on_action(|_: &ZoomIn, cx| theme::set_zoom(theme::zoom_index(cx) + 1, cx));
            cx.on_action(|_: &ZoomOut, cx| {
                theme::set_zoom(theme::zoom_index(cx).saturating_sub(1), cx)
            });
            cx.on_action(|_: &ResetZoom, cx| theme::set_zoom(theme::DEFAULT_ZOOM, cx));
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    settings::flush(cx);
                    cx.quit();
                }
            })
            .detach();

            let min_size = size(px(940.), px(560.));
            let window_bounds = settings::initial_window_bounds(min_size, cx)
                .unwrap_or_else(|| WindowBounds::centered(size(px(1440.), px(900.)), cx));
            let options = WindowOptions {
                window_bounds: Some(window_bounds),
                window_min_size: Some(min_size),
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
