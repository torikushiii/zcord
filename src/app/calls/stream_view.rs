mod controls;
mod stage;

pub(super) use controls::{
    control, controls_fade, disconnect_control, member_name, name_chip, share_control,
    show_chat_button, voice_controls,
};
use stage::popout_stage;
pub(crate) use stage::{render, stage_area, stage_frame};

use super::super::*;
use crate::app::calls::voice;
use crate::discord::{StreamEvent, StreamFrame};
use gpui_component::slider::{Slider, SliderEvent, SliderState};

const FRAME_POLL: Duration = Duration::from_millis(16);
const TILE_HEIGHT: f32 = 108.;
pub(super) const DOCK_HEIGHT: f32 = 72.;
pub(crate) const CHAT_WIDTH: f32 = 448.;
pub(super) const CHAT_GAP: f32 = 8.;
const VOLUME_MAX: f32 = 200.;
const CONTROLS_IDLE: Duration = Duration::from_secs(3);
const PANEL: u32 = 0x0F0F11;
pub(super) const STOP_RED: u32 = 0xD22D39FF;
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

pub(crate) struct StreamPopout {
    app: WeakEntity<DiscordApp>,
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
        match &mut self.stream.view {
            Some(view) => Some(&mut view.controls),
            None => self.voice.call_view.as_mut().map(|view| &mut view.controls),
        }
    }

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
            .stream
            .view
            .as_ref()
            .is_some_and(|view| view.volume_open || view.frame.is_none() || view.ended.is_some())
        {
            return;
        }
        let Some(controls) = self.stage_controls().filter(|controls| !controls.hidden) else {
            return;
        };
        controls.set_hidden(true);
        self.stream.hovered_control = None;
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    fn leave_stage(&mut self, cx: &mut Context<Self>) {
        if let Some(controls) = self.stage_controls() {
            controls.over = false;
        }
        self.hide_stage_controls(cx);
    }

    pub(super) fn stage_open(&self) -> bool {
        self.stream.view.is_some() || self.voice.call_view.is_some() || self.voice_lobby.is_some()
    }

    pub(crate) fn stage_channel(&self) -> Option<(u64, u64)> {
        if self.stream.view.is_some() || self.voice.call_view.is_some() {
            self.voice.joined_call()
        } else {
            self.voice_lobby
        }
    }

    pub(crate) fn stream_chat_open(&self) -> bool {
        self.stream.stage_chat.is_some() && self.stage_open()
    }

    pub(crate) fn stage_chat_orphaned(&self) -> bool {
        self.stream.stage_chat.is_some() && !self.stage_open()
    }

    pub(super) fn open_stream_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((guild_id, channel_id)) = self.stage_channel() else {
            return;
        };
        let back = (self.selected_guild_id, self.selected_channel_id);
        if guild_id != 0 && self.selected_guild_id != guild_id {
            let stage = (self.voice.call_view.take(), self.voice_lobby.take());
            self.select_guild(guild_id, window, cx);
            (self.voice.call_view, self.voice_lobby) = stage;
        }
        self.stream.stage_chat = Some(back);
        if guild_id == 0 {
            self.select_dm(channel_id, window, cx);
        } else {
            self.select_channel(channel_id, window, cx);
        }
    }

    pub(crate) fn close_stream_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((guild_id, channel_id)) = self.stream.stage_chat.take() else {
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
        if self.stream.volume.is_some() {
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
        self.stream.volume = Some((state, events));
    }

    fn toggle_stream_popout(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.stream.view.as_mut() else {
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
                let view = app.stream.view.as_mut();
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
                                    if let Some(view) = app.stream.view.as_mut().filter(|view| {
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
        self.stream.nonce += 1;
        let nonce = self.stream.nonce;
        self.stream.view = Some(StreamView {
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
                    let Some(view) = app.stream.view.as_mut().filter(|view| view.nonce == nonce)
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
        let Some(view) = self.stream.view.take() else {
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
        self.stream.view.as_ref().is_some_and(|view| view.own)
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

    fn return_to_call(&mut self, cx: &mut Context<Self>) {
        self.close_stream(None, cx);
        self.open_call_view(cx);
    }

    pub(crate) fn handle_stream_event(&mut self, event: StreamEvent, cx: &mut Context<Self>) {
        let StreamEvent::Ended { user_id, reason } = event else {
            return;
        };
        let Some(view) = self
            .stream
            .view
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
