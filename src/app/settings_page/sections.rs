use gpui::{prelude::*, *};

use super::{Page, SettingsPage, moved};
use crate::{
    app::{DiscordApp, voice::switch},
    discord::gifs::GifProvider,
    theme::{self, rpx},
};

/// The selected page's sections, in sidebar order.
pub(super) fn page(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> Vec<AnyElement> {
    match settings.page {
        Page::Account => account(app, settings),
        Page::GameActivity => vec![game_activity(app, settings)],
        Page::VoiceVideo => super::voice_audio::page(app, settings, cx),
        Page::Appearance => vec![
            theme_section(app, settings, cx),
            appearance(app, cx),
            chat_text(),
        ],
        Page::Accessibility => vec![accessibility(app, settings, cx)],
        Page::Chat => vec![chat_media(app, settings), chat(app, cx)],
        Page::Notifications => vec![notifications(app, settings)],
        Page::LanguageTime => vec![language_time(app, settings, cx)],
        Page::Advanced => vec![advanced(app, settings)],
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
    let email = account.as_ref().and_then(|account| account.email.clone());
    let phone = account.as_ref().and_then(|account| account.phone.clone());
    let email_value = match email {
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

fn theme_section(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> AnyElement {
    let light = theme::is_light();
    let options = [("Dark", false), ("Light", true)]
        .into_iter()
        .map(|(label, choice)| {
            let entity = app.entity.clone();
            (
                label,
                light == choice,
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    entity
                        .update(cx, |app, cx| app.choose_theme(choice, cx))
                        .ok();
                },
            )
        })
        .collect();
    let entity = app.entity.clone();
    section(
        "Theme",
        vec![
            row(
                "Theme",
                Some("Light or dark colors for the whole app."),
                segmented("settings-theme", options),
            ),
            toggle_row(
                app,
                settings,
                "settings-theme-sync",
                "Sync Across Clients",
                "Use the same theme as your Discord account on your other devices.",
                !crate::settings::get(cx).theme_sync_off,
                move |cx, on| {
                    entity.update(cx, |app, cx| app.set_theme_sync(on, cx)).ok();
                },
            ),
        ],
    )
}

fn appearance(_app: &DiscordApp, cx: &App) -> AnyElement {
    let index = theme::zoom_index(cx);
    let percent = (theme::ZOOM_LEVELS[index] * 100.).round() as u32;
    let step = |id: &'static str, icon: &'static str, target: Option<usize>| {
        let enabled = target.is_some();
        theme::icon_button(
            id,
            icon,
            16.,
            if enabled {
                theme::text()
            } else {
                theme::faint()
            },
        )
        .size(rpx(32.))
        .rounded(rpx(8.))
        .bg(rgba(theme::selected()))
        .when_some(target, |this, target| {
            this.on_click(move |_, _, cx| theme::set_zoom(target, cx))
        })
    };
    let last = theme::ZOOM_LEVELS.len() - 1;
    let control = div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .child(step("settings-zoom-out", "minus", index.checked_sub(1)))
        .child(
            div()
                .w(rpx(56.))
                .text_center()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(format!("{percent}%")),
        )
        .child(step(
            "settings-zoom-in",
            "plus",
            (index < last).then_some(index + 1),
        ))
        .when(index != theme::DEFAULT_ZOOM, |this| {
            this.child(button("settings-zoom-reset", "Reset", |_, _, cx| {
                theme::set_zoom(theme::DEFAULT_ZOOM, cx)
            }))
        });
    section(
        "Zoom",
        vec![row(
            "Zoom Level",
            Some("Scales the whole app. Ctrl + and Ctrl − do the same."),
            control.into_any_element(),
        )],
    )
}

fn accessibility(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> AnyElement {
    let on = crate::settings::get(cx).reduce_motion == Some(true);
    let entity = app.entity.clone();
    let toggle = div()
        .id("settings-reduce-motion")
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, _| app.mark_switch_moved("settings-motion"))
                .ok();
            crate::settings::set_reduce_motion(cx, (!on).then_some(true));
        })
        .child(switch(
            "settings-motion",
            on,
            moved(settings, "settings-motion"),
        ));
    section(
        "Motion",
        vec![
            row(
                "Reduced Motion",
                Some("Turns off popup, hover and avatar animations."),
                toggle.into_any_element(),
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
        ],
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
fn toggle_row(
    app: &DiscordApp,
    settings: &SettingsPage,
    id: &'static str,
    label: &'static str,
    description: &'static str,
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
    row(label, Some(description), toggle.into_any_element())
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

fn game_activity(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
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
    section("Activity Privacy", rows)
}

fn chat_media(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    section(
        "Display Images, Videos and Lolcats",
        vec![
            toggle_row(
                app,
                settings,
                "settings-gif-autoplay",
                "Automatically play GIFs",
                "When off, GIFs and animated stickers show a still frame.",
                crate::settings::gif_autoplay(),
                crate::settings::set_gif_autoplay,
            ),
            toggle_row(
                app,
                settings,
                "settings-emoji-animation",
                "Play animated emoji",
                "When off, animated custom emoji show a still frame.",
                crate::settings::emoji_animation(),
                crate::settings::set_emoji_animation,
            ),
            toggle_row(
                app,
                settings,
                "settings-link-previews",
                "Show link previews",
                "Shows embeds for websites linked in chat. Attachments and bot embeds always show.",
                crate::settings::link_previews(),
                crate::settings::set_link_previews,
            ),
        ],
    )
}

fn chat_text() -> AnyElement {
    let size = crate::settings::chat_font_size();
    let step = |id: &'static str, icon: &'static str, target: Option<f32>| {
        theme::icon_button(
            id,
            icon,
            16.,
            if target.is_some() {
                theme::text()
            } else {
                theme::faint()
            },
        )
        .size(rpx(32.))
        .rounded(rpx(8.))
        .bg(rgba(theme::selected()))
        .when_some(target, |this, target| {
            this.on_click(move |_, _, cx| crate::settings::set_chat_font_size(cx, target))
        })
    };
    let control = div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .child(step(
            "settings-font-smaller",
            "minus",
            (size > 12.).then_some(size - 1.),
        ))
        .child(
            div()
                .w(rpx(56.))
                .text_center()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(format!("{size:.0}px")),
        )
        .child(step(
            "settings-font-larger",
            "plus",
            (size < 24.).then_some(size + 1.),
        ))
        .when(size != 16., |this| {
            this.child(button("settings-font-reset", "Reset", |_, _, cx| {
                crate::settings::set_chat_font_size(cx, 16.)
            }))
        });
    section(
        "Chat Text",
        vec![
            row(
                "Chat Font Size",
                Some(
                    "Only message text changes; names, timestamps and the rest of the app keep their size.",
                ),
                control.into_any_element(),
            ),
            div()
                .mt(rpx(8.))
                .p(rpx(16.))
                .rounded(rpx(8.))
                .bg(rgb(theme::bg()))
                .text_size(rpx(size))
                .line_height(rpx(size * 1.375))
                .text_color(rgb(theme::text()))
                .child("Waiting for the day when zcord ships compact mode.")
                .into_any_element(),
        ],
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
                Some("Starts zcord when you sign in to Windows."),
                startup.into_any_element(),
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
                .line_height(rpx(30.))
                .font_weight(FontWeight::SEMIBOLD)
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
                        .text_color(rgb(theme::text()))
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

fn segmented<F>(id: &'static str, options: Vec<(&'static str, bool, F)>) -> AnyElement
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
