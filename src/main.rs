#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod assets;
mod credentials;
use zcord_discord as discord;
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
mod tray;

use gpui::{
    AppContext, KeyBinding, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions,
    actions, px, size,
};
use gpui_component::Root;

actions!(zcord, [ZoomIn, ZoomOut, ResetZoom]);

fn main() {
    discord::set_identity_source(settings::discord_identity);
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
    #[cfg(windows)]
    if tray::activate_existing_instance() {
        return;
    }
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
            #[cfg(windows)]
            tray::init(cx);
            let saved = settings::get(cx);
            settings::set_clock_24h(cx, saved.clock_24h);
            settings::apply_reduce_motion(cx);
            settings::set_underline_links(cx, saved.underline_links);
            settings::set_quiet_replies(cx, saved.quiet_replies);
            settings::set_mention_avatars(cx, saved.mention_avatars);
            settings::set_who_reacted(cx, saved.who_reacted);
            settings::set_rich_typing(cx, saved.rich_typing);
            settings::set_display_name_styles(cx, !saved.display_name_styles_off);
            settings::set_switch_indicators(cx, saved.switch_indicators);
            settings::set_message_group_spacing(cx, saved.message_group_spacing);
            settings::set_role_colors(cx, saved.role_colors);
            settings::set_notification_seconds(
                cx,
                saved
                    .notification_seconds
                    .unwrap_or(settings::DEFAULT_NOTIFICATION_SECONDS),
            );
            settings::set_gif_autoplay(cx, !saved.gif_autoplay_off);
            settings::set_smooth_scroll(cx, !saved.smooth_scroll_off);
            settings::set_emoji_animation(cx, !saved.emoji_animation_off);
            settings::set_link_previews(cx, !saved.link_previews_off);
            settings::set_notifications(cx, !saved.notifications_off);
            settings::set_notification_sound(cx, !saved.notification_sound_off);
            #[cfg(windows)]
            settings::set_close_to_tray(cx, !saved.close_to_tray_off);
            settings::set_game_activity(cx, saved.share_game_activity);
            settings::set_user_panel_extended(cx, !saved.user_panel_compact);
            settings::set_typing_hidden(cx, saved.typing_hidden);
            settings::set_always_animate(cx, saved.always_animate);
            settings::set_activity_toggle(cx, saved.activity_toggle);
            settings::set_chat_font_size(cx, saved.chat_font_size.unwrap_or(16.0));
            settings::set_profile_theme_sync(cx, saved.profile_theme_sync);
            settings::set_streamer(cx, saved.streamer);
            settings::watch_broadcasters(cx);
            app::apply_saved_theme(cx);
            theme::set_zoom(saved.zoom.unwrap_or(theme::DEFAULT_ZOOM), cx);
            cx.bind_keys([
                KeyBinding::new("secondary-=", ZoomIn, None),
                KeyBinding::new("secondary-+", ZoomIn, None),
                KeyBinding::new("secondary--", ZoomOut, None),
                KeyBinding::new("secondary-0", ResetZoom, None),
                KeyBinding::new("escape", app::JumpToPresent, None),
                KeyBinding::new("shift-pagedown", app::JumpToPresent, None),
                KeyBinding::new("pageup", app::ScrollPageUp, None),
                KeyBinding::new("pagedown", app::ScrollPageDown, None),
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
                    games::registry::flush();
                    cx.quit();
                }
            })
            .detach();

            let min_size = size(px(940.), px(560.));
            let window_bounds = settings::initial_window_bounds(min_size, cx)
                .unwrap_or_else(|| WindowBounds::centered(size(px(1440.), px(900.)), cx));
            #[cfg(windows)]
            let start_minimized =
                tray::available(cx) && std::env::args().any(|arg| arg == "--minimized");
            #[cfg(windows)]
            if start_minimized {
                tray::start_hidden(matches!(
                    window_bounds,
                    WindowBounds::Maximized(_) | WindowBounds::Fullscreen(_)
                ));
            }
            let options = WindowOptions {
                window_bounds: Some(window_bounds),
                window_min_size: Some(min_size),
                titlebar: Some(TitlebarOptions {
                    title: Some("zcord".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                window_decorations: cfg!(target_os = "linux").then_some(WindowDecorations::Client),
                #[cfg(windows)]
                show: !start_minimized,
                ..Default::default()
            };

            cx.spawn(async move |cx| {
                let handle = cx
                    .open_window(options, |window, cx| {
                        #[cfg(windows)]
                        window.on_window_should_close(cx, |window, cx| {
                            if crate::settings::close_to_tray() && crate::tray::available(cx) {
                                crate::settings::flush(cx);
                                crate::tray::hide(window);
                                false
                            } else {
                                true
                            }
                        });
                        let view = cx.new(|cx| app::AppShell::new(window, cx));
                        cx.new(|cx| Root::new(view, window, cx))
                    })
                    .expect("Failed to open the Discord window");
                #[cfg(windows)]
                cx.update(|cx| tray::set_main_window(cx, handle.into()));
                #[cfg(not(windows))]
                let _ = handle;
            })
            .detach();
        });
}
