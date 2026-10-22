use std::time::{Duration, Instant};

use gpui::{prelude::*, *};

use crate::theme::rpx;

const LINE_HEIGHT: f32 = 100. / 3.;
const TIME_CONSTANT: f32 = 0.055;
const MAX_FRAME: Duration = Duration::from_millis(50);

#[derive(Default)]
struct State {
    remaining: f32,
    last_frame: Option<Instant>,
    running: bool,
}

impl Global for State {}

pub fn interceptor() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |_, _, window, _| {
            window.on_mouse_event(|event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture || cx.reduce_motion() {
                    return;
                }
                let ScrollDelta::Lines(lines) = event.delta else {
                    return;
                };
                if lines.y == 0.
                    || lines.x != 0.
                    || event.modifiers.shift
                    || event.modifiers.control
                {
                    return;
                }
                cx.stop_propagation();
                let distance = lines.y * rpx(LINE_HEIGHT).to_pixels(window.rem_size()).as_f32();
                let state = cx.default_global::<State>();
                if state.remaining.signum() == distance.signum() {
                    state.remaining += distance;
                } else {
                    state.remaining = distance;
                }
                if !state.running {
                    state.running = true;
                    state.last_frame = Some(Instant::now());
                    window.on_next_frame(step);
                }
            });
        },
    )
    .absolute()
    .size_0()
}

pub fn cancel(cx: &mut App) {
    let state = cx.default_global::<State>();
    state.remaining = 0.;
}

fn step(window: &mut Window, cx: &mut App) {
    let now = Instant::now();
    let state = cx.default_global::<State>();
    let elapsed = state
        .last_frame
        .map_or(Duration::ZERO, |last| now - last)
        .min(MAX_FRAME);
    state.last_frame = Some(now);
    let fraction = 1. - (-elapsed.as_secs_f32() / TIME_CONSTANT).exp();
    let mut delta = state.remaining * fraction;
    if (state.remaining - delta).abs() < 0.5 {
        delta = state.remaining;
    }
    state.remaining -= delta;
    let done = state.remaining == 0.;
    if done {
        state.running = false;
        state.last_frame = None;
    }
    if delta != 0. {
        window.dispatch_event(
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: window.mouse_position(),
                delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                modifiers: window.modifiers(),
                touch_phase: TouchPhase::Moved,
            }),
            cx,
        );
    }
    if !done {
        window.on_next_frame(step);
    }
}
