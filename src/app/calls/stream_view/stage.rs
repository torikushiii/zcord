use super::controls::{
    control, controls_fade, disconnect_control, ended_panel, member_name, members_slide,
    quality_pill, share_control, show_chat_button, stage_members, voice_controls,
};
use super::*;

const PANEL_RADIUS: f32 = 8.;
const STRIP_HEIGHT: f32 = TILE_HEIGHT + 8.;
const DOCK_FADE: f32 = 36.;

pub(crate) fn stage_area(
    app: &DiscordApp,
    top: f32,
    dock_height: f32,
    chat_open: bool,
) -> Stateful<Div> {
    let activity = app.entity.clone();
    let left = app.entity.clone();
    div()
        .id("stream-stage")
        .absolute()
        .inset_0()
        .when(chat_open, |this| this.right(rpx(CHAT_WIDTH + CHAT_GAP)))
        .bg(rgb(0x000000))
        .occlude()
        .on_mouse_move(move |event, window, cx| {
            let rem = window.rem_size();
            let top = rpx(top + chat::HEADER_HEIGHT).to_pixels(rem);
            let bottom = window.viewport_size().height - rpx(dock_height).to_pixels(rem);
            let y = event.position.y;
            activity
                .update(cx, |app, cx| app.stage_activity(y < top || y > bottom, cx))
                .ok();
        })
        .on_hover(move |hovered, _, cx| {
            if !*hovered {
                left.update(cx, |app, cx| app.leave_stage(cx)).ok();
            }
        })
}

pub(crate) fn stage_frame(
    app: &DiscordApp,
    area: Stateful<Div>,
    chat_title: String,
    chat_open: bool,
) -> AnyElement {
    let hide_chat = app.entity.clone();
    div()
        .absolute()
        .top(rpx(titlebar::HEIGHT))
        .bottom_0()
        .right_0()
        .left(rpx(guilds::guild_rail::WIDTH + guilds::channel_list::WIDTH))
        .child(area)
        .when(chat_open, |this| {
            this.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(rpx(CHAT_WIDTH))
                    .w(rpx(CHAT_GAP))
                    .bg(rgb(theme::base_lowest())),
            )
            .child(
                div()
                    .id("stage-chat-header")
                    .absolute()
                    .top_0()
                    .right_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w(rpx(CHAT_WIDTH))
                    .h(rpx(chat::HEADER_HEIGHT))
                    .px(rpx(16.))
                    .bg(rgb(theme::bg()))
                    .border_b_1()
                    .border_color(rgba(theme::border()))
                    .occlude()
                    .text_size(rpx(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child(theme::icon("message-circle", 20., theme::muted()))
                    .child(div().flex_1().min_w_0().truncate().child(chat_title))
                    .child(
                        theme::icon_button("stage-hide-chat", "x", 20., theme::muted()).on_click(
                            move |_, window, cx| {
                                hide_chat
                                    .update(cx, |app, cx| app.close_stream_chat(window, cx))
                                    .ok();
                            },
                        ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(rpx(CHAT_WIDTH))
                    .rounded_l(rpx(PANEL_RADIUS))
                    .border_1()
                    .border_color(rgba(theme::border())),
            )
            .child(corner(1).top_0().right(rpx(CHAT_WIDTH + CHAT_GAP)))
            .child(corner(2).bottom_0().right(rpx(CHAT_WIDTH + CHAT_GAP)))
            .child(corner(0).top_0().right(rpx(CHAT_WIDTH - PANEL_RADIUS)))
            .child(corner(3).bottom_0().right(rpx(CHAT_WIDTH - PANEL_RADIUS)))
        })
        .into_any_element()
}

/// GPUI clips children to rectangles, so a rounded panel whose content paints to its edges gets
/// its corners covered in the gap's color instead.
fn corner(quarter_turns: u8) -> Svg {
    theme::icon("corner-mask", PANEL_RADIUS, theme::base_lowest())
        .absolute()
        .with_transformation(Transformation::rotate(radians(
            std::f32::consts::FRAC_PI_2 * f32::from(quarter_turns),
        )))
}

pub(crate) fn render(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let view = app.stream.view.as_ref()?;
    let chat_open = app.stream_chat_open();
    let (area, chat_title) = stage(app, view, false, cx);
    Some(stage_frame(app, area, chat_title, chat_open))
}

pub(super) fn popout_stage(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let view = app.stream.view.as_ref()?;
    Some(stage(app, view, true, cx).0.into_any_element())
}

pub(super) fn stage(
    app: &DiscordApp,
    view: &StreamView,
    in_popout: bool,
    cx: &App,
) -> (Stateful<Div>, String) {
    let call = app.voice.call.as_ref();
    let guild_id = call.map_or(0, |call| call.guild_id);
    let name = member_name(app, guild_id, view.user_id);
    let channel = call
        .and_then(|call| app.store.channel_name(call.channel_id))
        .unwrap_or_default()
        .to_owned();
    let chat_title = channel.clone();
    let members_shown = app.stream.members_shown;
    let popped_out = view.popout.is_some();
    let shown_elsewhere = popped_out && !in_popout;
    let live = view.ended.is_none();
    let status = if shown_elsewhere {
        "Streaming in another window"
    } else {
        "Connecting\u{2026}"
    };
    let video = match (&view.ended, view.frame.clone().filter(|_| !shown_elsewhere)) {
        (Some(reason), _) => ended_panel(app, reason, name.clone(), members_shown),
        (None, Some(frame)) => img(frame)
            .size_full()
            .object_fit(ObjectFit::Contain)
            .into_any_element(),
        (None, None) => div()
            .text_size(rpx(15.))
            .text_color(rgb(theme::muted()))
            .child(status)
            .into_any_element(),
    };
    let toggle = app.entity.clone();
    let stop = app.entity.clone();
    let chat_open = app.stream_chat_open() && !in_popout;
    let popout = app.entity.clone();
    let volume = app.entity.clone();
    let dismiss_volume = app.entity.clone();
    let header = div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(chat::HEADER_HEIGHT))
        .px(rpx(16.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgba(0x000000B3), 0.),
            linear_color_stop(rgba(0x00000000), 1.),
        ))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(white())
        .child(theme::icon("volume-2", 20., 0xFFFFFF))
        .child(channel)
        .child(div().text_color(rgb(theme::muted())).child("\u{b7}"))
        .child(theme::undecorated_member_avatar(
            &app.store,
            guild_id,
            view.user_id,
            24.,
            cx,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(format!("{name}'s Screen")),
        )
        .children(
            view.height
                .filter(|_| live)
                .map(|height| quality_pill(height, view.fps)),
        )
        .when(!chat_open && !in_popout, |this| {
            this.child(show_chat_button(app))
        });
    let controls = div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap_2()
        .h(rpx(DOCK_HEIGHT))
        .child(voice_controls(app, in_popout))
        .child(share_control(app, in_popout))
        .child(if view.own {
            disconnect_control(app)
        } else {
            control(
                app,
                "stage-stop",
                "Stop Watching",
                "monitor-x",
                0xFFFFFF,
                STOP_RED,
            )
            .w(rpx(64.))
            .h(rpx(54.))
            .rounded(rpx(12.))
            .on_click(move |_, _, cx| {
                stop.update(cx, |app, cx| app.return_to_call(cx)).ok();
            })
        })
        .child(
            div()
                .absolute()
                .right(rpx(16.))
                .flex()
                .items_center()
                .gap_1()
                .when(!view.own, |this| {
                    this.child(
                        control(
                            app,
                            "stage-volume",
                            "Stream Volume",
                            "volume-2",
                            0xFFFFFF,
                            0,
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, _, cx| {
                            volume
                                .update(cx, |app, cx| {
                                    if let Some(view) = app.stream.view.as_mut() {
                                        view.volume_open = !view.volume_open;
                                    }
                                    app.views.overlays.update(cx, |_, cx| cx.notify());
                                })
                                .ok();
                        })
                        .when(view.volume_open, |this| {
                            this.children(app.stream.volume.as_ref().map(|(state, _)| {
                                div()
                                    .id("stage-volume-slider")
                                    .absolute()
                                    .bottom(rpx(52.))
                                    .right_0()
                                    .w(rpx(180.))
                                    .px(rpx(12.))
                                    .py(rpx(10.))
                                    .rounded(rpx(8.))
                                    .bg(rgb(PANEL))
                                    .border_1()
                                    .border_color(rgba(theme::border_subtle()))
                                    .occlude()
                                    .on_click(|_, _, cx| cx.stop_propagation())
                                    .child(
                                        div().h(rpx(20.)).child(
                                            Slider::new(state)
                                                .bg(rgb(theme::BLURPLE))
                                                .text_color(white()),
                                        ),
                                    )
                            }))
                        }),
                    )
                })
                .child(
                    control(
                        app,
                        "stage-popout",
                        if popped_out { "Pop In" } else { "Pop Out" },
                        "external-link",
                        0xFFFFFF,
                        0,
                    )
                    .on_click(move |_, _, cx| {
                        popout
                            .update(cx, |app, cx| app.toggle_stream_popout(cx))
                            .ok();
                    }),
                )
                .child(
                    control(
                        app,
                        "stage-fullscreen",
                        "Full Screen",
                        "video-fullscreen",
                        0xFFFFFF,
                        0,
                    )
                    .on_click(|_, window, _| window.toggle_fullscreen()),
                ),
        );
    let members_toggle = div()
        .absolute()
        .top(rpx(8.))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            control(
                app,
                "stage-members",
                if members_shown {
                    "Hide Members"
                } else {
                    "Show Members"
                },
                "users",
                0xFFFFFF,
                0x121214E6,
            )
            .flex_row_reverse()
            .gap_1()
            .w(rpx(72.))
            .h(rpx(32.))
            .rounded_full()
            .child(theme::icon(
                if members_shown {
                    "chevron-down"
                } else {
                    "chevron-up"
                },
                16.,
                0xFFFFFF,
            ))
            .on_click(move |_, _, cx| {
                toggle
                    .update(cx, |app, cx| {
                        app.stream.members_shown = !app.stream.members_shown;
                        app.stream.members_toggles += 1;
                        app.views.overlays.update(cx, |_, cx| cx.notify());
                    })
                    .ok();
            }),
        );
    let slide = (members_shown, app.stream.members_toggles);
    let strip = members_slide(
        div()
            .flex_none()
            .overflow_hidden()
            .children(stage_members(app, view, cx)),
        "stage-strip",
        slide,
        |this, shown| this.h(rpx(STRIP_HEIGHT * shown)),
        cx,
    );
    let dock = members_slide(
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .child(controls_fade(
                div().absolute().inset_0().bg(linear_gradient(
                    0.,
                    linear_color_stop(rgba(0x000000CC), 0.),
                    linear_color_stop(rgba(0x00000000), 1.),
                )),
                "stage-dock-shade",
                &view.controls,
                cx,
            ))
            .child(strip)
            .child(controls_fade(
                controls,
                "stage-controls",
                &view.controls,
                cx,
            ))
            .child(controls_fade(
                members_toggle,
                "stage-members-toggle",
                &view.controls,
                cx,
            )),
        "stage-dock",
        slide,
        |this, shown| this.pt(rpx(DOCK_FADE * (1. - shown))),
        cx,
    );
    let stage = members_slide(
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .flex()
            .items_center()
            .justify_center()
            .child(video)
            .child(controls_fade(header, "stage-header", &view.controls, cx)),
        "stage-video",
        slide,
        |this, shown| this.bottom(rpx((STRIP_HEIGHT + DOCK_HEIGHT) * shown)),
        cx,
    );
    let dock_height = DOCK_HEIGHT
        + if members_shown {
            STRIP_HEIGHT
        } else {
            DOCK_FADE
        };
    let top = if in_popout {
        POPOUT_TITLEBAR
    } else {
        titlebar::HEIGHT
    };
    let stage_area = stage_area(app, top, dock_height, chat_open)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            dismiss_volume
                .update(cx, |app, cx| {
                    if let Some(view) = app.stream.view.as_mut().filter(|view| view.volume_open) {
                        view.volume_open = false;
                        app.views.overlays.update(cx, |_, cx| cx.notify());
                    }
                })
                .ok();
        })
        .child(stage)
        .child(dock);
    (stage_area, chat_title)
}
