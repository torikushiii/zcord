use std::{cell::Cell, rc::Rc};

use gpui::{prelude::*, *};
use gpui_component::ElementExt as _;

use crate::theme::{self, rpx};

pub(super) struct Mark {
    pub value: f32,
    pub label: Option<SharedString>,
}

impl Mark {
    pub fn labelled(value: f32, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: Some(label.into()),
        }
    }

    pub fn stop(value: f32) -> Self {
        Self { value, label: None }
    }
}

const HEIGHT: f32 = 48.;
const INSET: f32 = 5.;
const LABEL_WIDTH: f32 = 48.;
const GRABBER: f32 = 16.;
const BAR_TOP: f32 = 34.;
const BAR: f32 = 4.;

fn nearest(fractions: &[f32], fraction: f32) -> usize {
    fractions
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - fraction).abs().total_cmp(&(*b - fraction).abs()))
        .map_or(0, |(index, _)| index)
}

pub(super) fn marker_slider(
    id: &'static str,
    marks: Vec<Mark>,
    current: f32,
    default: Option<f32>,
    pick: impl Fn(usize, &mut App) + 'static,
) -> AnyElement {
    let (min, max) = marks
        .iter()
        .fold((f32::MAX, f32::MIN), |(low, high), mark| {
            (low.min(mark.value), high.max(mark.value))
        });
    let span = (max - min).max(f32::EPSILON);
    let fractions: Rc<[f32]> = marks.iter().map(|mark| (mark.value - min) / span).collect();
    let selected = nearest(&fractions, (current - min) / span);
    let at = fractions.get(selected).copied().unwrap_or(0.);
    let last = marks.len().saturating_sub(1);
    let default = default.map(|default| nearest(&fractions, (default - min) / span));

    let bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
    let measured = bounds.clone();
    let shown = Rc::new(Cell::new(selected));
    let choose = Rc::new({
        let fractions = fractions.clone();
        move |x: Pixels, window: &Window, cx: &mut App| {
            let Some(bar) = bounds.get() else {
                return;
            };
            let inset = rpx(INSET).to_pixels(window.rem_size());
            let travel = (bar.size.width - inset * 2.).max(px(1.));
            let fraction = ((x - bar.origin.x - inset) / travel).clamp(0., 1.);
            let index = nearest(&fractions, fraction);
            if index != shown.get() {
                shown.set(index);
                pick(index, cx);
            }
        }
    });
    let (down, moved) = (choose.clone(), choose);

    let anchor = |fraction: f32| div().absolute().top_0().left(relative(fraction));
    let ticks = marks.into_iter().enumerate().filter_map(|(index, mark)| {
        let label = mark.label?;
        let left = match index {
            0 => -INSET,
            index if index == last => INSET - LABEL_WIDTH,
            _ => -LABEL_WIDTH / 2.,
        };
        Some(
            anchor(fractions[index])
                .child(
                    div()
                        .absolute()
                        .left(rpx(left))
                        .w(rpx(LABEL_WIDTH))
                        .map(|this| match index {
                            0 => this.text_left(),
                            index if index == last => this.text_right(),
                            _ => this.text_center(),
                        })
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if default == Some(index) {
                            theme::text_positive()
                        } else {
                            theme::muted()
                        }))
                        .child(label),
                )
                .child(
                    div()
                        .absolute()
                        .top(rpx(24.))
                        .left(rpx(-1.))
                        .w(rpx(2.))
                        .h(rpx(24.))
                        .rounded(rpx(12.))
                        .bg(rgb(theme::slider_track())),
                ),
        )
    });
    let track = || {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(rpx(INSET))
            .right(rpx(INSET))
    };

    div()
        .id(id)
        .relative()
        .w_full()
        .h(rpx(HEIGHT))
        .cursor_pointer()
        .on_prepaint(move |bounds, _, _| measured.set(Some(bounds)))
        .child(track().children(ticks))
        .child(
            div()
                .absolute()
                .top(rpx(BAR_TOP))
                .left_0()
                .right_0()
                .h(rpx(BAR))
                .rounded(rpx(BAR))
                .overflow_hidden()
                .bg(rgb(theme::slider_track()))
                .child(
                    track().child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(rpx(-INSET))
                            .right(relative(1. - at))
                            .border_1()
                            .border_color(rgba(0xFFFFFF1A))
                            .bg(rgb(theme::BLURPLE)),
                    ),
                ),
        )
        .child(
            track().child(
                anchor(at).child(
                    div()
                        .absolute()
                        .top(rpx(BAR_TOP + BAR / 2. - GRABBER / 2.))
                        .left(rpx(-GRABBER / 2.))
                        .size(rpx(GRABBER))
                        .rounded_full()
                        .border_1()
                        .border_color(rgba(theme::border()))
                        .bg(rgb(theme::control_thumb()))
                        .shadow(vec![
                            BoxShadow::new(px(0.), px(1.), rgba(theme::control_shadow()).into())
                                .blur_radius(px(4.)),
                        ]),
                ),
            ),
        )
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            down(event.position.x, window, cx)
        })
        .on_mouse_move(move |event, window, cx| {
            if event.pressed_button == Some(MouseButton::Left) {
                moved(event.position.x, window, cx);
            }
        })
        .into_any_element()
}
