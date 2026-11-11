use super::*;

const SPINNER_RADIUS: f32 = 7.;
const SPINNER_STROKE: f32 = 2.;
const SPINNER_CIRCUMFERENCE: f32 = 44.;
const SPINNER_LONGEST_GAP: f32 = 11.;
const SPINNER_TURN_SECS: f32 = 0.5;
const SPINNER_DASH_SECS: f32 = 0.8;
const SPINNER_STEPS: usize = 48;
const SPINNER_CAP_STEPS: usize = 8;

pub fn media_loading() -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(spinner(32., strong()))
}

pub fn spinner(size: f32, color: u32) -> impl IntoElement {
    static START: OnceLock<Instant> = OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            window.request_animation_frame();
            let unit = bounds.size.width / px(16.);
            let elapsed = start.elapsed().as_secs_f32();
            let cycle = (elapsed / SPINNER_DASH_SECS).rem_euclid(2.);
            let growth = if cycle <= 1. { cycle } else { 2. - cycle };
            let gap = SPINNER_LONGEST_GAP + (SPINNER_CIRCUMFERENCE - SPINNER_LONGEST_GAP) * growth;
            let sweep = (SPINNER_CIRCUMFERENCE - gap) / SPINNER_RADIUS;
            let from = (elapsed / SPINNER_TURN_SECS).fract() * TAU;
            let center = bounds.center();
            let at = |radial: f32, tangent: f32, angle: f32| {
                let (sin, cos) = angle.sin_cos();
                center
                    + point(
                        px((radial * cos - tangent * sin) * unit),
                        px((radial * sin + tangent * cos) * unit),
                    )
            };
            let half = SPINNER_STROKE / 2.;
            let step = |index: usize, steps: usize| index as f32 / steps as f32;
            let outline = (0..=SPINNER_STEPS)
                .map(|index| {
                    at(
                        SPINNER_RADIUS + half,
                        0.,
                        from + sweep * step(index, SPINNER_STEPS),
                    )
                })
                .chain((1..SPINNER_CAP_STEPS).map(|index| {
                    let (sin, cos) = (step(index, SPINNER_CAP_STEPS) * PI).sin_cos();
                    at(SPINNER_RADIUS + half * cos, half * sin, from + sweep)
                }))
                .chain((0..=SPINNER_STEPS).rev().map(|index| {
                    at(
                        SPINNER_RADIUS - half,
                        0.,
                        from + sweep * step(index, SPINNER_STEPS),
                    )
                }))
                .chain((1..SPINNER_CAP_STEPS).map(|index| {
                    let (sin, cos) = (step(index, SPINNER_CAP_STEPS) * PI).sin_cos();
                    at(SPINNER_RADIUS - half * cos, -half * sin, from)
                }));
            let mut path = PathBuilder::fill();
            for (index, position) in outline.enumerate() {
                if index == 0 {
                    path.move_to(position);
                } else {
                    path.line_to(position);
                }
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(color));
            }
        },
    )
    .flex_none()
    .size(rpx(size))
}

pub fn badge(count: u32, ring: u32) -> impl IntoElement {
    let label = if count >= 1000 {
        format!("{}K+", count / 1000)
    } else {
        count.to_string()
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .min_w(rpx(20.))
        .h(rpx(20.))
        .px(rpx(5.))
        .rounded_full()
        .bg(rgb(NOTIFICATION))
        .border_2()
        .border_color(rgb(ring))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white())
        .child(label)
}

pub fn app_tag(verified: bool) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(16.))
        .pl(rpx(if verified { 2. } else { 4. }))
        .pr(rpx(4.))
        .rounded(rpx(4.))
        .bg(rgb(BLURPLE))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(white())
        .when(verified, |this| {
            this.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(rpx(16.))
                    .child(icon("check-bold", 14., 0xFFFFFF)),
            )
        })
        .child("APP")
}

pub fn guild_tag(tag: &GuildTag, cx: &App) -> Div {
    let badge = tag
        .badge_spec()
        .and_then(|spec| images::resolve(&spec, image_px(12.), cx));
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(3.))
        .px(rpx(4.))
        .rounded(rpx(4.))
        .bg(rgba(selected()))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(strong()))
        .children(badge.map(|path| img(path).size(rpx(12.)).flex_none()))
        .child(tag.text.clone())
}

/// For clickable elements that stop their click from propagating: the click's mouse-up then
/// never reaches the window's text selection, which would keep extending a selection begun
/// by the same press. Suppressing selection on mouse-down avoids starting one at all.
pub trait WithoutTextSelection: InteractiveElement + Sized {
    fn without_text_selection(self) -> Self {
        self.on_mouse_down(MouseButton::Left, |_, _, cx| {
            gpui_base::GlobalState::suppress_text_selection(cx)
        })
    }
}

impl<E: InteractiveElement> WithoutTextSelection for E {}

/// A selection begun on the press that closes a dialog lands on the text beneath it.
pub fn selection_guard() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |_, _, window, _| {
            // Registered after gpui-component's own capture listener, which clears the flag at
            // the start of each press, so this runs later and the suppression holds.
            window.on_mouse_event(|_: &MouseDownEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture {
                    gpui_base::GlobalState::suppress_text_selection(cx);
                }
            });
        },
    )
    .absolute()
    .size_0()
}

pub fn icon_button(id: impl Into<ElementId>, name: &str, size: f32, color: u32) -> Stateful<Div> {
    icon_button_with_hover(id, name, size, color, rgba(hover()).into())
}

pub fn icon_button_with_hover(
    id: impl Into<ElementId>,
    name: &str,
    size: f32,
    color: u32,
    hover: Hsla,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(size + 8.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(icon(name, size, color))
}
