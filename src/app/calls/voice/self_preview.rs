use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};

use super::MenuKind;
use crate::{
    app::{DiscordApp, calls::stream_view},
    discord::FrameSlot,
    theme::{self, WithoutTextSelection, rpx},
};

const POLL: Duration = Duration::from_millis(16);
const WIDTH: f32 = 320.;
const HEIGHT: f32 = 180.;
const MARGIN: f32 = 16.;
const BOTTOM: f32 = 88.;
const TOP: f32 = 88.;
const LEFT: f32 = 80.;
const DRAG_THRESHOLD: f32 = 3.;
const FLING_MS: f32 = 200.;
const SETTLE: Duration = Duration::from_millis(300);
const BAR_HEIGHT: f32 = 44.;
const FADE: Duration = Duration::from_millis(200);
const PLAYER_BOUNDS: (u32, u32) = (640, 360);
const STAGE_BOUNDS: (u32, u32) = (1920, 1080);

#[derive(Default)]
pub(crate) struct SelfPreview {
    frame: Option<Arc<RenderImage>>,
    /// Freed once the next frame replaces it, so a frame is never released while on screen.
    retired: Option<Arc<RenderImage>>,
    requested: Option<(u32, u32)>,
    running: bool,
    hovered: bool,
    hover_changes: u64,
    corner: Corner,
    drag: Option<Drag>,
    dragged: bool,
    settle: Option<Settle>,
    settles: u64,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

impl Corner {
    fn nearest(point: Point<Pixels>, viewport: Size<Pixels>) -> Self {
        match (
            point.x > viewport.width / 2.,
            point.y > viewport.height / 2.,
        ) {
            (false, false) => Self::TopLeft,
            (true, false) => Self::TopRight,
            (false, true) => Self::BottomLeft,
            (true, true) => Self::BottomRight,
        }
    }

    fn origin(self, window: &Window) -> Point<Pixels> {
        let px = |value: f32| rpx(value).to_pixels(window.rem_size());
        let viewport = window.viewport_size();
        let x = match self {
            Self::TopLeft | Self::BottomLeft => px(LEFT),
            Self::TopRight | Self::BottomRight => viewport.width - px(MARGIN + WIDTH),
        };
        let y = match self {
            Self::TopLeft | Self::TopRight => px(TOP),
            Self::BottomLeft | Self::BottomRight => viewport.height - px(BOTTOM + HEIGHT),
        };
        point(x, y)
    }
}

#[derive(Clone, Copy)]
struct Drag {
    start: Point<Pixels>,
    grab: Point<Pixels>,
    origin: Point<Pixels>,
    moved: bool,
    last: Point<Pixels>,
    last_at: Instant,
    velocity: Point<f32>,
}

#[derive(Clone, Copy)]
struct Settle {
    id: u64,
    from: Point<Pixels>,
}

impl DiscordApp {
    fn start_self_preview_drag(
        &mut self,
        position: Point<Pixels>,
        origin: Point<Pixels>,
        cx: &mut App,
    ) {
        let preview = &mut self.voice.self_preview;
        preview.dragged = false;
        preview.drag = Some(Drag {
            start: position,
            grab: position - origin,
            origin,
            moved: false,
            last: position,
            last_at: Instant::now(),
            velocity: Point::default(),
        });
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    fn drag_self_preview(&mut self, position: Point<Pixels>, window: &Window, cx: &mut App) {
        let preview = &mut self.voice.self_preview;
        let Some(drag) = &mut preview.drag else {
            return;
        };
        let threshold = f32::from(rpx(DRAG_THRESHOLD).to_pixels(window.rem_size()));
        let from_start = position - drag.start;
        let (dx, dy) = (f32::from(from_start.x), f32::from(from_start.y));
        if !drag.moved && dx * dx + dy * dy <= threshold * threshold {
            return;
        }
        drag.moved = true;
        preview.dragged = true;
        let now = Instant::now();
        let elapsed = now.duration_since(drag.last_at).as_secs_f32() * 1000.;
        if elapsed > 0. {
            let step = position - drag.last;
            drag.velocity = point(f32::from(step.x) / elapsed, f32::from(step.y) / elapsed);
            drag.last = position;
            drag.last_at = now;
        }
        let viewport = window.viewport_size();
        let player = |value: f32| f32::from(rpx(value).to_pixels(window.rem_size()));
        let max_x = (f32::from(viewport.width) - player(WIDTH)).max(0.);
        let max_y = (f32::from(viewport.height) - player(HEIGHT)).max(0.);
        let origin = position - drag.grab;
        drag.origin = point(
            px(f32::from(origin.x).clamp(0., max_x)),
            px(f32::from(origin.y).clamp(0., max_y)),
        );
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    fn release_self_preview(&mut self, position: Point<Pixels>, window: &Window, cx: &mut App) {
        let preview = &mut self.voice.self_preview;
        let Some(drag) = preview.drag.take() else {
            return;
        };
        if !drag.moved {
            self.views.overlays.update(cx, |_, cx| cx.notify());
            return;
        }
        let velocity = if drag.last_at.elapsed() > Duration::from_millis(100) {
            Point::default()
        } else {
            drag.velocity
        };
        let projected = position + point(px(velocity.x * FLING_MS), px(velocity.y * FLING_MS));
        preview.corner = Corner::nearest(projected, window.viewport_size());
        if !cx.reduce_motion() {
            preview.settles += 1;
            let id = preview.settles;
            preview.settle = Some(Settle {
                id,
                from: drag.origin,
            });
            let app = self.entity.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(SETTLE).await;
                app.update(cx, |app, cx| {
                    let preview = &mut app.voice.self_preview;
                    if preview.settle.is_some_and(|settle| settle.id == id) {
                        preview.settle = None;
                        app.views.overlays.update(cx, |_, cx| cx.notify());
                    }
                })
                .ok();
            })
            .detach();
        }
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    fn self_preview_shown(&self) -> bool {
        self.voice.sharing_screen() && !self.stage_open()
    }

    fn hover_self_preview(&mut self, hovered: bool, cx: &mut Context<Self>) {
        let preview = &mut self.voice.self_preview;
        if preview.hovered != hovered {
            preview.hovered = hovered;
            preview.hover_changes += 1;
            self.views.overlays.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn start_self_preview(&mut self, cx: &mut Context<Self>) {
        if self.voice.self_preview.running {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.voice.self_preview.running = true;
        let frames = session.share_frames();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let alive = this.update(cx, |app, cx| app.tick_self_preview(&frames, cx));
                if !matches!(alive, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    fn tick_self_preview(&mut self, frames: &FrameSlot, cx: &mut Context<Self>) -> bool {
        if !self.voice.sharing_screen() {
            self.end_self_preview(cx);
            return false;
        }
        let playing = self.self_preview_shown() && self.window_active;
        let wanted = if self.watching_own_stream() {
            Some(STAGE_BOUNDS)
        } else {
            playing.then_some(PLAYER_BOUNDS)
        };
        let mut changed = false;
        // A player that vanishes under the pointer never hears it leave.
        if !self.self_preview_shown() {
            self.voice.self_preview.hovered = false;
            self.voice.self_preview.drag = None;
        }
        if self.voice.self_preview.requested != wanted {
            self.voice.self_preview.requested = wanted;
            if let Some(session) = &self.session {
                session.set_share_preview(wanted);
            }
            changed = true;
        }
        if playing
            && let Some(image) = frames
                .lock()
                .ok()
                .and_then(|mut slot| slot.take())
                .and_then(stream_view::render_image)
        {
            let preview = &mut self.voice.self_preview;
            let retired = preview.retired.take();
            preview.retired = preview.frame.replace(image);
            if let Some(retired) = retired {
                cx.drop_image(retired, None);
            }
            changed = true;
        }
        if changed {
            self.views.overlays.update(cx, |_, cx| cx.notify());
        }
        true
    }

    fn end_self_preview(&mut self, cx: &mut Context<Self>) {
        let preview = &mut self.voice.self_preview;
        preview.running = false;
        preview.hovered = false;
        preview.drag = None;
        preview.settle = None;
        if preview.requested.take().is_some()
            && let Some(session) = &self.session
        {
            session.set_share_preview(None);
        }
        for image in [preview.frame.take(), preview.retired.take()]
            .into_iter()
            .flatten()
        {
            cx.drop_image(image, None);
        }
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    fn open_own_stream(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let user_id = self.store.current_user_id;
        self.watch_stream(user_id, Some(window), cx);
    }

    fn open_self_preview_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.toggle_menu(MenuKind::Stream, position, cx);
        let corner = self.voice.self_preview.corner;
        if let Some(menu) = self.voice.menu.as_mut() {
            menu.flyout_left = matches!(corner, Corner::TopRight | Corner::BottomRight);
        }
    }
}

fn notice(title: &'static str, detail: Option<&'static str>) -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .px(rpx(16.))
        .text_center()
        .text_color(white())
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .children(detail.map(|detail| {
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::MEDIUM)
                .child(detail)
        }))
}

fn bar_button(
    id: &'static str,
    tip_id: &'static str,
    icon: &'static str,
    tip: &'static str,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(32.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(0xFFFFFF1F)))
        .child(theme::icon(icon, 24., 0xFFFFFF))
        .child(theme::hover_tooltip(tip_id, tip))
}

fn bar(top: bool) -> Div {
    div()
        .absolute()
        .left_0()
        .right_0()
        .when(top, |this| this.top_0())
        .when(!top, |this| this.bottom_0())
        .flex()
        .items_center()
        .gap_1()
        .h(rpx(BAR_HEIGHT))
        .px(rpx(8.))
        .bg(linear_gradient(
            if top { 180. } else { 0. },
            linear_color_stop(rgba(0x000000B3), 0.),
            linear_color_stop(rgba(0x00000000), 1.),
        ))
}

fn drag_listeners(app: &DiscordApp) -> impl IntoElement {
    let moved = app.entity.clone();
    let released = app.entity.clone();
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let moved = moved.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                let position = event.position;
                moved
                    .update(cx, |app, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            app.drag_self_preview(position, window, cx);
                        } else {
                            app.release_self_preview(position, window, cx);
                        }
                    })
                    .ok();
            });
            let released = released.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                    let position = event.position;
                    released
                        .update(cx, |app, cx| app.release_self_preview(position, window, cx))
                        .ok();
                }
            });
        },
    )
    .absolute()
    .size_0()
}

pub(crate) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    if !app.self_preview_shown() {
        return None;
    }
    let (guild_id, channel_id) = app.voice.joined_call()?;
    let preview = &app.voice.self_preview;
    let paused = !app.window_active;
    let channel = if guild_id == 0 {
        app.store.private_channel_name(channel_id)
    } else {
        app.store
            .channel_name(channel_id)
            .unwrap_or_default()
            .to_owned()
    };
    let picture = match (&preview.frame, paused) {
        (_, true) => notice(
            "Your stream is still running!",
            Some("We\u{2019}ve paused this preview to save your resources."),
        )
        .into_any_element(),
        (Some(frame), false) => img(frame.clone())
            .size_full()
            .object_fit(ObjectFit::Contain)
            .into_any_element(),
        (None, false) => notice("Generating preview\u{2026}", None).into_any_element(),
    };
    let back = app.entity.clone();
    let header = bar(true).child(
        div()
            .id("self-preview-back")
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .h(rpx(32.))
            .px(rpx(4.))
            .rounded(rpx(8.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(0xFFFFFF1F)))
            .text_size(rpx(16.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(white())
            .on_click(move |_, window, cx| {
                back.update(cx, |app, cx| {
                    if !app.voice.self_preview.dragged {
                        app.open_own_stream(window, cx);
                    }
                })
                .ok();
            })
            .child(theme::icon("arrow-left", 20., 0xFFFFFF))
            .child(div().truncate().child(channel)),
    );
    let settings = app.entity.clone();
    let stop = app.entity.clone();
    let name =
        (guild_id != 0).then(|| stream_view::member_name(app, guild_id, app.store.current_user_id));
    let footer = bar(false)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pl(rpx(4.))
                .truncate()
                .text_size(rpx(15.))
                .text_color(white())
                .children(name),
        )
        .child(
            bar_button(
                "self-preview-settings",
                "self-preview-settings-tip",
                "settings",
                "Stream Settings",
            )
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                cx.stop_propagation();
                let position = event.position;
                settings
                    .update(cx, |app, cx| app.open_self_preview_menu(position, cx))
                    .ok();
            }),
        )
        .child(
            bar_button(
                "self-preview-stop",
                "self-preview-stop-tip",
                "monitor-x",
                "Stop Streaming",
            )
            .on_click(move |_, _, cx| {
                stop.update(cx, |app, cx| {
                    if !app.voice.self_preview.dragged {
                        app.stop_share(cx);
                    }
                })
                .ok();
            }),
        );
    let controls = div().absolute().inset_0().child(header).child(footer);
    let shown = preview.hovered;
    let controls = if preview.hover_changes == 0 || cx.reduce_motion() {
        controls
            .opacity(if shown { 1. } else { 0. })
            .into_any_element()
    } else {
        controls
            .with_animation(
                ("self-preview-controls", preview.hover_changes),
                Animation::new(FADE).with_easing(ease_out_quint()),
                move |this, delta| this.opacity(if shown { delta } else { 1. - delta }),
            )
            .into_any_element()
    };
    let hover = app.entity.clone();
    let press = app.entity.clone();
    let rest = preview.corner.origin(window);
    let dragging = preview.drag.filter(|drag| drag.moved);
    let origin = dragging.map_or(rest, |drag| drag.origin);
    let player = div()
        .id("self-preview")
        .on_hover(move |hovered, _, cx| {
            let hovered = *hovered;
            hover
                .update(cx, |app, cx| app.hover_self_preview(hovered, cx))
                .ok();
        })
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            let position = event.position;
            press
                .update(cx, |app, cx| {
                    app.start_self_preview_drag(position, origin, cx)
                })
                .ok();
        })
        .without_text_selection()
        .absolute()
        .left(origin.x)
        .top(origin.y)
        .w(rpx(WIDTH))
        .h(rpx(HEIGHT))
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(rgb(0x000000))
        .border_1()
        .border_color(rgba(theme::border_subtle()))
        .shadow_lg()
        .occlude()
        .child(picture)
        .child(controls)
        .when(preview.drag.is_some(), |this| {
            this.child(drag_listeners(app))
        });
    Some(match preview.settle.filter(|_| dragging.is_none()) {
        Some(Settle { id, from }) => player
            .with_animation(
                ("self-preview-settle", id),
                Animation::new(SETTLE).with_easing(ease_out_quint()),
                move |this, delta| {
                    let lerp = |from: Pixels, to: Pixels| from + (to - from) * delta;
                    this.left(lerp(from.x, rest.x)).top(lerp(from.y, rest.y))
                },
            )
            .into_any_element(),
        None => player.into_any_element(),
    })
}
