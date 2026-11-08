use super::members::connected_title;
use super::*;

pub(crate) fn panel(app: &DiscordApp, cx: &App) -> Option<impl IntoElement + use<>> {
    let call = app.voice.call.as_ref()?;
    let guild = app
        .store
        .guilds
        .iter()
        .find(|guild| guild.id == call.guild_id);
    let channel = guild.and_then(|guild| guild.channel(call.channel_id));
    let place = match (channel, guild) {
        (Some(channel), Some(guild)) => format!("{} / {}", channel.name, guild.name),
        _ if call.guild_id == 0 => app.store.private_channel_name(call.channel_id),
        _ => String::new(),
    };
    let (status, color) = if call.connected {
        ("Voice Connected", theme::GREEN)
    } else {
        ("Connecting…", theme::YELLOW)
    };
    let details_open = app
        .voice
        .menu
        .as_ref()
        .is_some_and(|menu| menu.kind == MenuKind::Details && !menu.closing);
    let title = if call.connected {
        connected_title(
            app.voice.status_hovered || details_open,
            app.voice.status_hovered_once,
            color,
        )
    } else {
        div().child(status).into_any_element()
    };
    let ping: SharedString = match call.last_ping() {
        Some(ms) => format!("{ms} ms").into(),
        None if call.connected => "Measuring ping…".into(),
        None => "Connecting to voice…".into(),
    };
    let entity = app.entity.clone();
    let header = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .id("voice-signal")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(32.))
                .rounded(rpx(8.))
                .bg(rgba(color << 8 | 0x14))
                .child(theme::icon("rss", 18., color))
                .map(|this| with_tooltip(this, app, Control::Signal, ping)),
        )
        .child(
            div()
                .id("voice-status")
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .when(call.connected, |this| {
                    let hover = app.entity.clone();
                    let open = app.entity.clone();
                    this.cursor_pointer()
                        .on_hover(move |hovered, _, cx| {
                            let hovered = *hovered;
                            hover
                                .update(cx, |app, cx| {
                                    app.voice.status_hovered = hovered;
                                    app.voice.status_hovered_once = true;
                                    app.views.channel_list.update(cx, |_, cx| cx.notify());
                                })
                                .ok();
                        })
                        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                            cx.stop_propagation();
                            let position = event.position;
                            open.update(cx, |app, cx| {
                                app.toggle_menu(MenuKind::Details, position, cx)
                            })
                            .ok();
                        })
                })
                .child(
                    div()
                        .mt(rpx(-1.))
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(color))
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::muted()))
                        .child(place),
                ),
        )
        .child({
            let entity = app.entity.clone();
            let color = if app.voice.noise_suppression {
                theme::text()
            } else {
                theme::faint()
            };
            let menu_open = app
                .voice
                .menu
                .as_ref()
                .is_some_and(|menu| menu.kind == MenuKind::NoiseSuppression && !menu.closing);
            theme::icon_button("voice-noise", "audio-lines", 20., color)
                .when(!menu_open, |this| {
                    with_tooltip(
                        this,
                        app,
                        Control::NoiseSuppression,
                        "Noise Suppression powered by RNNoise",
                    )
                })
                .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                    cx.stop_propagation();
                    let position = event.position;
                    entity
                        .update(cx, |app, cx| {
                            app.toggle_menu(MenuKind::NoiseSuppression, position, cx)
                        })
                        .ok();
                })
        })
        .child(
            div()
                .id("voice-disconnect")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(28.))
                .rounded(rpx(4.))
                .cursor_pointer()
                .hover(|style| style.bg(rgba(theme::selected())))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |app, cx| app.leave_voice(cx)).ok();
                })
                .map(|this| with_tooltip(this, app, Control::Disconnect, "Disconnect"))
                // Lucide's handset tilted down into Discord's hang-up glyph.
                .child(
                    theme::icon("phone", 20., theme::text()).with_transformation(
                        Transformation::rotate(radians(std::f32::consts::FRAC_PI_4 * 3.)),
                    ),
                ),
        );
    let action = |id: &'static str, icon: &'static str| {
        div()
            .id(id)
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .h(rpx(32.))
            .rounded(rpx(8.))
            .bg(rgba(theme::hover()))
            .child(theme::icon(icon, 20., theme::text()))
    };
    let sharing = app.voice.sharing != Sharing::Off;
    let share = app.entity.clone();
    let stream_menu_open = app
        .voice
        .menu
        .as_ref()
        .is_some_and(|menu| menu.kind == MenuKind::Stream && !menu.closing);
    let screen = action("voice-screen", "monitor-up")
        .cursor_pointer()
        .when(sharing, |this| this.bg(rgb(theme::GREEN)))
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            cx.stop_propagation();
            let position = event.position;
            share
                .update(cx, |app, cx| {
                    if app.voice.sharing == Sharing::Off {
                        app.screen_button(cx);
                    } else {
                        app.toggle_menu(MenuKind::Stream, position, cx);
                    }
                })
                .ok();
        })
        .when(!stream_menu_open, |this| {
            with_tooltip(
                this,
                app,
                Control::Screen,
                if sharing {
                    "Stream Settings"
                } else {
                    "Share Your Screen"
                },
            )
        });
    let actions = div()
        .flex()
        .gap_2()
        .child(unavailable(action("voice-camera", "video-off"), "Camera"))
        .child(screen)
        .child(unavailable(
            action("voice-activities", "shapes"),
            "Activities",
        ))
        .child(unavailable(
            action("voice-soundboard", "megaphone"),
            "Soundboard",
        ));
    let stream = sharing
        .then(|| share_panel::render(app, call.guild_id, cx))
        .flatten();
    Some(
        div()
            .flex()
            .flex_col()
            .bg(rgb(theme::bg()))
            .children(stream)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p(rpx(8.))
                    .child(header)
                    .child(actions),
            ),
    )
}

fn with_tooltip(
    element: Stateful<Div>,
    app: &DiscordApp,
    control: Control,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    element
        .relative()
        .on_hover(move |hovered, _, cx| {
            let hovered = *hovered;
            entity
                .update(cx, |app, cx| {
                    let voice = &mut app.voice;
                    if hovered {
                        voice.hovered_control = Some(control);
                    } else if voice.hovered_control == Some(control) {
                        voice.hovered_control = None;
                    }
                    app.views.channel_list.update(cx, |_, cx| cx.notify());
                })
                .ok();
        })
        .when(app.voice.hovered_control == Some(control), |this| {
            this.child(theme::tooltip_above(label))
        })
}

/// Discord's voice controls that zcord doesn't implement yet: shown for familiarity, with a
/// tooltip saying so instead of doing nothing silently.
pub(crate) fn unavailable(button: Stateful<Div>, feature: &'static str) -> Stateful<Div> {
    button.relative().child(theme::hover_tooltip(
        SharedString::from(format!("unavailable-{feature}")),
        format!("{feature} isn't available in zcord yet"),
    ))
}
