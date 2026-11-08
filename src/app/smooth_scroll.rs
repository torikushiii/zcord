use std::time::{Duration, Instant};

use gpui::{prelude::*, *};

use crate::{settings, theme::rpx};

const LINE_HEIGHT: f32 = 100. / 3.;
const GRID_PX: f32 = 1. / 1024.;
const EASE_IN_OUT: [f64; 4] = [0.42, 0.0, 0.58, 1.0];
const EPSILON_PX: f64 = 0.01;

#[derive(Default)]
struct State {
    glide: Glide,
    last_frame: Option<Instant>,
    running: bool,
    timings: Timings,
}

impl Global for State {}

#[derive(Default)]
struct Timings {
    frames: u32,
    slow_frames: u32,
    total_gap: Duration,
    max_gap: Duration,
    max_dispatch: Duration,
}

impl Timings {
    fn record(&mut self, gap: Duration, dispatch: Duration, done: bool) {
        self.frames += 1;
        self.slow_frames += u32::from(gap > Duration::from_millis(25));
        self.total_gap += gap;
        self.max_gap = self.max_gap.max(gap);
        self.max_dispatch = self.max_dispatch.max(dispatch);
        if done || self.total_gap >= Duration::from_secs(1) {
            log::debug!(
                target: "zcord::perf",
                "scroll frames={} gaps_over_25ms={} mean_gap_ms={:.2} max_gap_ms={:.2} max_dispatch_ms={:.2}",
                self.frames, self.slow_frames,
                self.total_gap.as_secs_f64() * 1000. / f64::from(self.frames),
                self.max_gap.as_secs_f64() * 1000.,
                self.max_dispatch.as_secs_f64() * 1000.,
            );
            *self = Self::default();
        }
    }
}

pub fn interceptor() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |_, _, window, _cx| {
            window.on_mouse_event(|_: &MouseDownEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    cancel(cx);
                }
            });
            window.on_mouse_event(|event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture
                    || cx.reduce_motion()
                    || !settings::smooth_scroll()
                {
                    return;
                }
                let ScrollDelta::Lines(lines) = event.delta else {
                    return;
                };
                if lines.y == 0.
                    || lines.x != 0.
                    || event.modifiers.shift
                    || event.modifiers.control
                    || event.modifiers.alt
                    || event.modifiers.platform
                {
                    return;
                }
                cx.stop_propagation();
                let distance = lines.y * rpx(LINE_HEIGHT).to_pixels(window.rem_size()).as_f32();
                let now = Instant::now();
                let state = cx.default_global::<State>();
                state.glide.push(distance, now);
                if !state.running {
                    state.running = true;
                    state.last_frame = Some(now);
                    window.on_next_frame(step);
                }
            });
        },
    )
    .absolute()
    .size_0()
}

pub fn cancel(cx: &mut App) {
    cx.default_global::<State>().glide.stop();
}

fn step(window: &mut Window, cx: &mut App) {
    let now = Instant::now();
    let state = cx.default_global::<State>();
    let elapsed = state.last_frame.map_or(Duration::ZERO, |last| now - last);
    state.last_frame = Some(now);
    let delta = state.glide.advance(now);
    let done = !state.glide.active();
    if done {
        state.running = false;
        state.last_frame = None;
    }
    let tracing = log::log_enabled!(target: "zcord::perf", log::Level::Debug);
    let dispatch_start = tracing.then(Instant::now);
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
    if let Some(start) = dispatch_start {
        cx.default_global::<State>()
            .timings
            .record(elapsed, start.elapsed(), done);
    }
    if !done {
        window.on_next_frame(step);
    }
}

#[derive(Default)]
struct Glide {
    pending: f32,
    segment: Option<Segment>,
}

impl Glide {
    fn push(&mut self, delta: f32, now: Instant) {
        let delta = snap(delta);
        let Some(segment) = self.segment.take() else {
            self.pending = delta;
            self.segment = Some(Segment::from_rest(delta, now));
            return;
        };
        let left = segment.remaining(now);
        if segment.delta * delta < 0.0 {
            self.pending = delta;
            self.segment = Some(Segment::from_rest(delta, now));
            return;
        }
        self.pending += delta;
        let target = left + delta;
        self.segment = Some(segment.retarget(target, now, duration(target)));
    }

    fn advance(&mut self, now: Instant) -> f32 {
        let Some(segment) = &self.segment else {
            return 0.;
        };
        if segment.time(now) >= 1.0 {
            return self.finish();
        }
        let left = segment.remaining(now);
        let slice = snap(self.pending - left);
        self.pending -= slice;
        slice
    }

    fn finish(&mut self) -> f32 {
        let rest = self.pending;
        self.stop();
        rest
    }

    fn stop(&mut self) {
        self.pending = 0.;
        self.segment = None;
    }

    fn active(&self) -> bool {
        self.segment.is_some()
    }
}

fn snap(pixels: f32) -> f32 {
    (pixels / GRID_PX).round() * GRID_PX
}

fn duration(delta: f32) -> f64 {
    let px = f64::from(delta).abs();
    let frames = 12.0 - (px - 120.0) * (12.0 - 6.0) / (480.0 - 120.0);
    frames.clamp(6.0, 12.0) / 60.0
}

struct Segment {
    delta: f32,
    start: Instant,
    duration: f64,
    points: [f64; 4],
    ease: Bezier,
}

impl Segment {
    fn new(
        delta: f32,
        start: Instant,
        duration: f64,
        points: [f64; 4],
        slope: Option<f64>,
    ) -> Self {
        let [x1, y1, x2, y2] = points;
        let y1 = slope.map_or(y1, |slope| x1 * slope.clamp(-1000.0, 1000.0));
        Self {
            delta,
            start,
            duration,
            points,
            ease: Bezier::new(x1, y1, x2, y2),
        }
    }

    fn from_rest(delta: f32, start: Instant) -> Self {
        Self::new(delta, start, duration(delta), EASE_IN_OUT, None)
    }

    fn time(&self, now: Instant) -> f64 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.start).as_secs_f64();
        (elapsed / self.duration).min(1.0)
    }

    fn remaining(&self, now: Instant) -> f32 {
        let left = 1.0 - self.ease.value(self.time(now));
        (f64::from(self.delta) * left) as f32
    }

    fn velocity(&self, now: Instant) -> f64 {
        if self.duration <= 0.0 {
            return 0.0;
        }
        self.ease.slope(self.time(now)) * f64::from(self.delta) / self.duration
    }

    fn retarget(&self, delta: f32, now: Instant, duration: f64) -> Self {
        let distance = f64::from(delta);
        if distance.abs() < EPSILON_PX {
            return Self::new(delta, now, 0.0, self.points, None);
        }
        let velocity = self.velocity(now);
        let bound = (velocity.abs() >= EPSILON_PX)
            .then(|| distance / velocity * 2.5)
            .filter(|bound| *bound >= 0.0);
        let duration = bound.map_or(duration, |bound| duration.min(bound));
        if duration < 0.01 {
            return Self::new(delta, now, 0.0, self.points, None);
        }
        let slope = velocity * duration / distance;
        Self::new(delta, now, duration, self.points, Some(slope))
    }
}

struct Bezier {
    ax: f64,
    bx: f64,
    cx: f64,
    ay: f64,
    by: f64,
    cy: f64,
}

impl Bezier {
    const EPSILON: f64 = 1e-7;

    fn new(x1: f64, y1: f64, x2: f64, y2: f64) -> Self {
        let (cx, cy) = (3.0 * x1, 3.0 * y1);
        let (bx, by) = (3.0 * (x2 - x1) - cx, 3.0 * (y2 - y1) - cy);
        Self {
            ax: 1.0 - cx - bx,
            bx,
            cx,
            ay: 1.0 - cy - by,
            by,
            cy,
        }
    }

    fn x(&self, t: f64) -> f64 {
        ((self.ax * t + self.bx) * t + self.cx) * t
    }

    fn y(&self, t: f64) -> f64 {
        ((self.ay * t + self.by) * t + self.cy) * t
    }

    fn dx(&self, t: f64) -> f64 {
        (3.0 * self.ax * t + 2.0 * self.bx) * t + self.cx
    }

    fn dy(&self, t: f64) -> f64 {
        (3.0 * self.ay * t + 2.0 * self.by) * t + self.cy
    }

    fn solve(&self, x: f64) -> f64 {
        let mut t = x;
        for _ in 0..4 {
            let error = self.x(t) - x;
            if error.abs() < Self::EPSILON {
                return t;
            }
            let slope = self.dx(t);
            if slope.abs() < Self::EPSILON {
                break;
            }
            t -= error / slope;
        }
        let (mut low, mut high) = (0.0, 1.0);
        t = x;
        for _ in 0..64 {
            let at = self.x(t);
            if (at - x).abs() < Self::EPSILON {
                break;
            }
            if at < x {
                low = t;
            } else {
                high = t;
            }
            t = (low + high) / 2.0;
        }
        t
    }

    fn value(&self, x: f64) -> f64 {
        self.y(self.solve(x.clamp(0.0, 1.0)))
    }

    fn slope(&self, x: f64) -> f64 {
        let t = self.solve(x.clamp(0.0, 1.0));
        let (dx, dy) = (self.dx(t), self.dy(t));
        if dx == 0.0 && dy == 0.0 { 0.0 } else { dy / dx }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Bezier, EASE_IN_OUT, GRID_PX, Glide, Segment, duration};

    const FRAME: Duration = Duration::from_millis(8);

    fn land(glide: &mut Glide, now: &mut Instant, frame: Duration) -> Vec<f32> {
        let mut slices = Vec::new();
        while glide.active() {
            *now += frame;
            slices.push(glide.advance(*now));
            assert!(slices.len() < 500, "the glide never landed");
        }
        slices
    }

    fn total(slices: &[f32]) -> f32 {
        slices.iter().sum()
    }

    #[test]
    fn a_notch_eases_in_and_out_and_lands_on_exactly_its_pixels() {
        let start = Instant::now();
        let mut now = start;
        let mut glide = Glide::default();
        glide.push(100.0, now);
        let leg = glide.segment.as_ref().unwrap();
        let half = leg.remaining(start + Duration::from_millis(100));
        assert!((half - 50.0).abs() < 1e-4, "{half:?}");

        let slices = land(&mut glide, &mut now, FRAME);
        assert_eq!(total(&slices), 100.0);
        assert_eq!(slices.len(), 25);
        let speeds = slices;
        let peak = (0..speeds.len())
            .max_by(|a, b| speeds[*a].total_cmp(&speeds[*b]))
            .unwrap();
        assert!((11..=13).contains(&peak), "fastest at frame {peak}");
        assert!(
            speeds[..=peak]
                .windows(2)
                .all(|pair| pair[1] >= pair[0] - GRID_PX)
        );
        assert!(
            speeds[peak..]
                .windows(2)
                .all(|pair| pair[1] <= pair[0] + GRID_PX)
        );
        assert!(speeds[0] > 0.0 && speeds[0] < speeds[peak] / 10.0);
        assert!(speeds[24] > 0.0 && speeds[24] < speeds[peak] / 10.0);
    }

    #[test]
    fn longer_moves_take_less_time_as_in_chrome() {
        for (pixels, ms) in [
            (100.0, 200.0),
            (120.0, 200.0),
            (300.0, 150.0),
            (480.0, 100.0),
            (1200.0, 100.0),
        ] {
            let took = duration(pixels) * 1000.0;
            assert!((took - ms).abs() < 1e-9, "{pixels}px took {took}ms");
        }
    }

    #[test]
    fn a_retarget_matches_chromiums_own_numbers() {
        let points = [0.4, 0.0, 0.0, 1.0];
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let first = Segment::new(3600.0, start, 0.15, points, None);
        let left = first.remaining(at(75));
        assert!((3600.0 - left - 3101.4792).abs() < 0.002, "{left:?}");

        let second = first.retarget(left + 6300.0, at(75), 0.15);
        assert!((second.duration - 0.15).abs() < 0.0002);
        let position = |ms| 9900.0 - second.remaining(at(ms));
        assert!((position(150) - 9097.8).abs() < 0.1, "{}", position(150));
        assert!((position(225) - 9900.0).abs() < 0.0002);
    }

    #[test]
    fn the_ease_hits_both_endpoints_and_its_midpoint() {
        let ease = Bezier::new(
            EASE_IN_OUT[0],
            EASE_IN_OUT[1],
            EASE_IN_OUT[2],
            EASE_IN_OUT[3],
        );
        assert_eq!(ease.value(0.0), 0.0);
        assert_eq!(ease.value(1.0), 1.0);
        assert!((ease.value(0.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_notch_in_flight_keeps_the_glide_at_speed() {
        let mut now = Instant::now();
        let mut glide = Glide::default();
        glide.push(100.0, now);
        for _ in 0..6 {
            now += FRAME;
            glide.advance(now);
        }
        let before = glide.segment.as_ref().unwrap().velocity(now);
        glide.push(100.0, now);
        let after = glide.segment.as_ref().unwrap().velocity(now);
        assert!((after - before).abs() < 1e-6, "{after} after {before}");
    }

    #[test]
    fn notches_in_flight_merge_into_one_glide() {
        let mut now = Instant::now();
        let mut glide = Glide::default();
        glide.push(100.0, now);
        let mut delivered = 0.0;
        for _ in 0..3 {
            now += FRAME;
            delivered += glide.advance(now);
        }
        glide.push(100.0, now);
        delivered += total(&land(&mut glide, &mut now, FRAME));
        assert_eq!(delivered, 200.0);
    }

    #[test]
    fn reversing_drops_the_rest_of_the_old_direction() {
        let mut now = Instant::now();
        let mut glide = Glide::default();
        glide.push(100.0, now);
        now += FRAME;
        let first = glide.advance(now);
        assert!(first > 0.0 && first < 100.0);
        glide.push(-50.0, now);
        assert_eq!(glide.pending, -50.0);
        let rest = total(&land(&mut glide, &mut now, FRAME));
        assert_eq!(rest, -50.0);
    }

    #[test]
    fn the_curve_does_not_depend_on_frame_rate() {
        let position = |frame: Duration, frames: u32| {
            let mut now = Instant::now();
            let mut glide = Glide::default();
            glide.push(100.0, now);
            (0..frames)
                .map(|_| {
                    now += frame;
                    glide.advance(now)
                })
                .sum::<f32>()
        };
        let at_60hz = position(Duration::from_millis(16), 3);
        let at_120hz = position(FRAME, 6);
        assert!(
            (at_60hz - at_120hz).abs() < 0.002,
            "{at_60hz} vs {at_120hz}"
        );
    }
}
