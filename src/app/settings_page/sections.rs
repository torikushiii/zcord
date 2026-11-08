use gpui::{prelude::*, *};

use super::{Page, SettingsPage, moved};
use crate::{
    app::{DiscordApp, voice::switch},
    discord::{
        gifs::GifProvider,
        settings_proto::{StickerAnimation, TextImages},
    },
    settings::RoleColors,
    theme::{self, rpx},
};

use super::{
    appearance::{radio, related_setting, select_field},
    marker_slider::{Mark, marker_slider},
};
use std::rc::Rc;

/// The selected page's sections, in sidebar order.
pub(super) fn page(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> Vec<AnyElement> {
    match settings.page {
        Page::Account => account(app, settings),
        Page::GameActivity => super::activity_privacy::page(app, settings, cx),
        Page::RegisteredGames => super::registered_games::page(app, settings, cx),
        Page::VoiceVideo => super::voice_audio::page(app, settings, cx),
        Page::Appearance => super::appearance::page(app, settings, cx),
        Page::Accessibility => vec![
            text_readability(app, settings),
            visual_density(app, cx),
            color_contrast(app, settings),
            reduced_motion(app, settings, cx),
            screen_reader(app, settings),
        ],
        Page::Chat => vec![chat(app, cx)],
        Page::Notifications => vec![notifications(app, settings)],
        Page::LanguageTime => vec![language_time(app, settings, cx)],
        Page::Advanced => vec![advanced(app, settings)],
        Page::Tweaks => vec![
            layout(app, settings),
            chat_tweaks(app, settings),
            animation_tweaks(app, settings),
            privacy_tweaks(app, settings),
        ],
    }
}

fn account(app: &DiscordApp, settings: &SettingsPage) -> Vec<AnyElement> {
    let account = app.store.account();
    let reveal = |id: &'static str, shown: bool, field: fn(&mut SettingsPage) -> &mut bool| {
        let entity = app.entity.clone();
        div()
            .id(id)
            .cursor_pointer()
            .text_color(rgb(theme::link()))
            .hover(|style| style.underline())
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        if let Some(settings) = &mut app.settings_page {
                            let flag = field(settings);
                            *flag = !*flag;
                        }
                        cx.notify();
                    })
                    .ok();
            })
            .child(if shown { "Hide" } else { "Reveal" })
    };
    let hidden = crate::settings::streamer().hides_personal();
    let email = account.as_ref().and_then(|account| account.email.clone());
    let phone = account.as_ref().and_then(|account| account.phone.clone());
    let email_value = match email {
        Some(email) if hidden => value(mask_email(&email)),
        Some(email) => value_with(
            if settings.reveal_email {
                email
            } else {
                mask_email(&email)
            },
            reveal("settings-reveal-email", settings.reveal_email, |settings| {
                &mut settings.reveal_email
            }),
        ),
        None => value("Not added").into_any_element(),
    };
    let phone_value = match phone {
        Some(phone) if hidden => value(mask_phone(&phone)),
        Some(phone) => value_with(
            if settings.reveal_phone {
                phone
            } else {
                mask_phone(&phone)
            },
            reveal("settings-reveal-phone", settings.reveal_phone, |settings| {
                &mut settings.reveal_phone
            }),
        ),
        None => value("Not added").into_any_element(),
    };
    let username = account
        .as_ref()
        .map_or_else(String::new, |account| account.username.clone());
    let mfa = account.as_ref().is_some_and(|account| account.mfa_enabled);
    vec![
        section(
            "Account Info",
            vec![
                row("Username", None::<&str>, value(username)),
                row("Email", None::<&str>, email_value),
                row("Phone Number", None::<&str>, phone_value),
            ],
        ),
        section(
            "Password & Security",
            vec![
                row(
                    "Multi-Factor Authentication",
                    None::<&str>,
                    value(if mfa { "Enabled" } else { "Disabled" }),
                ),
                note(
                    "zcord shows these but can't change them. Use the official Discord app to \
                     change your password, email, phone number or two-factor authentication.",
                ),
            ],
        ),
    ]
}

/// `alice@gmail.com` becomes `*******@gmail.com`, like Discord.
fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((_, domain)) => format!("*******@{domain}"),
        None => "*******".to_owned(),
    }
}

/// Only the last four digits stay visible, like Discord.
fn mask_phone(phone: &str) -> String {
    let digits: Vec<char> = phone.chars().filter(char::is_ascii_digit).collect();
    let last: String = digits[digits.len().saturating_sub(4)..].iter().collect();
    format!("**********{last}")
}

fn zoom(cx: &App) -> AnyElement {
    let marks = theme::ZOOM_LEVELS
        .into_iter()
        .map(|level| Mark::labelled(level * 100., format!("{}", (level * 100.).round() as u32)))
        .collect();
    slider_row(
        "Zoom level",
        "Adjust the size of the interface. You can also change this with ctrl +/-.",
        marker_slider(
            "settings-zoom",
            marks,
            theme::ZOOM_LEVELS[theme::zoom_index(cx)] * 100.,
            Some(theme::ZOOM_LEVELS[theme::DEFAULT_ZOOM] * 100.),
            theme::set_zoom,
        ),
    )
}

pub(super) fn slider_row(
    label: &'static str,
    description: &'static str,
    slider: AnyElement,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .py(rpx(8.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(4.))
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::muted()))
                        .child(description),
                ),
        )
        .child(div().mt(rpx(8.)).child(slider))
        .into_any_element()
}

fn layout(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    section(
        "Layout",
        vec![
            toggle_row(
                app,
                settings,
                "settings-user-panel-extended",
                "Extend User Panel",
                "Stretches the panel with your name and voice controls under the server list too.",
                crate::settings::user_panel_extended(),
                crate::settings::set_user_panel_extended,
            ),
            toggle_row(
                app,
                settings,
                "settings-activity-toggle",
                "Activity Toggle",
                "Adds a button before your mic that turns sharing your activity on or off for \
             your whole account.",
                crate::settings::activity_toggle(),
                crate::settings::set_activity_toggle,
            ),
        ],
    )
}

fn chat_tweaks(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    section(
        "Chat",
        vec![
            toggle_row(
                app,
                settings,
                "settings-quiet-replies",
                "Quiet Replies",
                "Replies start with the author's ping turned off. Click @ in the reply bar to ping \
                 them anyway.",
                crate::settings::quiet_replies(),
                crate::settings::set_quiet_replies,
            ),
            toggle_row(
                app,
                settings,
                "settings-mention-avatars",
                "Mention Avatars",
                "Shows the person's avatar in user mentions and the role's icon in role mentions.",
                crate::settings::mention_avatars(),
                crate::settings::set_mention_avatars,
            ),
            toggle_row(
                app,
                settings,
                "settings-who-reacted",
                "Who Reacted",
                "Shows the avatars of the first few people beside each reaction's count.",
                crate::settings::who_reacted(),
                crate::settings::set_who_reacted,
            ),
            toggle_row(
                app,
                settings,
                "settings-rich-typing",
                "Typing Avatars",
                "Shows avatars and role colors for the people in the typing indicator.",
                crate::settings::rich_typing(),
                crate::settings::set_rich_typing,
            ),
        ],
    )
}

fn animation_tweaks(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    section(
        "Animation",
        vec![toggle_row(
            app,
            settings,
            "settings-always-animate",
            "Always Animate",
            "Plays animated avatars, decorations, nameplates, server icons and display name \
             effects all the time instead of only when you hover them.",
            crate::settings::always_animate(),
            crate::settings::set_always_animate,
        )],
    )
}

fn privacy_tweaks(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    section(
        "Privacy",
        vec![toggle_row(
            app,
            settings,
            "settings-typing-hidden",
            "Hide My Typing",
            "Others won't see that you're typing. You still see when they type.",
            crate::settings::typing_hidden(),
            crate::settings::set_typing_hidden,
        )],
    )
}

fn text_readability(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    let mut rows = vec![chat_text()];
    rows.extend([
        toggle_row(
            app,
            settings,
            "settings-underline-links",
            "Always underline links",
            "Make links stand out more.",
            crate::settings::underline_links(),
            crate::settings::set_underline_links,
        ),
        toggle_row(
            app,
            settings,
            "settings-display-name-styles",
            "Display Name Styles",
            "Enable display name styles — including font, effect, and color — across zcord.",
            crate::settings::display_name_styles(),
            crate::settings::set_display_name_styles,
        ),
    ]);
    section("Text Readability", rows)
}

fn visual_density(app: &DiscordApp, cx: &App) -> AnyElement {
    let compact = app.text_images().message_display_compact;
    let marks = crate::settings::GROUP_SPACINGS
        .into_iter()
        .map(|spacing| Mark::labelled(spacing as f32, format!("{spacing}px")))
        .collect();
    section(
        "Visual Density",
        vec![
            super::appearance::message_display(app),
            slider_row(
                "Space Between Message Groups",
                "Adjust the spacing between message groups.",
                marker_slider(
                    "settings-group-spacing",
                    marks,
                    crate::settings::message_group_spacing(compact) as f32,
                    Some(if compact { 0. } else { 16. }),
                    |index, cx| {
                        crate::settings::set_message_group_spacing(
                            cx,
                            Some(crate::settings::GROUP_SPACINGS[index]),
                        )
                    },
                ),
            ),
            zoom(cx),
        ],
    )
}

fn color_contrast(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    let current = crate::settings::role_colors();
    let options = RoleColors::ALL
        .into_iter()
        .map(|mode| {
            let choose: Rc<dyn Fn(&mut App)> =
                Rc::new(move |cx| crate::settings::set_role_colors(cx, mode));
            (mode.label(), current == mode, choose)
        })
        .collect();
    section(
        "Color & Contrast",
        vec![
            row(
                "Role Colors",
                Some("Choose how you would like role colors to be displayed."),
                select_field(
                    app,
                    "settings-role-colors",
                    settings.role_colors_menu_open,
                    |page| &mut page.role_colors_menu_open,
                    options,
                ),
            ),
            toggle_row(
                app,
                settings,
                "settings-switch-indicators",
                "Show on/off indicators",
                "On forms, menus and settings, on/off toggles will have icons.",
                crate::settings::switch_indicators(),
                crate::settings::set_switch_indicators,
            ),
            related_setting(
                app,
                "settings-related-appearance",
                Page::Appearance,
                "palette",
                "Appearance",
                "Change your app theme",
            ),
        ],
    )
}

fn display_toggle(
    app: &DiscordApp,
    settings: &SettingsPage,
    id: &'static str,
    label: &'static str,
    description: Option<&'static str>,
    on: bool,
    set: fn(&mut TextImages, bool),
) -> AnyElement {
    let entity = app.entity.clone();
    toggle_row(app, settings, id, label, description, on, move |cx, on| {
        entity
            .update(cx, |app, cx| {
                app.change_text_images(|display| set(display, on), cx)
            })
            .ok();
    })
}

fn reduced_motion(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> AnyElement {
    let synced = crate::settings::reduce_motion_sync(cx);
    let on = cx.reduce_motion();
    let entity = app.entity.clone();
    let reduce = div()
        .id("settings-reduce-motion")
        .when(synced, |this| this.opacity(0.5))
        .when(!synced, |this| {
            this.cursor_pointer().on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, _| app.mark_switch_moved("settings-motion"))
                    .ok();
                crate::settings::set_reduce_motion(cx, Some(!on));
            })
        })
        .child(switch(
            "settings-motion",
            on,
            moved(settings, "settings-motion"),
        ));
    let display = app.text_images();
    let durations = crate::settings::NOTIFICATION_SECONDS
        .map(|seconds| match seconds {
            3 | 5 | 10 | 15 | 20 => Mark::labelled(seconds as f32, format!("{seconds}s")),
            _ => Mark::stop(seconds as f32),
        })
        .collect();
    let stickers = [
        ("Always animate", None, StickerAnimation::Always),
        (
            "Animate on interaction",
            Some("Stickers animate while the pointer is over them."),
            StickerAnimation::OnInteraction,
        ),
        ("Never animate", None, StickerAnimation::Never),
    ]
    .into_iter()
    .map(|(label, note, mode)| {
        let entity = app.entity.clone();
        div()
            .id(("settings-sticker-animation", mode as u64))
            .flex()
            .gap(rpx(12.))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        app.change_text_images(|display| display.animate_stickers = mode, cx)
                    })
                    .ok();
            })
            .child(radio(display.animate_stickers == mode))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rpx(4.))
                    .child(
                        div()
                            .text_size(rpx(16.))
                            .line_height(rpx(20.))
                            .text_color(rgb(theme::text()))
                            .child(label),
                    )
                    .children(note.map(|note| {
                        div()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .text_color(rgb(theme::muted()))
                            .child(note)
                    })),
            )
    });
    section(
        "Reduced Motion",
        vec![
            row(
                "Enable Reduced Motion",
                Some(
                    "Reduce the amount and intensity of animations, hover effects, and other \
                     moving effects.",
                ),
                reduce.into_any_element(),
            ),
            toggle_row(
                app,
                settings,
                "settings-motion-sync",
                "Sync with computer setting",
                None,
                synced,
                crate::settings::set_reduce_motion_sync,
            ),
            toggle_row(
                app,
                settings,
                "settings-smooth-scroll",
                "Smooth Scrolling",
                "Mouse-wheel scrolling glides like a browser. Reduced Motion turns it off.",
                crate::settings::smooth_scroll(),
                crate::settings::set_smooth_scroll,
            ),
            slider_row(
                "Minimum notification duration",
                "Keep pop-up notifications on screen for at least this long. Notifications that \
                 already stay longer are unaffected.",
                marker_slider(
                    "settings-notification-duration",
                    durations,
                    crate::settings::notification_seconds() as f32,
                    Some(crate::settings::DEFAULT_NOTIFICATION_SECONDS as f32),
                    |index, cx| {
                        crate::settings::set_notification_seconds(
                            cx,
                            crate::settings::NOTIFICATION_SECONDS.start() + index as u32,
                        )
                    },
                ),
            ),
            display_toggle(
                app,
                settings,
                "settings-gif-autoplay",
                "Play GIFs when zcord is focused",
                None,
                display.gif_auto_play,
                |display, on| display.gif_auto_play = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-emoji-animation",
                "Play animated emoji",
                None,
                display.animate_emoji,
                |display, on| display.animate_emoji = on,
            ),
            div()
                .flex()
                .flex_col()
                .gap(rpx(16.))
                .py(rpx(12.))
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .child("Play sticker animations"),
                )
                .children(stickers)
                .into_any_element(),
        ],
    )
}

fn screen_reader(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    section(
        "Audio & Screen Reader",
        vec![display_toggle(
            app,
            settings,
            "settings-image-descriptions",
            "Show image descriptions",
            Some("Image descriptions are used to describe images for screen readers."),
            app.text_images().view_image_descriptions,
            |display, on| display.view_image_descriptions = on,
        )],
    )
}

fn chat(_app: &DiscordApp, cx: &App) -> AnyElement {
    let current = crate::settings::get(cx).gif_provider;
    let options = GifProvider::ALL
        .into_iter()
        .map(|provider| {
            (
                provider.label(),
                current == provider,
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    crate::settings::update(cx, |settings| settings.gif_provider = provider);
                    cx.refresh_windows();
                },
            )
        })
        .collect();
    section(
        "GIFs",
        vec![row(
            "GIF Provider",
            Some("Where the GIF picker searches. You can also switch it inside the picker."),
            segmented("settings-gif-provider", options),
        )],
    )
}

/// A labelled switch row whose click flips `on` through `set`.
pub(super) fn toggle_row(
    app: &DiscordApp,
    settings: &SettingsPage,
    id: &'static str,
    label: &'static str,
    description: impl Into<Option<&'static str>>,
    on: bool,
    set: impl Fn(&mut App, bool) + 'static,
) -> AnyElement {
    let entity = app.entity.clone();
    let toggle = div()
        .id(id)
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity.update(cx, |app, _| app.mark_switch_moved(id)).ok();
            set(cx, !on);
        })
        .child(switch(id, on, moved(settings, id)));
    row(label, description.into(), toggle.into_any_element())
}

fn notifications(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    section(
        "Notifications",
        vec![
            toggle_row(
                app,
                settings,
                "settings-notifications",
                "Enable desktop notifications",
                "Shows a Windows notification for new messages while zcord isn't focused.",
                crate::settings::notifications(),
                crate::settings::set_notifications,
            ),
            toggle_row(
                app,
                settings,
                "settings-notification-sound",
                "Play notification sound",
                "Plays a sound for new messages outside the channel you have open.",
                crate::settings::notification_sound(),
                crate::settings::set_notification_sound,
            ),
            note("Follows your Discord notification settings for each server and channel."),
        ],
    )
}

pub(super) fn detected_games(app: &DiscordApp, settings: &SettingsPage) -> Vec<AnyElement> {
    let on = crate::settings::game_activity();
    let entity = app.entity.clone();
    let mut rows = vec![toggle_row(
        app,
        settings,
        "settings-share-game-activity",
        "Share your detected games",
        "Scans your running programs and tells Discord which game you play. Programs using \
         Rich Presence can also set your status.",
        on,
        move |cx, on| {
            crate::settings::set_game_activity(cx, on);
            entity
                .update(cx, |app, cx| {
                    app.update_game_activity();
                    cx.notify();
                })
                .ok();
        },
    )];
    if on {
        let status = if app
            .session
            .as_ref()
            .is_some_and(|session| !session.shows_current_game())
        {
            "Display current activity is turned off in your Discord account".to_owned()
        } else if let Some(game) = &app.current_game {
            format!("Sharing: {game}")
        } else {
            "No game detected".to_owned()
        };
        rows.push(note(status));
    }
    rows
}

const CHAT_FONT_SIZES: [f32; 7] = [12., 14., 15., 16., 18., 20., 24.];

fn chat_text() -> AnyElement {
    let marks = CHAT_FONT_SIZES
        .into_iter()
        .map(|size| Mark::labelled(size, format!("{size}px")))
        .collect();
    slider_row(
        "Text size in chat",
        "Adjust the size of the chat font.",
        marker_slider(
            "settings-chat-font",
            marks,
            crate::settings::chat_font_size(),
            Some(16.),
            |index, cx| crate::settings::set_chat_font_size(cx, CHAT_FONT_SIZES[index]),
        ),
    )
}

fn advanced(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    let startup = {
        let entity = app.entity.clone();
        let on = settings.launch_on_startup;
        div()
            .id("settings-startup")
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.toggle_launch_on_startup(cx))
                    .ok();
            })
            .child(switch(
                "settings-startup",
                on,
                moved(settings, "settings-startup"),
            ))
    };
    let clear = {
        let entity = app.entity.clone();
        button("settings-clear-cache", "Clear Cache", move |_, _, cx| {
            entity.update(cx, |app, cx| app.clear_image_cache(cx)).ok();
        })
    };
    let open_folder = button("settings-data-folder", "Open Folder", |_, _, cx| {
        if let Some(dir) = crate::settings::data_dir() {
            cx.open_with_system(&dir);
        }
    });
    section(
        "Windows",
        vec![
            row(
                "Launch zcord when Windows starts",
                Some(if cfg!(windows) {
                    "Starts zcord in the tray when you sign in to Windows."
                } else {
                    "Starts zcord when you sign in to Windows."
                }),
                startup.into_any_element(),
            ),
            #[cfg(windows)]
            toggle_row(
                app,
                settings,
                "settings-close-to-tray",
                "Close to system tray",
                "Closing the window keeps zcord running in the tray so notifications keep arriving.",
                crate::settings::close_to_tray(),
                crate::settings::set_close_to_tray,
            ),
            row(
                "Image Cache",
                Some(settings.cache_status.clone().unwrap_or_else(|| {
                    "Frees disk space used by downloaded images and videos.".into()
                })),
                clear.into_any_element(),
            ),
            row(
                "Data Folder",
                Some("Where zcord keeps its settings."),
                open_folder.into_any_element(),
            ),
        ],
    )
}

fn language_time(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> AnyElement {
    let on = crate::settings::get(cx).clock_24h;
    let entity = app.entity.clone();
    let toggle = div()
        .id("settings-clock")
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, _| app.mark_switch_moved("settings-clock"))
                .ok();
            crate::settings::set_clock_24h(cx, !on);
        })
        .child(switch(
            "settings-clock",
            on,
            moved(settings, "settings-clock"),
        ));
    section(
        "Time",
        vec![row(
            "24-Hour Clock",
            Some(if on {
                "Times show like 13:45."
            } else {
                "Times show like 1:45 PM."
            }),
            toggle.into_any_element(),
        )],
    )
}

pub(super) fn section(title: &'static str, rows: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .pb(rpx(40.))
        .mb(rpx(40.))
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(
            div()
                .mb(rpx(16.))
                .text_size(rpx(24.))
                .line_height(rpx(32.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .child(title),
        )
        .children(rows)
        .into_any_element()
}

pub(super) fn row(
    label: &'static str,
    description: Option<impl Into<SharedString>>,
    control: AnyElement,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rpx(24.))
        .min_h(rpx(56.))
        .py(rpx(12.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(4.))
                .min_w_0()
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .child(label),
                )
                .children(description.map(|description| {
                    let description: SharedString = description.into();
                    div()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::muted()))
                        .child(description)
                })),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

pub(super) fn note(text: impl Into<SharedString>) -> AnyElement {
    div()
        .mt(rpx(8.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(theme::muted()))
        .child(text.into())
        .into_any_element()
}

fn value(text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .text_color(rgb(theme::strong()))
        .child(text.into())
        .into_any_element()
}

fn value_with(text: String, action: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(rpx(6.))
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .text_color(rgb(theme::strong()))
        .child(text)
        .child(action)
        .into_any_element()
}

fn button(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(rpx(32.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(rgba(theme::selected()))
        .hover(|style| style.bg(rgb(theme::placeholder_strong())))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .on_click(on_click)
        .child(label)
}

type OptionClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub(super) fn segmented<F>(id: &'static str, options: Vec<(&'static str, bool, F)>) -> AnyElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    div()
        .flex()
        .p(rpx(2.))
        .gap(rpx(2.))
        .rounded(rpx(8.))
        .bg(rgb(theme::bg()))
        .border_1()
        .border_color(rgba(theme::border()))
        .children(
            options
                .into_iter()
                .enumerate()
                .map(|(index, (label, selected, on_click))| {
                    let on_click: OptionClick = Box::new(on_click);
                    div()
                        .id((id, index as u64))
                        .flex()
                        .items_center()
                        .h(rpx(28.))
                        .px(rpx(12.))
                        .rounded(rpx(6.))
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .map(|this| {
                            if selected {
                                this.bg(rgba(theme::selected()))
                                    .text_color(rgb(theme::strong()))
                            } else {
                                this.cursor_pointer()
                                    .text_color(rgb(theme::muted()))
                                    .hover(|style| style.text_color(rgb(theme::text())))
                                    .on_click(on_click)
                            }
                        })
                        .child(label)
                }),
        )
        .into_any_element()
}
