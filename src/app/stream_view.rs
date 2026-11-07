use super::*;
use crate::app::voice;
use crate::discord::{StreamEvent, StreamFrame};
use gpui_component::slider::{Slider, SliderEvent, SliderState};

const FRAME_POLL: Duration = Duration::from_millis(16);
const TILE_WIDTH: f32 = 192.;
const TILE_HEIGHT: f32 = 108.;
pub(super) const DOCK_HEIGHT: f32 = 72.;
pub(super) const CHAT_WIDTH: f32 = 448.;
pub(super) const CHAT_GAP: f32 = 8.;
const PANEL_RADIUS: f32 = 8.;
const VOLUME_MAX: f32 = 200.;
const STRIP_HEIGHT: f32 = TILE_HEIGHT + 8.;
const DOCK_FADE: f32 = 36.;
const CONTROLS_IDLE: Duration = Duration::from_secs(3);
const CONTROLS_FADE: Duration = Duration::from_millis(200);
const MEMBERS_SLIDE: Duration = Duration::from_millis(250);
const PANEL: u32 = 0x0F0F11;
pub(super) const STOP_RED: u32 = 0xD22D39FF;
const MUTED_TINT: u32 = 0xD22D3933;
const POPOUT_TITLEBAR: f32 = 28.;
const POPOUT_CAPTION_WIDTH: f32 = 36.;
const POPOUT_WIDTH: f32 = 1080.;
const POPOUT_HEIGHT: f32 = 640.;

pub(crate) struct StreamView {
    user_id: u64,
    own: bool,
    nonce: u64,
    frame: Option<Arc<RenderImage>>,
    retired: Option<Arc<RenderImage>>,
    ended: Option<String>,
    height: Option<u32>,
    fps: u32,
    controls: StageControls,
    volume_open: bool,
    popout: Option<WindowHandle<StreamPopout>>,
    popout_opening: bool,
    popout_closed: Option<Subscription>,
}

/// It draws from the app, so it is only ever opened outside an update of the app: opening
/// draws it at once.
pub(crate) struct StreamPopout {
    app: WeakEntity<DiscordApp>,
    /// Everything that repaints the main window's stage repaints the overlays view.
    _overlays: Subscription,
    pinned: bool,
    pin_hovered: bool,
}

impl StreamPopout {
    fn toggle_pin(&mut self, window: &Window, cx: &mut Context<Self>) {
        if set_topmost(window, !self.pinned) {
            self.pinned = !self.pinned;
            cx.notify();
        }
    }

    /// Only Windows lets zcord raise a window above the others.
    fn pin_button(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if !cfg!(windows) {
            return None;
        }
        let pinned = self.pinned;
        Some(
            div()
                .id("stream-popout-pin")
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .w(rpx(POPOUT_CAPTION_WIDTH))
                .h_full()
                .cursor_pointer()
                // Occluding takes the button out of the drag area under it, so a press clicks
                // it instead of moving the window.
                .occlude()
                .hover(|style| style.bg(rgba(theme::hover())))
                .on_hover(cx.listener(|popout, hovered: &bool, _, cx| {
                    popout.pin_hovered = *hovered;
                    cx.notify();
                }))
                .on_click(cx.listener(|popout, _, window, cx| popout.toggle_pin(window, cx)))
                .child(theme::icon(
                    "pin",
                    16.,
                    if pinned {
                        theme::BLURPLE
                    } else {
                        theme::muted()
                    },
                ))
                .when(self.pin_hovered, |this| {
                    this.child(theme::tooltip_below(if pinned {
                        "Unpin"
                    } else {
                        "Pin to Top"
                    }))
                }),
        )
    }
}

#[cfg(windows)]
fn set_topmost(window: &Window, topmost: bool) -> bool {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        UI::WindowsAndMessaging::{
            HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
        },
    };
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return false;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return false;
    };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
    let after = if topmost {
        HWND_TOPMOST
    } else {
        HWND_NOTOPMOST
    };
    // SAFETY: the handle belongs to this live window, and only its z-order changes.
    unsafe {
        SetWindowPos(
            hwnd,
            Some(after),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    }
    .is_ok()
}

#[cfg(not(windows))]
fn set_topmost(_: &Window, _: bool) -> bool {
    false
}

impl Render for StreamPopout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let strip = div()
            .id("stream-popout-titlebar")
            .flex()
            .flex_none()
            .justify_end()
            .h(rpx(POPOUT_TITLEBAR))
            .w_full()
            .window_control_area(WindowControlArea::Drag)
            .when(!cfg!(target_os = "windows"), |this| {
                this.on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            })
            .children(self.pin_button(cx))
            .child(titlebar::caption_buttons(window, POPOUT_CAPTION_WIDTH));
        let stage = self
            .app
            .upgrade()
            .and_then(|app| popout_stage(app.read(cx), cx));
        if stage.is_none() {
            window.remove_window();
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x000000))
            .text_color(rgb(theme::text()))
            .text_size(rpx(16.))
            .child(strip)
            .child(div().relative().flex_1().min_h_0().children(stage))
    }
}

/// Updating a window fails while it is busy, as it is when the click that ends the stream
/// came from the popout itself.
fn close_popout(popout: WindowHandle<StreamPopout>, cx: &mut App) {
    cx.defer(move |cx| {
        popout
            .update(cx, |_, window, _| window.remove_window())
            .ok();
    });
}

pub(super) fn render_image(frame: StreamFrame) -> Option<Arc<RenderImage>> {
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

    pub(super) fn stage_open(&self) -> bool {
        self.stream_view.is_some() || self.voice.call_view.is_some() || self.voice_lobby.is_some()
    }

    pub(super) fn stage_channel(&self) -> Option<(u64, u64)> {
        if self.stream_view.is_some() || self.voice.call_view.is_some() {
            self.voice.joined_call()
        } else {
            self.voice_lobby
        }
    }

    pub(crate) fn stream_chat_open(&self) -> bool {
        self.stage_chat.is_some() && self.stage_open()
    }

    /// A call can end from places without a window, which leaves its chat to close later.
    pub(super) fn stage_chat_orphaned(&self) -> bool {
        self.stage_chat.is_some() && !self.stage_open()
    }

    /// Voice channels have their own text chat, which only opens from a stage.
    pub(super) fn open_stream_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((guild_id, channel_id)) = self.stage_channel() else {
            return;
        };
        let back = (self.selected_guild_id, self.selected_channel_id);
        if guild_id != 0 && self.selected_guild_id != guild_id {
            // Passing through the guild's text channel must not dismiss the stage.
            let stage = (self.voice.call_view.take(), self.voice_lobby.take());
            self.select_guild(guild_id, window, cx);
            (self.voice.call_view, self.voice_lobby) = stage;
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
        // Going back selects a text channel, which would otherwise dismiss the stage behind it.
        let stage = (self.voice.call_view.take(), self.voice_lobby.take());
        if guild_id == 0 {
            self.open_home(window, cx);
        } else {
            self.select_guild_channel(guild_id, Some(channel_id), window, cx);
        }
        (self.voice.call_view, self.voice_lobby) = stage;
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
            close_popout(popout, cx);
            self.views.overlays.update(cx, |_, cx| cx.notify());
            return;
        }
        if std::mem::replace(&mut view.popout_opening, true) {
            return;
        }
        let app = cx.entity();
        let overlays = self.views.overlays.clone();
        // Opening draws the window straight away, and the popout reads the app to draw, so it
        // has to wait until this update of the app is over.
        cx.defer(move |cx| {
            let weak = app.downgrade();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(POPOUT_WIDTH), px(POPOUT_HEIGHT)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Stream".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                window_decorations: cfg!(target_os = "linux").then_some(WindowDecorations::Client),
                ..Default::default()
            };
            let opened = cx.open_window(options, |_, cx| {
                cx.new(|cx| StreamPopout {
                    app: weak,
                    _overlays: cx.observe(&overlays, |_, _, cx| cx.notify()),
                    pinned: false,
                    pin_hovered: false,
                })
            });
            app.update(cx, |app, cx| {
                let entity = cx.entity().downgrade();
                let view = app.stream_view.as_mut();
                match (opened, view) {
                    (Ok(handle), Some(view)) => {
                        let window_id = handle.window_id();
                        view.popout_opening = false;
                        view.popout = Some(handle);
                        view.popout_closed = Some(cx.on_window_closed(move |cx, id| {
                            if id != window_id {
                                return;
                            }
                            entity
                                .update(cx, |app, cx| {
                                    if let Some(view) = app.stream_view.as_mut().filter(|view| {
                                        view.popout
                                            .as_ref()
                                            .is_some_and(|popout| popout.window_id() == id)
                                    }) {
                                        view.popout = None;
                                    }
                                    app.views.overlays.update(cx, |_, cx| cx.notify());
                                })
                                .ok();
                        }));
                    }
                    (Ok(handle), None) => close_popout(handle, cx),
                    (Err(error), view) => {
                        if let Some(view) = view {
                            view.popout_opening = false;
                        }
                        log::warn!("Could not open the stream window: {error}");
                    }
                }
                app.views.overlays.update(cx, |_, cx| cx.notify());
            });
        });
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
        let own = user_id == self.store.current_user_id;
        if own && !self.voice.sharing_screen() {
            return;
        }
        self.close_stream(window, cx);
        let frames = if own {
            session.share_frames()
        } else {
            session.watch_stream(user_id);
            session.stream_frames()
        };
        self.stream_nonce += 1;
        let nonce = self.stream_nonce;
        self.stream_view = Some(StreamView {
            user_id,
            own,
            nonce,
            frame: None,
            retired: None,
            ended: None,
            height: None,
            fps: 0,
            controls: StageControls::new(),
            volume_open: false,
            popout: None,
            popout_opening: false,
            popout_closed: None,
        });
        self.ensure_stream_volume(cx);
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
            close_popout(popout, cx);
        }
        if view.ended.is_none()
            && !view.own
            && let Some(session) = &self.session
        {
            session.stop_watching();
        }
        for image in [view.frame, view.retired].into_iter().flatten() {
            cx.drop_image(image, window.as_deref_mut());
        }
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    pub(crate) fn watching_own_stream(&self) -> bool {
        self.stream_view.as_ref().is_some_and(|view| view.own)
    }

    pub(crate) fn own_stream_ended(&mut self, cx: &mut Context<Self>) {
        if !self.watching_own_stream() {
            return;
        }
        if self.voice.call.is_some() {
            self.return_to_call(cx);
        } else {
            self.close_stream(None, cx);
        }
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
        .bg(rgb(theme::surface_highest()))
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
    bare_control(id, background)
        .on_hover(move |hovered, window, cx| {
            let key = (id, window.window_handle().window_id());
            entity
                .update(cx, |app, cx| {
                    if *hovered {
                        app.hovered_stage_control = Some(key);
                    } else if app.hovered_stage_control == Some(key) {
                        app.hovered_stage_control = None;
                    }
                    app.views.overlays.update(cx, |_, cx| cx.notify());
                })
                .ok();
        })
        .children(
            app.hovered_stage_control
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

pub(super) fn voice_controls(app: &DiscordApp, in_popout: bool) -> Div {
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

pub(super) fn disconnect_control(app: &DiscordApp) -> Stateful<Div> {
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

pub(super) fn share_control(app: &DiscordApp, in_popout: bool) -> Div {
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

pub(super) fn stage_area(
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

pub(super) fn render(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let view = app.stream_view.as_ref()?;
    let chat_open = app.stream_chat_open();
    let (area, chat_title) = stage(app, view, false, cx);
    Some(stage_frame(app, area, chat_title, chat_open))
}

fn popout_stage(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let view = app.stream_view.as_ref()?;
    Some(stage(app, view, true, cx).0.into_any_element())
}

fn stage(
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
    let members_shown = app.stream_members_shown;
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
    let top = if in_popout {
        POPOUT_TITLEBAR
    } else {
        titlebar::HEIGHT
    };
    let stage_area = stage_area(app, top, dock_height, chat_open)
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
    (stage_area, chat_title)
}
