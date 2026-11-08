use super::*;

const TILE_WIDTH: f32 = 192.;
const CONTROLS_FADE: Duration = Duration::from_millis(200);
const MEMBERS_SLIDE: Duration = Duration::from_millis(250);
const MUTED_TINT: u32 = 0xD22D3933;

pub(crate) fn member_name(app: &DiscordApp, guild_id: u64, user_id: u64) -> String {
    app.store
        .member_name(guild_id, user_id)
        .map(str::to_owned)
        .or_else(|| app.store.user(user_id).map(|user| user.name.clone()))
        .unwrap_or_default()
}

pub(crate) fn name_chip(icon: Option<&str>, name: String) -> Div {
    div()
        .absolute()
        .left(rpx(8.))
        .bottom(rpx(8.))
        .flex()
        .items_center()
        .gap_1()
        .max_w(relative(0.9))
        .px(rpx(8.))
        .h(rpx(24.))
        .rounded(rpx(6.))
        .bg(rgba(0x000000B3))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(white())
        .children(icon.map(|icon| theme::icon(icon, 16., 0xFFFFFF)))
        .child(div().truncate().child(name))
}

fn tile() -> Div {
    div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(rpx(TILE_WIDTH))
        .h(rpx(TILE_HEIGHT))
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(rgb(theme::surface_highest()))
}

pub(crate) fn control(
    app: &DiscordApp,
    id: &'static str,
    tip: &'static str,
    icon: &str,
    color: u32,
    background: u32,
) -> Stateful<Div> {
    control_with(app, id, tip, background).child(theme::icon(icon, 22., color))
}

pub(super) fn control_with(
    app: &DiscordApp,
    id: &'static str,
    tip: &'static str,
    background: u32,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    bare_control(id, background)
        .on_hover(move |hovered, window, cx| {
            let key = (id, window.window_handle().window_id());
            entity
                .update(cx, |app, cx| {
                    if *hovered {
                        app.stream.hovered_control = Some(key);
                    } else if app.stream.hovered_control == Some(key) {
                        app.stream.hovered_control = None;
                    }
                    app.views.overlays.update(cx, |_, cx| cx.notify());
                })
                .ok();
        })
        .children(
            app.stream
                .hovered_control
                .filter(|(hovered, _)| *hovered == id)
                .map(|(_, window)| ControlTip { tip, window }),
        )
}

fn bare_control(id: &'static str, background: u32) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .w(rpx(48.))
        .h(rpx(44.))
        .rounded(rpx(8.))
        .flex_none()
        .cursor_pointer()
        .bg(rgba(background))
        .hover(|style| style.opacity(0.85))
}

fn menu_chevron(id: &'static str, open: bool, color: u32, background: u32) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w(rpx(24.))
        .h(rpx(44.))
        .rounded_r(rpx(8.))
        .cursor_pointer()
        .bg(rgba(background))
        .hover(|style| style.opacity(0.85))
        .child(theme::icon(
            if open { "chevron-up" } else { "chevron-down" },
            16.,
            color,
        ))
}

fn with_menu(control: Stateful<Div>, chevron: Option<Stateful<Div>>) -> Div {
    let joined = chevron.is_some();
    div()
        .flex()
        .flex_none()
        .items_center()
        .child(control.when(joined, |this| this.rounded_r(px(0.))))
        .children(chevron)
}

fn chevron_tint(tint: u32) -> u32 {
    if tint == 0 {
        0
    } else {
        (tint & !0xFF) | ((tint & 0xFF) + 0x14).min(0xFF)
    }
}

#[derive(IntoElement)]
struct ControlTip {
    tip: &'static str,
    window: WindowId,
}

impl RenderOnce for ControlTip {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        // Fills the control, so the tooltip measures from it as if it were a direct child.
        div()
            .absolute()
            .inset_0()
            .when(window.window_handle().window_id() == self.window, |this| {
                this.child(theme::tooltip_above(self.tip))
            })
    }
}

/// Plays `apply` towards `shown` after a toggle. With `toggles` at 0, or reduced motion, it
/// settles at once, so opening a stream doesn't slide.
pub(super) fn members_slide(
    element: Div,
    id: &'static str,
    (shown, toggles): (bool, u64),
    apply: impl Fn(Div, f32) -> Div + 'static,
    cx: &App,
) -> AnyElement {
    if toggles == 0 || cx.reduce_motion() {
        return apply(element, if shown { 1. } else { 0. }).into_any_element();
    }
    element
        .with_animation(
            (id, toggles),
            Animation::new(MEMBERS_SLIDE).with_easing(ease_out_quint()),
            move |element, delta| apply(element, if shown { delta } else { 1. - delta }),
        )
        .into_any_element()
}

pub(crate) fn controls_fade(
    element: Div,
    id: &'static str,
    controls: &StageControls,
    cx: &App,
) -> AnyElement {
    let hidden = controls.hidden;
    if controls.changes == 0 || cx.reduce_motion() {
        return element
            .opacity(if hidden { 0. } else { 1. })
            .into_any_element();
    }
    element
        .with_animation(
            (id, controls.changes),
            Animation::new(CONTROLS_FADE).with_easing(ease_out_quint()),
            move |element, delta| element.opacity(if hidden { 1. - delta } else { delta }),
        )
        .into_any_element()
}

/// Discord's end reasons are codes, not sentences. Anything else is already readable.
fn ended_message(reason: &str) -> &str {
    match reason {
        "stream_ended" | "user_requested" => "This stream has ended.",
        "stream_full" => "This stream is full.",
        "unauthorized" => "You can't watch this stream.",
        other => other,
    }
}

pub(super) fn ended_panel(
    app: &DiscordApp,
    reason: &str,
    name: String,
    members_shown: bool,
) -> AnyElement {
    let back = app.entity.clone();
    let inset = if members_shown { 8. } else { DOCK_HEIGHT + 8. };
    div()
        .absolute()
        .top(rpx(chat::HEADER_HEIGHT))
        .bottom(rpx(inset))
        .left(rpx(16.))
        .right(rpx(16.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rpx(16.))
        .rounded(rpx(8.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgb(0x241D35), 0.),
            linear_color_stop(rgb(0x0B0B0D), 0.6),
        ))
        .child(
            div()
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .child(ended_message(reason).to_owned()),
        )
        .child(
            div()
                .id("stage-close-stream")
                .flex()
                .items_center()
                .h(rpx(36.))
                .px(rpx(16.))
                .rounded(rpx(8.))
                .cursor_pointer()
                .bg(rgb(0x1E1F22))
                .border_1()
                .border_color(rgba(theme::border_subtle()))
                .hover(|style| style.bg(rgb(0x2B2C31)))
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .on_click(move |_, _, cx| {
                    back.update(cx, |app, cx| app.return_to_call(cx)).ok();
                })
                .child("Close Stream"),
        )
        .child(name_chip(Some("monitor-filled"), name))
        .into_any_element()
}

pub(super) fn control_group() -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .p(rpx(4.))
        .rounded(rpx(12.))
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgba(theme::border_subtle()))
}

pub(super) fn quality_pill(height: u32, fps: u32) -> Div {
    let quality = if fps == 0 {
        format!("{height}p")
    } else {
        format!("{height}p {fps}FPS")
    };
    div()
        .flex()
        .flex_none()
        .h(rpx(20.))
        .text_size(rpx(12.))
        .line_height(rpx(20.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white())
        .child(
            div()
                .px(rpx(8.))
                .rounded_l_full()
                .bg(rgba(0x4E505899))
                .child(quality),
        )
        .child(
            div()
                .px(rpx(6.))
                .rounded_r_full()
                .bg(rgba(STOP_RED))
                .font_weight(FontWeight::BOLD)
                .child("LIVE"),
        )
}

pub(super) fn stage_members(app: &DiscordApp, view: &StreamView, cx: &App) -> Option<Div> {
    let call = app.voice.call.as_ref()?;
    let guild_id = call.guild_id;
    let name = member_name(app, guild_id, view.user_id);
    let live = view.ended.is_none();
    let stream_tile = tile()
        .bg(rgb(theme::bg()))
        .children(view.frame.clone().filter(|_| live).map(|frame| {
            img(frame)
                .size_full()
                .rounded(rpx(8.))
                .object_fit(ObjectFit::Contain)
        }))
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(theme::icon("monitor-filled", 32., 0xFFFFFF)),
        )
        .child(
            div()
                .absolute()
                .top(rpx(8.))
                .right(rpx(8.))
                .when(live, |this| this.child(voice::live_badge())),
        )
        .child(name_chip(Some("monitor-filled"), name));
    let members = app
        .store
        .voice_members(guild_id, call.channel_id)
        .into_iter()
        .map(|member| {
            let muted = member.mute || member.self_mute;
            let speaking = app.voice.speaking.contains(&member.user_id);
            tile()
                .when(speaking, |this| {
                    this.border_2().border_color(rgb(theme::GREEN))
                })
                .child(theme::undecorated_member_avatar(
                    &app.store,
                    guild_id,
                    member.user_id,
                    64.,
                    cx,
                ))
                .child(name_chip(
                    muted.then_some("mic-off"),
                    member_name(app, guild_id, member.user_id),
                ))
        });
    Some(
        div()
            .flex()
            .flex_none()
            .justify_center()
            .gap_2()
            .px(rpx(16.))
            .pt(rpx(8.))
            .child(stream_tile)
            .children(members),
    )
}

pub(crate) fn voice_controls(app: &DiscordApp, in_popout: bool) -> Div {
    let mute = app.entity.clone();
    let menu = app.entity.clone();
    let muted = app.voice.self_mute || app.voice.self_deaf;
    let (color, tint) = if muted {
        (theme::RED, MUTED_TINT)
    } else {
        (theme::text(), 0)
    };
    let mic = control(
        app,
        "stage-mute",
        if muted { "Unmute" } else { "Mute" },
        if muted { "mic-off" } else { "mic" },
        color,
        tint,
    )
    .on_click(move |_, _, cx| {
        mute.update(cx, |app, cx| app.toggle_self_mute(cx)).ok();
    });
    let mic_menu = (!in_popout).then(|| {
        menu_chevron(
            "stage-mic-menu",
            app.call_audio_menu_open(),
            color,
            chevron_tint(tint),
        )
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            // The open menu closes on any press outside it, this one included.
            cx.stop_propagation();
            let position = event.position;
            menu.update(cx, |app, cx| app.toggle_call_audio_menu(position, cx))
                .ok();
        })
    });
    let camera = voice::unavailable(
        bare_control("stage-camera", 0).child(theme::icon("video-slash", 22., theme::text())),
        "Camera",
    );
    let camera_menu = (!in_popout).then(|| {
        voice::unavailable(
            menu_chevron("stage-camera-menu", false, theme::text(), 0),
            "Camera",
        )
    });
    control_group()
        .child(with_menu(mic, mic_menu))
        .child(with_menu(camera, camera_menu))
}

pub(crate) fn disconnect_control(app: &DiscordApp) -> Stateful<Div> {
    let leave = app.entity.clone();
    control_with(app, "stage-leave", "Disconnect", STOP_RED)
        .w(rpx(64.))
        .h(rpx(54.))
        .rounded(rpx(12.))
        .child(
            theme::icon("phone", 22., 0xFFFFFF).with_transformation(Transformation::rotate(
                radians(std::f32::consts::FRAC_PI_4 * 3.),
            )),
        )
        .on_click(move |_, _, cx| {
            leave.update(cx, |app, cx| app.leave_voice(cx)).ok();
        })
}

pub(crate) fn share_control(app: &DiscordApp, in_popout: bool) -> Div {
    let share = app.entity.clone();
    let menu = app.entity.clone();
    let sharing = app.voice.sharing_screen();
    let tint = if sharing {
        (theme::GREEN << 8) | 0x29
    } else {
        0
    };
    let button = control(
        app,
        "stage-share",
        if sharing {
            "Stop Streaming"
        } else {
            "Share Your Screen"
        },
        if sharing { "monitor-x" } else { "monitor-up" },
        if sharing { theme::GREEN } else { theme::text() },
        tint,
    )
    .on_click(move |_, _, cx| {
        share.update(cx, |app, cx| app.screen_button(cx)).ok();
    });
    let chevron = (sharing && !in_popout).then(|| {
        menu_chevron(
            "stage-share-menu",
            app.stream_menu_open(),
            theme::GREEN,
            chevron_tint(tint),
        )
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            cx.stop_propagation();
            let position = event.position;
            menu.update(cx, |app, cx| app.toggle_live_menu(position, cx))
                .ok();
        })
    });
    control_group().child(with_menu(button, chevron))
}

pub(crate) fn show_chat_button(app: &DiscordApp) -> Stateful<Div> {
    let show_chat = app.entity.clone();
    div()
        .id("stage-show-chat")
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(32.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::hover())))
        .relative()
        .child(theme::hover_tooltip("stream-show-chat-tip", "Show Chat"))
        .on_click(move |_, window, cx| {
            show_chat
                .update(cx, |app, cx| app.open_stream_chat(window, cx))
                .ok();
        })
        .child(theme::icon("message-circle", 20., 0xFFFFFF))
}
