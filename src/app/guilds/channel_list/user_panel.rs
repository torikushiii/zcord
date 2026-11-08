use super::*;

const CURRENT_GAME_ICON: f32 = 32.;
const SHARING_CARD_WIDTH: f32 = 240.;
const SHARING_CARD_ANCHOR: f32 = 12.;
const SUBTITLE_HEIGHT: f32 = 13.;
const STATUS_EMOJI: f32 = 14.;
pub(crate) const USER_PANEL_HEIGHT: f32 = 56.;
const MUTED_GLOW: u32 = 0xD22D391F;
const ACCOUNT_AVATAR_SIZE: f32 = 32.;

fn account_subtitle(
    user: &User,
    hovered: bool,
    animate: bool,
    activities: Option<Div>,
    cx: &App,
) -> AnyElement {
    let line = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(4.))
            .min_w_0()
            .h(rpx(SUBTITLE_HEIGHT))
    };
    let status = user
        .custom_status
        .as_ref()
        .filter(|status| user.status != Status::Offline && !status.is_expired(SystemTime::now()));
    let status_line = match status {
        None => line().child(div().truncate().child(status_label(user.status))),
        Some(status) => {
            let emoji = status.emoji.as_ref().and_then(|emoji| {
                let source: ImageSource = match emoji {
                    StatusEmoji::Unicode(sequence) => {
                        emoji::image(sequence, theme::image_px(STATUS_EMOJI))?.into()
                    }
                    StatusEmoji::Custom { id, .. } => images::resolve(
                        &ImageSpec::Emoji {
                            id: *id,
                            animated: false,
                        },
                        theme::image_px(STATUS_EMOJI),
                        cx,
                    )?
                    .into(),
                };
                Some(
                    img(source)
                        .debug_selector(|| "account-status-emoji".into())
                        .size(rpx(STATUS_EMOJI))
                        .flex_none(),
                )
            });
            line()
                .children(activities)
                .children(emoji)
                .children(status.text.as_ref().map(|text| {
                    div()
                        .min_w_0()
                        .truncate()
                        .child(label(("self-status", user.id), text))
                }))
        }
    };
    let hovered = hovered && !user.username.is_empty();
    let lines = div()
        .absolute()
        .left_0()
        .right_0()
        .flex()
        .flex_col()
        .child(status_line)
        .child(
            line().child(
                div()
                    .truncate()
                    .child(crate::settings::shown_username(&user.username)),
            ),
        );
    let lines = if animate {
        let animation = Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint());
        lines
            .with_animation(
                ("account-subtitle", u64::from(hovered)),
                animation,
                move |this, delta| {
                    let shown = if hovered { delta } else { 1. - delta };
                    this.top(rpx(-SUBTITLE_HEIGHT * shown))
                },
            )
            .into_any_element()
    } else {
        lines.top_0().into_any_element()
    };
    div()
        .debug_selector(|| "account-subtitle".into())
        .relative()
        .h(rpx(SUBTITLE_HEIGHT))
        .min_w_0()
        .overflow_hidden()
        .text_size(rpx(12.))
        .line_height(rpx(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::muted()))
        .child(lines)
        .into_any_element()
}

pub(super) fn account_identity(
    user: &User,
    hovered: bool,
    animate_subtitle: bool,
    compact: bool,
    activities: Option<Div>,
    cx: &App,
) -> Div {
    div()
        .debug_selector(|| "account-identity".into())
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(rpx(8.))
        .px(rpx(4.))
        .ml(rpx(-4.))
        .py(rpx(5.))
        .rounded(rpx(4.))
        .child(theme::user_avatar(
            Some(user),
            user.id,
            if compact {
                ACCOUNT_AVATAR_SIZE * 0.9
            } else {
                ACCOUNT_AVATAR_SIZE
            },
            Some((user.status, theme::elevated())),
            hovered,
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .debug_selector(|| "account-name".into())
                        .h(rpx(18.))
                        .mt(rpx(-1.))
                        .mb(rpx(2.))
                        .min_w_0()
                        .text_size(rpx(16.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .child(crate::app::display_name::render(
                            ("self-name", user.id),
                            &user.name,
                            user.name_style.as_ref(),
                            crate::app::display_name::Surface::Account { hovered },
                            cx,
                        )),
                )
                .child(account_subtitle(
                    user,
                    hovered,
                    animate_subtitle,
                    activities,
                    cx,
                )),
        )
}

fn with_options(
    app: &DiscordApp,
    button: Stateful<Div>,
    kind: DeviceKind,
    glow: bool,
) -> AnyElement {
    let (id, control, tip) = match kind {
        DeviceKind::Input => (
            "input-options",
            AccountControl::InputOptions,
            "Input Options",
        ),
        DeviceKind::Output => (
            "output-options",
            AccountControl::OutputOptions,
            "Output Options",
        ),
    };
    let open = app.device_menu_open(kind);
    let entity = app.entity.clone();
    let chevron = theme::icon_button(
        id,
        if open { "chevron-up" } else { "chevron-down" },
        16.,
        if glow { theme::RED } else { theme::muted() },
    )
    .w(rpx(16.))
    .h(rpx(32.))
    .ml(rpx(1.))
    .rounded_l(px(0.))
    .rounded_r(rpx(8.))
    .when(glow, |this| this.bg(rgba(MUTED_GLOW)))
    .when(open, |this| this.bg(rgba(theme::hover())))
    .when(!open, |this| account_tooltip(this, app, control, tip))
    .on_mouse_down(MouseButton::Left, move |event, _, cx| {
        cx.stop_propagation();
        let position = event.position;
        entity
            .update(cx, |app, cx| app.toggle_device_menu(kind, position, cx))
            .ok();
    });
    div()
        .flex()
        .flex_none()
        .items_center()
        .child(
            button
                .rounded_r(px(0.))
                .when(glow, |this| this.bg(rgba(MUTED_GLOW))),
        )
        .child(chevron)
        .into_any_element()
}

pub(super) fn account_button(id: &'static str, icon: &str, color: u32) -> Stateful<Div> {
    theme::icon_button(id, icon, 20., color)
        .debug_selector(move || format!("account-{id}-button"))
        .size(rpx(32.))
        .rounded(rpx(8.))
}

pub(super) fn account_row(
    nameplate: Option<AnyElement>,
    identity: Option<AnyElement>,
    controls: Vec<AnyElement>,
    compact: bool,
) -> Div {
    div()
        .debug_selector(|| "account-strip".into())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(if compact { 4. } else { 8. }))
        .h(rpx(USER_PANEL_HEIGHT))
        .p(rpx(if compact { 4. } else { 12. }))
        .children(nameplate.map(|nameplate| {
            div()
                .debug_selector(|| "account-nameplate".into())
                .absolute()
                .top(rpx(-1.))
                .bottom(rpx(-1.))
                .left_0()
                .right(rpx(-1.))
                .rounded(rpx(7.))
                .child(nameplate)
        }))
        .children(identity)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .when(!compact, |this| this.gap(rpx(8.)))
                .children(controls),
        )
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Online => "Online",
        Status::Idle => "Idle",
        Status::DoNotDisturb => "Do Not Disturb",
        Status::Offline => "Invisible",
    }
}

fn account_tooltip(
    element: Stateful<Div>,
    app: &DiscordApp,
    control: AccountControl,
    label: &'static str,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    element
        .relative()
        .on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, cx| {
                    if *hovered {
                        app.channel_rows.hovered_account_control = Some(control);
                    } else if app.channel_rows.hovered_account_control == Some(control) {
                        app.channel_rows.hovered_account_control = None;
                    } else {
                        return;
                    }
                    app.views.channel_list.update(cx, |_, cx| cx.notify());
                })
                .ok();
        })
        .when(
            app.channel_rows.hovered_account_control == Some(control),
            |this| this.child(theme::tooltip_above(label)),
        )
}

/// Tooltips are deferred above modals, and a control covered by one never sees its hover end.
pub(crate) fn hide_account_tooltip(app: &mut DiscordApp, cx: &mut App) {
    if app.channel_rows.hovered_account_control.take().is_some() {
        app.views.channel_list.update(cx, |_, cx| cx.notify());
    }
}

pub(crate) fn self_invisible(app: &DiscordApp) -> bool {
    app.store
        .user(app.store.current_user_id)
        .is_some_and(|user| user.status == Status::Offline)
}

pub(crate) fn sharing_paused_card(app: &DiscordApp, window: &Window) -> Option<AnyElement> {
    let position = app.sharing_paused?;
    app.game.current.as_ref()?;
    if !self_invisible(app) {
        return None;
    }
    let close = app.entity.clone();
    let go_online = app.entity.clone();
    Some(
        div()
            .id("sharing-paused")
            .absolute()
            .left(position.x - px(SHARING_CARD_ANCHOR))
            .bottom(window.viewport_size().height - position.y + px(SHARING_CARD_ANCHOR))
            .flex()
            .flex_col()
            .w(rpx(SHARING_CARD_WIDTH))
            .p_4()
            .rounded(rpx(12.))
            .bg(rgb(theme::surface_high()))
            .border_1()
            .border_color(rgba(theme::border()))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(move |_, _, cx| {
                close
                    .update(cx, |app, cx| {
                        app.sharing_paused = None;
                        cx.notify();
                    })
                    .ok();
            })
            .child(
                div()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .child("Activity sharing is paused"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(6.))
                    .mt(rpx(2.))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(theme::muted()))
                    .child(div().size(rpx(8.)).rounded_full().bg(rgb(theme::GREEN)))
                    .child("Go Online to share"),
            )
            .child(
                div()
                    .mt_2()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .text_color(rgb(theme::text()))
                    .child(
                        "Being Invisible hides my activity information about games and \
                         connected apps I use.",
                    ),
            )
            .child(
                div()
                    .id("sharing-go-online")
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(rpx(36.))
                    .mt_3()
                    .rounded(rpx(8.))
                    .cursor_pointer()
                    .bg(rgb(theme::BLURPLE))
                    .hover(|style| style.opacity(0.9))
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .on_click(move |_, _, cx| {
                        go_online
                            .update(cx, |app, cx| {
                                app.sharing_paused = None;
                                app.set_self_status(
                                    SelfStatus::Online,
                                    StatusDuration::Forever,
                                    cx,
                                );
                            })
                            .ok();
                    })
                    .child("Go Online"),
            )
            .into_any_element(),
    )
}

fn current_game_row(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let name = app.game.current.as_deref()?;
    let invisible = self_invisible(app);
    let entity = app.entity.clone();
    let icon = app
        .game
        .application
        .and_then(|id| app.game_icon_spec(id))
        .and_then(|spec| images::resolve(&spec, theme::image_px(CURRENT_GAME_ICON), cx))
        .map(|path| img(path).size_full().rounded(rpx(8.)).into_any_element())
        .unwrap_or_else(|| theme::icon("gamepad-2", 20., theme::muted()).into_any_element());
    Some(
        div()
            .debug_selector(|| "account-game".into())
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(8.))
            .p(rpx(8.))
            .border_b_1()
            .border_color(rgba(theme::border()))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(rpx(CURRENT_GAME_ICON))
                    .child(icon),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::strong()))
                            .child(label("account-game-name", name)),
                    )
                    .when(invisible, |this| {
                        this.child(
                            div()
                                .id("account-game-sharing")
                                .self_start()
                                .text_size(rpx(12.))
                                .line_height(rpx(16.))
                                .text_color(rgb(theme::muted()))
                                .cursor_pointer()
                                .hover(|style| style.underline())
                                .on_click(move |event, _, cx| {
                                    let position = event.position();
                                    entity
                                        .update(cx, |app, cx| {
                                            app.sharing_paused = Some(position);
                                            cx.notify();
                                        })
                                        .ok();
                                })
                                .child("Not Sharing"),
                        )
                    }),
            )
            .into_any_element(),
    )
}

pub(crate) fn user_panel(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let account_bounds = app.channel_rows.account_bounds.clone();
    let user = app.store.user(app.store.current_user_id);
    let nameplate = user
        .and_then(|user| app.store.nameplate(None, user.id))
        .and_then(|nameplate| {
            member_list::nameplate_layer(
                nameplate,
                app.store.current_user_id,
                USER_PANEL_HEIGHT + 2.,
                app.hover.account,
                cx,
            )
        });
    let voice = &app.voice;
    let (mic, mic_color, mic_tip) = if voice.self_mute || voice.self_deaf {
        ("mic-off", theme::RED, "Unmute")
    } else {
        ("mic", theme::text(), "Mute")
    };
    let (deafen, deafen_color, deafen_tip) = if voice.self_deaf {
        ("headphone-off", theme::RED, "Undeafen")
    } else {
        ("headphones", theme::text(), "Deafen")
    };
    let compact = crate::settings::activity_toggle();
    let identity = user.map(|user| {
        let game = user
            .game
            .as_ref()
            .map(|game| game.name.as_str())
            .or(app.game.current.as_deref());
        let voice = app
            .store
            .user_voice_channel(user.id)
            .or(app.voice.joined_call().map(|(_, channel_id)| channel_id))
            .and_then(|channel_id| app.store.channel_name(channel_id));
        account_identity(
            user,
            app.hover.account,
            app.channel_rows.subtitle_hovered_once,
            compact,
            member_list::activity_prefix("account-activity-prefix", user.id, game, voice),
            cx,
        )
        .id("account")
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::account_hover())))
        .on_hover({
            let entity = app.entity.clone();
            move |hovered: &bool, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        app.hover.account = *hovered;
                        app.channel_rows.subtitle_hovered_once = true;
                        cx.notify();
                    })
                    .ok();
            }
        })
        .without_text_selection()
        .on_popup_click(
            profile::shown(app, user.id),
            profile::account_opener(app, user.id, app.channel_rows.account_bounds.clone()),
        )
        .into_any_element()
    });
    let activity_toggle = crate::settings::activity_toggle().then(|| {
        let sharing = app.activity_privacy().share_activity;
        let (icon, color, tip) = if sharing {
            ("activity-game", theme::muted(), "Disable Activities")
        } else {
            ("activity-game-off", theme::RED, "Enable Activities")
        };
        account_button("activity-toggle", icon, color)
            .map(|this| account_tooltip(this, app, AccountControl::ActivityToggle, tip))
            .on_click({
                let entity = app.entity.clone();
                move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| app.toggle_activity_sharing(cx))
                        .ok();
                }
            })
            .into_any_element()
    });
    let controls = activity_toggle
        .into_iter()
        .chain([
            with_options(
                app,
                account_button("mic", mic, mic_color)
                    .map(|this| account_tooltip(this, app, AccountControl::Mute, mic_tip))
                    .on_click({
                        let entity = app.entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |app, cx| app.toggle_self_mute(cx)).ok();
                        }
                    }),
                DeviceKind::Input,
                voice.self_mute || voice.self_deaf,
            ),
            with_options(
                app,
                account_button("deafen", deafen, deafen_color)
                    .map(|this| account_tooltip(this, app, AccountControl::Deafen, deafen_tip))
                    .on_click({
                        let entity = app.entity.clone();
                        move |_, _, cx| {
                            entity.update(cx, |app, cx| app.toggle_self_deaf(cx)).ok();
                        }
                    }),
                DeviceKind::Output,
                voice.self_deaf,
            ),
            account_button("user-settings", "settings", theme::text())
                .map(|this| account_tooltip(this, app, AccountControl::Settings, "User Settings"))
                .on_click({
                    let entity = app.entity.clone();
                    move |_, window, cx| {
                        entity
                            .update(cx, |app, cx| app.open_settings(window, cx))
                            .ok();
                    }
                })
                .into_any_element(),
        ])
        .collect();
    let account = account_row(nameplate, identity, controls, compact)
        .on_prepaint(move |bounds, _, _| account_bounds.set(Some(bounds)));
    div()
        .flex()
        .flex_col()
        .flex_none()
        .m_2()
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .children(voice::panel(app, cx))
        .children(current_game_row(app, cx))
        .child(account)
}
