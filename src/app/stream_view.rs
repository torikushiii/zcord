use super::*;
use crate::app::voice;
use crate::discord::{StreamEvent, StreamFrame};
use gpui_component::slider::{Slider, SliderEvent, SliderState};

const FRAME_POLL: Duration = Duration::from_millis(16);
const TILE_WIDTH: f32 = 192.;
const TILE_HEIGHT: f32 = 108.;
pub(super) const DOCK_HEIGHT: f32 = 72.;
pub(super) const CHAT_WIDTH: f32 = 448.;
const VOLUME_MAX: f32 = 200.;
const STRIP_HEIGHT: f32 = TILE_HEIGHT + 8.;
const DOCK_FADE: f32 = 36.;
const CONTROLS_IDLE: Duration = Duration::from_secs(3);
const CONTROLS_FADE: Duration = Duration::from_millis(200);
const MEMBERS_SLIDE: Duration = Duration::from_millis(250);
const PANEL: u32 = 0x0F0F11;
pub(super) const STOP_RED: u32 = 0xD22D39FF;
const MUTED_TINT: u32 = 0xD22D3933;

pub(crate) struct StreamView {
    user_id: u64,
    nonce: u64,
    frame: Option<Arc<RenderImage>>,
    retired: Option<Arc<RenderImage>>,
    ended: Option<String>,
    height: Option<u32>,
    fps: u32,
    controls: StageControls,
    volume_open: bool,
    popout: Option<WindowHandle<StreamPopout>>,
}

pub(crate) struct StreamPopout {
    app: WeakEntity<DiscordApp>,
}

impl Render for StreamPopout {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = self
            .app
            .upgrade()
            .and_then(|app| app.read(cx).stream_view.as_ref()?.frame.clone());
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x000000))
            .children(frame.map(|frame| img(frame).size_full().object_fit(ObjectFit::Contain)))
    }
}

fn render_image(frame: StreamFrame) -> Option<Arc<RenderImage>> {
    // gpui's `RenderImage` holds BGRA in an `RgbaImage` container.
    let image = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
}

/// The header and dock of a stage, which fade out while the pointer is idle.
pub(super) struct StageControls {
    last_activity: Instant,
    over: bool,
    hidden: bool,
    changes: u64,
}

impl StageControls {
    pub(super) fn new() -> Self {
        Self {
            last_activity: Instant::now(),
            over: false,
            hidden: false,
            changes: 0,
        }
    }

    pub(super) fn idle(&self) -> bool {
        !self.over && self.last_activity.elapsed() >= CONTROLS_IDLE
    }

    fn set_hidden(&mut self, hidden: bool) {
        if self.hidden != hidden {
            self.hidden = hidden;
            self.changes += 1;
        }
    }
}

impl DiscordApp {
    fn stage_controls(&mut self) -> Option<&mut StageControls> {
        match &mut self.stream_view {
            Some(view) => Some(&mut view.controls),
            None => self.voice.call_view.as_mut().map(|view| &mut view.controls),
        }
    }

    /// The bars stay up while the pointer rests on them, and the stage's timer hides them
    /// otherwise.
    fn stage_activity(&mut self, over_controls: bool, cx: &mut Context<Self>) {
        let Some(controls) = self.stage_controls() else {
            return;
        };
        controls.last_activity = Instant::now();
        controls.over = over_controls;
        if controls.hidden {
            controls.set_hidden(false);
            self.views.overlays.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn hide_stage_controls(&mut self, cx: &mut Context<Self>) {
        if self
            .stream_view
            .as_ref()
            .is_some_and(|view| view.volume_open || view.frame.is_none() || view.ended.is_some())
        {
            return;
        }
        let Some(controls) = self.stage_controls().filter(|controls| !controls.hidden) else {
            return;
        };
        controls.set_hidden(true);
        self.hovered_stage_control = None;
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    fn leave_stage(&mut self, cx: &mut Context<Self>) {
        if let Some(controls) = self.stage_controls() {
            controls.over = false;
        }
        self.hide_stage_controls(cx);
    }

    fn stage_open(&self) -> bool {
        self.stream_view.is_some() || self.voice.call_view.is_some()
    }

    pub(crate) fn stream_chat_open(&self) -> bool {
        self.stage_chat.is_some() && self.stage_open()
    }

    /// A call can end from places without a window, which leaves its chat to close later.
    pub(super) fn stage_chat_orphaned(&self) -> bool {
        self.stage_chat.is_some() && !self.stage_open()
    }

    /// Voice channels have their own text chat, which only opens from a stage.
    fn open_stream_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((guild_id, channel_id)) = self.voice.joined_call() else {
            return;
        };
        if !self.stage_open() {
            return;
        }
        let back = (self.selected_guild_id, self.selected_channel_id);
        if guild_id != 0 && self.selected_guild_id != guild_id {
            // Passing through the guild's text channel must not dismiss the call.
            let call_view = self.voice.call_view.take();
            self.select_guild(guild_id, window, cx);
            self.voice.call_view = call_view;
        }
        self.stage_chat = Some(back);
        if guild_id == 0 {
            self.select_dm(channel_id, window, cx);
        } else {
            self.select_channel(channel_id, window, cx);
        }
    }

    pub(super) fn close_stream_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((guild_id, channel_id)) = self.stage_chat.take() else {
            return;
        };
        // Going back selects a text channel, which would otherwise dismiss the call behind it.
        let call_view = self.voice.call_view.take();
        if guild_id == 0 {
            self.open_home(window, cx);
        } else {
            self.select_guild_channel(guild_id, Some(channel_id), window, cx);
        }
        self.voice.call_view = call_view;
    }

    fn ensure_stream_volume(&mut self, cx: &mut Context<Self>) {
        if self.stream_volume.is_some() {
            return;
        }
        let state = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(VOLUME_MAX)
                .step(1.)
                .default_value(100.)
        });
        let events = cx.subscribe(&state, |app, _, event: &SliderEvent, _| {
            if let SliderEvent::Change(value) = event
                && let Some(session) = &app.session
            {
                session.set_stream_volume(value.start() / 100.);
            }
        });
        self.stream_volume = Some((state, events));
    }

    fn toggle_stream_popout(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.stream_view.as_mut() else {
            return;
        };
        if let Some(popout) = view.popout.take() {
            popout
                .update(cx, |_, window, _| window.remove_window())
                .ok();
            self.views.overlays.update(cx, |_, cx| cx.notify());
            return;
        }
        let app = cx.entity().downgrade();
        let closed = app.clone();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(960.), px(540.)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Stream".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let opened = cx.open_window(options, |window, cx| {
            window.on_window_should_close(cx, move |_, cx| {
                closed
                    .update(cx, |app, cx| {
                        if let Some(view) = app.stream_view.as_mut() {
                            view.popout = None;
                        }
                        app.views.overlays.update(cx, |_, cx| cx.notify());
                    })
                    .ok();
                true
            });
            cx.new(|_| StreamPopout { app })
        });
        match opened {
            Ok(handle) => {
                if let Some(view) = self.stream_view.as_mut() {
                    view.popout = Some(handle);
                }
            }
            Err(error) => log::warn!("Could not open the stream window: {error}"),
        }
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    pub(crate) fn watch_stream(
        &mut self,
        user_id: u64,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        self.close_stream(window, cx);
        session.watch_stream(user_id);
        self.stream_nonce += 1;
        let nonce = self.stream_nonce;
        self.stream_view = Some(StreamView {
            user_id,
            nonce,
            frame: None,
            retired: None,
            ended: None,
            height: None,
            fps: 0,
            controls: StageControls::new(),
            volume_open: false,
            popout: None,
        });
        self.ensure_stream_volume(cx);
        let frames = session.stream_frames();
        cx.spawn(async move |this, cx| {
            let mut counted = 0;
            let mut counting_since = Instant::now();
            loop {
                cx.background_executor().timer(FRAME_POLL).await;
                let frame = frames.lock().ok().and_then(|mut slot| slot.take());
                let height = frame.as_ref().map(|frame| frame.height);
                counted += u32::from(frame.is_some());
                let fps = (counting_since.elapsed() >= Duration::from_secs(1)).then(|| {
                    counting_since = Instant::now();
                    std::mem::take(&mut counted)
                });
                let image = frame.and_then(render_image);
                let keep_going = this.update(cx, |app, cx| {
                    let Some(view) = app.stream_view.as_mut().filter(|view| view.nonce == nonce)
                    else {
                        return false;
                    };
                    if let Some(fps) = fps {
                        view.fps = fps;
                    }
                    let idle = view.controls.idle();
                    if let Some(image) = image {
                        view.height = height;
                        let retired = view.retired.take();
                        view.retired = view.frame.replace(image);
                        if let Some(retired) = retired {
                            cx.drop_image(retired, None);
                        }
                        if let Some(popout) = view.popout {
                            popout.update(cx, |_, _, cx| cx.notify()).ok();
                        }
                        app.views.overlays.update(cx, |_, cx| cx.notify());
                    }
                    if idle {
                        app.hide_stage_controls(cx);
                    }
                    true
                });
                if !matches!(keep_going, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    pub(crate) fn close_stream(&mut self, mut window: Option<&mut Window>, cx: &mut Context<Self>) {
        if let Some(window) = window.as_deref_mut() {
            self.close_stream_chat(window, cx);
        }
        let Some(view) = self.stream_view.take() else {
            return;
        };
        if let Some(popout) = view.popout {
            popout
                .update(cx, |_, window, _| window.remove_window())
                .ok();
        }
        if view.ended.is_none()
            && let Some(session) = &self.session
        {
            session.stop_watching();
        }
        for image in [view.frame, view.retired].into_iter().flatten() {
            cx.drop_image(image, window.as_deref_mut());
        }
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    /// The chat stays open across the switch, since both stages share the call's channel.
    fn return_to_call(&mut self, cx: &mut Context<Self>) {
        self.close_stream(None, cx);
        self.open_call_view(cx);
    }

    pub(crate) fn handle_stream_event(&mut self, event: StreamEvent, cx: &mut Context<Self>) {
        let StreamEvent::Ended { user_id, reason } = event else {
            return;
        };
        let Some(view) = self
            .stream_view
            .as_mut()
            .filter(|view| view.user_id == user_id)
        else {
            return;
        };
        let finished = reason
            .as_deref()
            .is_none_or(|reason| matches!(reason, "stream_ended" | "user_requested"));
        view.ended = Some(reason.unwrap_or_else(|| "The stream has ended.".into()));
        view.controls.set_hidden(false);
        if finished && self.voice.call.is_some() {
            self.return_to_call(cx);
            return;
        }
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }
}

pub(super) fn member_name(app: &DiscordApp, guild_id: u64, user_id: u64) -> String {
    app.store
        .member_name(guild_id, user_id)
        .map(str::to_owned)
        .or_else(|| app.store.user(user_id).map(|user| user.name.clone()))
        .unwrap_or_default()
}

pub(super) fn name_chip(icon: Option<&str>, name: String) -> Div {
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
        .bg(rgb(theme::SURFACE_HIGHEST))
}

pub(super) fn control(
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
        .on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, cx| {
                    app.hovered_stage_control = hovered.then_some(id);
                    app.views.overlays.update(cx, |_, cx| cx.notify());
                })
                .ok();
        })
        .when(app.hovered_stage_control == Some(id), |this| {
            this.child(theme::tooltip_above(tip))
        })
}

/// Plays `apply` towards `shown` after a toggle. With `toggles` at 0, or reduced motion, it
/// settles at once, so opening a stream doesn't slide.
fn members_slide(
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

pub(super) fn controls_fade(
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

fn ended_panel(app: &DiscordApp, reason: &str, name: String, members_shown: bool) -> AnyElement {
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
                .text_color(rgb(theme::STRONG))
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
                .border_color(rgba(theme::BORDER_SUBTLE))
                .hover(|style| style.bg(rgb(0x2B2C31)))
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::STRONG))
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
        .border_color(rgba(theme::BORDER_SUBTLE))
}

fn quality_pill(height: u32, fps: u32) -> Div {
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

fn stage_members(app: &DiscordApp, view: &StreamView, cx: &App) -> Option<Div> {
    let call = app.voice.call.as_ref()?;
    let guild_id = call.guild_id;
    let name = member_name(app, guild_id, view.user_id);
    let live = view.ended.is_none();
    let stream_tile = tile()
        .bg(rgb(theme::BG))
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

pub(super) fn voice_controls(app: &DiscordApp) -> Div {
    let mute = app.entity.clone();
    let deafen = app.entity.clone();
    let muted = app.voice.self_mute;
    let deafened = app.voice.self_deaf;
    let tint = |on: bool| if on { MUTED_TINT } else { 0 };
    control_group()
        .child(
            control(
                app,
                "stage-mute",
                if muted { "Unmute" } else { "Mute" },
                if muted { "mic-off" } else { "mic" },
                if muted { theme::RED } else { theme::TEXT },
                tint(muted),
            )
            .on_click(move |_, _, cx| {
                mute.update(cx, |app, cx| app.toggle_self_mute(cx)).ok();
            }),
        )
        .child(
            control(
                app,
                "stage-deafen",
                if deafened { "Undeafen" } else { "Deafen" },
                if deafened {
                    "headphone-off"
                } else {
                    "headphones"
                },
                if deafened { theme::RED } else { theme::TEXT },
                tint(deafened),
            )
            .on_click(move |_, _, cx| {
                deafen.update(cx, |app, cx| app.toggle_self_deaf(cx)).ok();
            }),
        )
}

pub(super) fn share_control(app: &DiscordApp) -> Div {
    let share = app.entity.clone();
    let sharing = app.voice.sharing_screen();
    control_group().child(
        control(
            app,
            "stage-share",
            if sharing {
                "Stop Streaming"
            } else {
                "Share Your Screen"
            },
            "monitor-up",
            if sharing { theme::GREEN } else { theme::TEXT },
            0,
        )
        .on_click(move |_, _, cx| {
            share.update(cx, |app, cx| app.screen_button(cx)).ok();
        }),
    )
}

pub(super) fn show_chat_button(app: &DiscordApp) -> Stateful<Div> {
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
        .hover(|style| style.bg(rgba(theme::HOVER)))
        .relative()
        .child(theme::hover_tooltip("stream-show-chat-tip", "Show Chat"))
        .on_click(move |_, window, cx| {
            show_chat
                .update(cx, |app, cx| app.open_stream_chat(window, cx))
                .ok();
        })
        .child(theme::icon("message-circle", 20., 0xFFFFFF))
}

/// The stage itself, which reports pointer activity to the fading controls.
pub(super) fn stage_area(app: &DiscordApp, dock_height: f32, chat_open: bool) -> Stateful<Div> {
    let activity = app.entity.clone();
    let left = app.entity.clone();
    div()
        .id("stream-stage")
        .absolute()
        .inset_0()
        .when(chat_open, |this| this.right(rpx(CHAT_WIDTH)))
        .bg(rgb(0x000000))
        .occlude()
        .on_mouse_move(move |event, window, cx| {
            let rem = window.rem_size();
            let top = rpx(titlebar::HEIGHT + chat::HEADER_HEIGHT).to_pixels(rem);
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

/// Places a stage over the chat column, leaving the chat itself uncovered while it's open.
pub(super) fn stage_frame(
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
        .left(rpx(guild_rail::WIDTH + channel_list::WIDTH))
        .child(area)
        .when(chat_open, |this| {
            this.child(
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
                    .bg(rgb(theme::BG))
                    .border_b_1()
                    .border_color(rgba(theme::BORDER))
                    .occlude()
                    .text_size(rpx(16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::STRONG))
                    .child(theme::icon("message-circle", 20., theme::MUTED))
                    .child(div().flex_1().min_w_0().truncate().child(chat_title))
                    .child(
                        theme::icon_button("stage-hide-chat", "x", 20., theme::MUTED).on_click(
                            move |_, window, cx| {
                                hide_chat
                                    .update(cx, |app, cx| app.close_stream_chat(window, cx))
                                    .ok();
                            },
                        ),
                    ),
            )
        })
        .into_any_element()
}

pub(super) fn render(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let view = app.stream_view.as_ref()?;
    let call = app.voice.call.as_ref();
    let guild_id = call.map_or(0, |call| call.guild_id);
    let name = member_name(app, guild_id, view.user_id);
    let channel = call
        .and_then(|call| app.store.channel_name(call.channel_id))
        .unwrap_or_default()
        .to_owned();
    let chat_title = channel.clone();
    let members_shown = app.stream_members_shown;
    let popped_out = view.popout.is_some();
    let live = view.ended.is_none();
    let status = if popped_out {
        "Streaming in another window"
    } else {
        "Connecting\u{2026}"
    };
    let video = match (&view.ended, view.frame.clone().filter(|_| !popped_out)) {
        (Some(reason), _) => ended_panel(app, reason, name.clone(), members_shown),
        (None, Some(frame)) => img(frame)
            .size_full()
            .object_fit(ObjectFit::Contain)
            .into_any_element(),
        (None, None) => div()
            .text_size(rpx(15.))
            .text_color(rgb(theme::MUTED))
            .child(status)
            .into_any_element(),
    };
    let toggle = app.entity.clone();
    let stop = app.entity.clone();
    let chat_open = app.stream_chat_open();
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
        .child(div().text_color(rgb(theme::MUTED)).child("\u{b7}"))
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
        .when(!chat_open, |this| this.child(show_chat_button(app)));
    let controls = div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap_2()
        .h(rpx(DOCK_HEIGHT))
        .child(voice_controls(app))
        .child(share_control(app))
        .child(
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
            }),
        )
        .child(
            div()
                .absolute()
                .right(rpx(16.))
                .flex()
                .items_center()
                .gap_1()
                .child(
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
                                if let Some(view) = app.stream_view.as_mut() {
                                    view.volume_open = !view.volume_open;
                                }
                                app.views.overlays.update(cx, |_, cx| cx.notify());
                            })
                            .ok();
                    })
                    .when(view.volume_open, |this| {
                        this.children(app.stream_volume.as_ref().map(|(state, _)| {
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
                                .border_color(rgba(theme::BORDER_SUBTLE))
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
                        app.stream_members_shown = !app.stream_members_shown;
                        app.stream_members_toggles += 1;
                        app.views.overlays.update(cx, |_, cx| cx.notify());
                    })
                    .ok();
            }),
        );
    let slide = (members_shown, app.stream_members_toggles);
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
    let stage_area = stage_area(app, dock_height, chat_open)
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            dismiss_volume
                .update(cx, |app, cx| {
                    if let Some(view) = app.stream_view.as_mut().filter(|view| view.volume_open) {
                        view.volume_open = false;
                        app.views.overlays.update(cx, |_, cx| cx.notify());
                    }
                })
                .ok();
        })
        .child(stage)
        .child(dock);
    Some(stage_frame(app, stage_area, chat_title, chat_open))
}
