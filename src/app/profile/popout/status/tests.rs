use std::{cell::Cell, rc::Rc};

use gpui::{
    Bounds, Context, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, Pixels, Point,
    Render, TestAppContext, VisualTestContext, Window, div, point, prelude::*, px, size,
};

use super::{Event, Listener, Part, Props, State, flyout_position, render};
use crate::{
    app::profile::ProfilePalette,
    discord::{SelfStatus, StatusDuration},
    theme::rpx,
};

struct StatusSample {
    state: State,
    selected: Rc<Cell<Option<(SelfStatus, StatusDuration)>>>,
    origin: Point<Pixels>,
    enabled: bool,
    pending: bool,
}

impl StatusSample {
    fn new(selected: Rc<Cell<Option<(SelfStatus, StatusDuration)>>>) -> Self {
        Self {
            state: State::default(),
            selected,
            origin: point(px(32.), px(80.)),
            enabled: true,
            pending: false,
        }
    }
}

impl Render for StatusSample {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let listener: Listener = Rc::new(move |event, window, cx| {
            entity
                .update(cx, |sample, cx| {
                    if let Some(selection) = sample.state.handle(event, window, cx) {
                        sample.selected.set(Some(selection));
                    }
                    cx.notify();
                })
                .ok();
        });
        div().size_full().child(
            div()
                .absolute()
                .left(self.origin.x)
                .top(self.origin.y)
                .w(rpx(268.))
                .h(rpx(64.))
                .rounded(rpx(16.))
                .overflow_hidden()
                .debug_selector(|| "status-profile-sample".into())
                .child(render(
                    &self.state,
                    Props {
                        current: Some(SelfStatus::Idle),
                        pending: self.pending,
                        enabled: self.enabled,
                        error: None,
                        palette: ProfilePalette::of(None),
                    },
                    listener,
                    window,
                    cx,
                )),
        )
    }
}

fn draw(cx: &mut VisualTestContext) {
    // A surface's measured height repositions its anchored child on the next frame.
    for _ in 0..3 {
        cx.update(|window, cx| {
            window.refresh();
            let _ = window.draw(cx);
        });
    }
}

fn keys(cx: &mut VisualTestContext, sequence: &str) {
    for key in sequence.split(' ') {
        let keystroke = Keystroke::parse(key).unwrap();
        cx.simulate_event(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(KeyUpEvent { keystroke });
        draw(cx);
    }
}

#[test]
fn flyouts_prefer_right_then_flip_and_stay_inside_the_viewport() {
    let viewport = size(px(1000.), px(600.));
    let menu = size(px(300.), px(205.));
    let place = |x, y| {
        flyout_position(
            Bounds::new(point(px(x), px(y)), size(px(268.), px(36.))),
            menu,
            viewport,
            px(4.),
            px(8.),
            px(26.),
        )
    };
    assert_eq!(place(20., 40.), point(px(292.), px(32.)));
    assert_eq!(place(700., 550.), point(px(396.), px(387.)));
    assert_eq!(place(-400., -20.), point(px(8.), px(8.)));
}

#[test]
fn first_options_align_with_parent_chevrons_for_different_row_heights_and_zoom() {
    for zoom in [1., 1.25, 1.5] {
        for (row_height, first_row_offset) in [(36., 26.), (36., 25.), (50., 25.), (66., 25.)] {
            let parent = Bounds::new(
                point(px(100.), px(200.)),
                size(px(268. * zoom), px(row_height * zoom)),
            );
            let offset = px(first_row_offset * zoom);
            let position = flyout_position(
                parent,
                size(px(188. * zoom), px(220. * zoom)),
                size(px(1400.), px(900.)),
                px(4. * zoom),
                px(8. * zoom),
                offset,
            );
            assert_eq!(position.y + offset, parent.center().y);
        }
    }
}

#[gpui::test]
fn hovering_opens_both_flyouts_and_duration_clicks_escape_profile_clipping(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_component::init);
    let selected = Rc::new(Cell::new(None));
    let (_, cx) = cx.add_window_view(|_, _| StatusSample::new(selected.clone()));
    cx.simulate_resize(size(px(1100.), px(700.)));
    draw(cx);
    assert!(cx.debug_bounds("account-status-choices").is_none());
    assert!(cx.debug_bounds("account-status-durations").is_none());
    let trigger = cx.debug_bounds("account-status-trigger").unwrap();
    cx.simulate_mouse_move(trigger.center(), None, Modifiers::none());
    draw(cx);
    let profile = cx.debug_bounds("status-profile-sample").unwrap();
    let choices = cx.debug_bounds("account-status-choices").unwrap();
    assert!(choices.left() >= profile.right());
    let online = cx.debug_bounds("online").unwrap();
    assert!((online.center().y - trigger.center().y).abs() <= px(1.));
    let dnd = cx.debug_bounds("dnd").unwrap();
    cx.simulate_mouse_move(dnd.center(), None, Modifiers::none());
    draw(cx);
    let durations = cx.debug_bounds("account-status-durations").unwrap();
    assert!(durations.left() >= choices.right());
    let first_duration = cx.debug_bounds("For 15 Minutes").unwrap();
    assert!((first_duration.center().y - dnd.center().y).abs() <= px(1.));
    for duration in StatusDuration::ALL {
        let row = cx.debug_bounds(duration.label()).unwrap();
        assert!(durations.contains(&row.center()));
    }
    let row = cx.debug_bounds("For 8 Hours").unwrap();
    cx.simulate_click(row.center(), Modifiers::none());
    draw(cx);
    assert_eq!(
        selected.get(),
        Some((SelfStatus::DoNotDisturb, StatusDuration::Hours8))
    );
    assert!(cx.debug_bounds("account-status-choices").is_none());
    assert!(cx.debug_bounds("account-status-durations").is_none());
    assert!(cx.debug_bounds("account-status-trigger").is_some());
}

#[gpui::test]
fn keyboard_navigation_and_escape_preserve_the_parent_menu(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let selected = Rc::new(Cell::new(None));
    let (_, cx) = cx.add_window_view(|_, _| StatusSample::new(selected.clone()));
    draw(cx);
    let trigger = cx.debug_bounds("account-status-trigger").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::none());
    draw(cx);
    keys(cx, "down down right");
    draw(cx);
    assert!(cx.debug_bounds("account-status-durations").is_some());
    keys(cx, "escape");
    draw(cx);
    assert!(cx.debug_bounds("account-status-choices").is_some());
    assert!(cx.debug_bounds("account-status-durations").is_none());
    keys(cx, "right down enter");
    draw(cx);
    assert_eq!(
        selected.get(),
        Some((SelfStatus::DoNotDisturb, StatusDuration::Hour1))
    );
    assert!(cx.debug_bounds("account-status-choices").is_none());

    keys(cx, "space");
    draw(cx);
    assert!(cx.debug_bounds("account-status-choices").is_some());
    keys(cx, "escape");
    draw(cx);
    assert!(cx.debug_bounds("account-status-choices").is_none());
    assert!(cx.debug_bounds("account-status-trigger").is_some());
}

#[gpui::test]
fn online_activates_without_a_duration_and_outside_click_dismisses(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let selected = Rc::new(Cell::new(None));
    let (_, cx) = cx.add_window_view(|_, _| StatusSample::new(selected.clone()));
    draw(cx);
    let trigger = cx.debug_bounds("account-status-trigger").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::none());
    draw(cx);
    let online = cx.debug_bounds("online").unwrap();
    cx.simulate_click(online.center(), Modifiers::none());
    draw(cx);
    assert_eq!(
        selected.get(),
        Some((SelfStatus::Online, StatusDuration::Forever))
    );
    cx.simulate_click(trigger.center(), Modifiers::none());
    draw(cx);
    cx.simulate_click(point(px(4.), px(4.)), Modifiers::none());
    draw(cx);
    assert!(cx.debug_bounds("account-status-choices").is_none());
    assert!(cx.debug_bounds("account-status-trigger").is_some());
}

#[gpui::test]
fn flyouts_fit_at_zoom_and_flip_near_the_window_edges(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| StatusSample::new(Rc::default()));
    let viewport = size(px(1100.), px(700.));
    cx.simulate_resize(viewport);
    for zoom in [1., 1.25, 1.5] {
        for right in [false, true] {
            view.update(cx, |sample, cx| {
                sample.state.close();
                sample.origin = point(
                    if right {
                        px(1100. - 268. * zoom - 32.)
                    } else {
                        px(32.)
                    },
                    px(700. - 72. * zoom),
                );
                cx.notify();
            });
            cx.update(|window, _| window.set_rem_size(px(16. * zoom)));
            draw(cx);
            let trigger = cx.debug_bounds("account-status-trigger").unwrap();
            cx.simulate_click(trigger.center(), Modifiers::none());
            draw(cx);
            let choices = cx.debug_bounds("account-status-choices").unwrap();
            let idle = cx.debug_bounds("idle").unwrap();
            cx.simulate_mouse_move(idle.center(), None, Modifiers::none());
            draw(cx);
            let durations = cx.debug_bounds("account-status-durations").unwrap();
            for bounds in [choices, durations] {
                assert!(bounds.left() >= px(8. * zoom - 1.));
                assert!(bounds.top() >= px(8. * zoom - 1.));
                assert!(bounds.right() <= viewport.width - px(8. * zoom - 1.));
                assert!(bounds.bottom() <= viewport.height - px(8. * zoom - 1.));
            }
            assert!(choices.right() <= durations.left() || durations.right() <= choices.left());
            if right {
                assert!(
                    choices.right() <= trigger.left(),
                    "zoom={zoom}, trigger={trigger:?}, choices={choices:?}"
                );
            } else {
                assert!(choices.left() >= trigger.right());
            }
        }
    }
}

#[gpui::test]
fn disabled_or_submitting_status_cannot_activate(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let selected = Rc::new(Cell::new(None));
    let (view, cx) = cx.add_window_view(|_, _| StatusSample::new(selected.clone()));
    for pending in [false, true] {
        view.update(cx, |sample, cx| {
            sample.state.close();
            sample.enabled = false;
            sample.pending = pending;
            cx.notify();
        });
        draw(cx);
        let trigger = cx.debug_bounds("account-status-trigger").unwrap();
        cx.simulate_mouse_move(trigger.center(), None, Modifiers::none());
        cx.simulate_click(trigger.center(), Modifiers::none());
        keys(cx, "right enter");
        draw(cx);
        assert!(cx.debug_bounds("account-status-choices").is_none());
        assert_eq!(selected.get(), None);
    }
}

#[gpui::test]
fn stale_duration_bounds_do_not_keep_a_dismissed_flyout_alive(cx: &mut TestAppContext) {
    let mut state = State::default();
    let (_, cx) = cx.add_window_view(|_, _| StatusSample::new(Rc::default()));
    cx.update(|window, cx| {
        state.handle(Event::Hover(Part::Trigger, true), window, cx);
        assert!(state.open && !state.idle());
        state.handle(Event::Hover(Part::Trigger, false), window, cx);
        assert!(state.idle());
        state.handle(Event::Duration(SelfStatus::Idle, false), window, cx);
        let surface = Bounds::new(point(px(500.), px(80.)), size(px(188.), px(220.)));
        let mut bounds = state.bounds.get();
        bounds[2] = Some(surface);
        state.bounds.set(bounds);
        assert!(state.hit_test()(surface.center()));
        state.handle(Event::Dismiss, window, cx);
        assert!(state.open && state.duration.is_none());
        assert!(state.keyboard && !state.idle());
        assert!(!state.hit_test()(surface.center()));
        state.handle(Event::Dismiss, window, cx);
        assert!(!state.open);
    });
}
