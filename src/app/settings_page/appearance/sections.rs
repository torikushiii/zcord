use super::*;

const SPOILER_MODES: [(&str, SpoilerMode); 3] = [
    ("On click", SpoilerMode::OnClick),
    ("On servers I moderate", SpoilerMode::IfModerator),
    ("Always", SpoilerMode::Always),
];

fn spoiler_select(app: &DiscordApp, settings: &SettingsPage, current: SpoilerMode) -> AnyElement {
    let options = SPOILER_MODES
        .iter()
        .map(|&(label, mode)| {
            let entity = app.entity.clone();
            let choose: Rc<dyn Fn(&mut App)> = Rc::new(move |cx| {
                entity
                    .update(cx, |app, cx| {
                        app.change_text_images(|display| display.render_spoilers = mode, cx)
                    })
                    .ok();
            });
            (label, mode == current, choose)
        })
        .collect();
    select_field(
        app,
        "settings-spoilers",
        settings.spoiler_menu_open,
        |page| &mut page.spoiler_menu_open,
        options,
    )
}

pub(super) fn messages(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    let display = app.text_images();
    let avatars = if display.message_display_compact {
        display_toggle(
            app,
            settings,
            "settings-compact-avatars",
            "Show user avatars",
            None,
            display.display_compact_avatars,
            |display, on| display.display_compact_avatars = on,
        )
    } else {
        disabled_row(
            settings,
            "settings-compact-avatars",
            "Show user avatars",
            "To hide avatars, go to Accessibility and select Compact under Chat Message Display.",
            true,
        )
    };
    let favorites = {
        let entity = app.entity.clone();
        toggle_row(
            app,
            settings,
            "settings-favorites-server",
            "Show Favorites server",
            Some("Your Favorites stay saved, even if you toggle this off."),
            app.favorites_visible(),
            move |cx, on| {
                entity
                    .update(cx, |app, cx| app.change_favorites_visible(on, cx))
                    .ok();
            },
        )
    };
    section(
        "Messages",
        vec![
            legend("Show images, videos, and lolcats..."),
            display_toggle(
                app,
                settings,
                "settings-inline-embeds",
                "When posted as links to chat",
                None,
                display.inline_embed_media,
                |display, on| display.inline_embed_media = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-inline-attachments",
                "When uploaded directly to Discord",
                None,
                display.inline_attachment_media,
                |display, on| display.inline_attachment_media = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-render-embeds",
                "Show embeds and link previews",
                None,
                display.render_embeds,
                |display, on| display.render_embeds = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-render-reactions",
                "Show emoji reactions",
                None,
                display.render_reactions,
                |display, on| display.render_reactions = on,
            ),
            row(
                "Show spoiler content",
                None::<&str>,
                spoiler_select(app, settings, display.render_spoilers),
            ),
            display_toggle(
                app,
                settings,
                "settings-thread-sidebar",
                "Open threads in split view",
                None,
                display.use_thread_sidebar,
                |display, on| display.use_thread_sidebar = on,
            ),
            avatars,
            favorites,
            related_accessibility(
                app,
                "settings-related-messages",
                "Adjust message display and the space between message groups",
            ),
        ],
    )
}

pub(super) fn chat_box(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> AnyElement {
    let display = app.text_images();
    let send_button = crate::settings::get(cx).composer_send_button;
    section(
        "Chat Box",
        vec![
            display_toggle(
                app,
                settings,
                "settings-rich-input",
                "Preview emoji, mentions, and markdown syntax as you type",
                None,
                display.use_rich_chat_input,
                |display, on| display.use_rich_chat_input = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-convert-emoticons",
                "Automatically convert emoticons in your messages to emoji",
                Some("For example, when you type :) Discord will convert it to 🙂"),
                display.convert_emoticons,
                |display, on| display.convert_emoticons = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-sticker-suggestions",
                "Show stickers in autocomplete results",
                None,
                display.include_stickers_in_autocomplete,
                |display, on| display.include_stickers_in_autocomplete = on,
            ),
            display_toggle(
                app,
                settings,
                "settings-game-suggestions",
                "Show games in autocomplete results",
                None,
                display.include_game_mentions_in_autocomplete,
                |display, on| display.include_game_mentions_in_autocomplete = on,
            ),
            toggle_row(
                app,
                settings,
                "settings-send-button",
                "Show send message button",
                None,
                send_button,
                |cx, on| {
                    crate::settings::update(cx, |settings| settings.composer_send_button = on);
                    cx.refresh_windows();
                },
            ),
        ],
    )
}

pub(super) fn search(app: &DiscordApp) -> AnyElement {
    let cross = app.text_images().is_cross_dm_search_enabled;
    let options = [
        ("Searches only in the selected DM", false),
        ("Searches across all my DMs", true),
    ]
    .into_iter()
    .map(|(label, value)| {
        let entity = app.entity.clone();
        div()
            .id(("settings-dm-search", u64::from(value)))
            .flex()
            .items_center()
            .gap(rpx(12.))
            .h(rpx(20.))
            .cursor_pointer()
            .text_size(rpx(16.))
            .line_height(rpx(20.))
            .text_color(rgb(theme::text()))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        app.change_text_images(
                            |display| display.is_cross_dm_search_enabled = value,
                            cx,
                        )
                    })
                    .ok();
            })
            .child(radio(cross == value))
            .child(label)
            .into_any_element()
    });
    section(
        "Search",
        vec![
            div()
                .mt(rpx(4.))
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .child("By default, searching in DMs...")
                .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(rpx(16.))
                .pt(rpx(16.))
                .pb(rpx(4.))
                .children(options)
                .into_any_element(),
        ],
    )
}

pub(super) fn streamer(app: &DiscordApp, settings: &SettingsPage) -> AnyElement {
    let current = settings::streamer();
    let option =
        |id: &'static str, label: &'static str, on: bool, set: fn(&mut StreamerMode, bool)| {
            toggle_row(app, settings, id, label, None, on, move |cx, on| {
                let mut streamer = settings::streamer();
                set(&mut streamer, on);
                settings::set_streamer(cx, streamer);
            })
        };
    let mut rows = vec![
        toggle_row(
            app,
            settings,
            "settings-streamer",
            "Enable Streamer Mode",
            Some(
                "Streamer Mode hides your personal info so others watching your stream can't see it.",
            ),
            current.enabled,
            |cx, on| {
                settings::set_streamer(
                    cx,
                    StreamerMode {
                        enabled: on,
                        ..settings::streamer()
                    },
                );
            },
        ),
        option(
            "settings-streamer-auto",
            "Automatically enable Streamer Mode if OBS or XSplit is running",
            current.auto,
            |streamer, on| streamer.auto = on,
        ),
    ];
    rows.push(legend("If Streamer Mode is enabled..."));
    rows.extend([
        option(
            "settings-streamer-personal",
            "Hide my personal information, such as email, connected accounts, and notes",
            current.hide_personal,
            |streamer, on| streamer.hide_personal = on,
        ),
        option(
            "settings-streamer-sounds",
            "Disable all sound effects",
            current.mute_sounds,
            |streamer, on| streamer.mute_sounds = on,
        ),
        option(
            "settings-streamer-notifications",
            "Disable notifications",
            current.mute_notifications,
            |streamer, on| streamer.mute_notifications = on,
        ),
    ]);
    section("Streamer Mode", rows)
}

pub(crate) fn message_display(app: &DiscordApp) -> AnyElement {
    let compact = app.text_images().message_display_compact;
    let options = [("Default", false), ("Compact", true)]
        .into_iter()
        .map(|(label, value)| {
            let entity = app.entity.clone();
            (
                label,
                compact == value,
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    entity
                        .update(cx, |app, cx| {
                            app.change_text_images(
                                |display| display.message_display_compact = value,
                                cx,
                            )
                        })
                        .ok();
                },
            )
        })
        .collect();
    row(
        "Chat Message Display",
        Some(if compact {
            "Compact fits more messages on screen, with names and times inline."
        } else {
            "Default shows avatars and names above each group of messages."
        }),
        segmented("settings-message-display", options),
    )
}
