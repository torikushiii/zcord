use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gpui::{prelude::*, *};

use crate::{
    app::DiscordApp,
    discord::{SelfStatus, StatusDuration},
    theme::{self, WithoutTextSelection, rpx},
};

use super::super::{ProfileCard, ProfilePalette};

const CHOICES_WIDTH: f32 = 300.;
const DURATION_WIDTH: f32 = 188.;
const CLOSE_DELAY: Duration = Duration::from_millis(180);

#[derive(Clone, Copy)]
enum Part {
    Trigger,
    Choices,
    Durations,
}

#[derive(Clone, Copy)]
enum Event {
    Hover(Part, bool),
    Open(bool),
    Duration(SelfStatus, bool),
    FocusChoice(usize),
    FocusDuration(usize),
    Dismiss,
    Outside,
    Activate(SelfStatus, StatusDuration),
}

type Listener = Rc<dyn Fn(Event, &mut Window, &mut App)>;

#[derive(Default)]
pub(in crate::app::profile) struct State {
    open: bool,
    duration: Option<SelfStatus>,
    keyboard: bool,
    hovered: [bool; 3],
    // Trigger, two surfaces, four status rows, then the first duration row.
    bounds: Rc<Cell<[Option<Bounds<Pixels>>; 8]>>,
    focus: RefCell<Option<[FocusHandle; 11]>>,
    close_task: Option<Task<()>>,
}

impl State {
    pub(in crate::app::profile) fn hit_test(&self) -> impl Fn(Point<Pixels>) -> bool + use<> {
        let bounds = self.bounds.clone();
        move |point| {
            bounds.get()[..3]
                .iter()
                .flatten()
                .any(|bounds| bounds.contains(&point))
        }
    }

    fn focus(&self, index: usize, cx: &App) -> FocusHandle {
        self.focus
            .borrow_mut()
            .get_or_insert_with(|| std::array::from_fn(|_| cx.focus_handle()))[index]
            .clone()
    }

    fn close(&mut self) {
        self.open = false;
        self.duration = None;
        self.keyboard = false;
        self.hovered = [false; 3];
        self.close_task = None;
        let mut bounds = self.bounds.get();
        bounds[1..].fill(None);
        self.bounds.set(bounds);
    }

    fn idle(&self) -> bool {
        !self.keyboard && !self.hovered.iter().any(|hovered| *hovered)
    }

    fn handle(
        &mut self,
        event: Event,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<(SelfStatus, StatusDuration)> {
        self.close_task = None;
        match event {
            Event::Hover(part, hovered) => {
                self.hovered[part as usize] = hovered;
                if hovered {
                    self.keyboard = false;
                }
                if hovered && matches!(part, Part::Trigger) {
                    self.open = true;
                }
            }
            Event::Open(focus) => {
                self.open = true;
                self.duration = None;
                self.keyboard = focus;
                if focus {
                    self.focus(1, cx).focus(window, cx);
                }
            }
            Event::Duration(status, focus) => {
                self.duration = (status != SelfStatus::Online).then_some(status);
                if focus {
                    self.keyboard = true;
                }
                if focus && self.duration.is_some() {
                    self.focus(5, cx).focus(window, cx);
                }
            }
            Event::FocusChoice(index) => {
                self.duration = None;
                self.keyboard = true;
                self.focus(index + 1, cx).focus(window, cx);
            }
            Event::FocusDuration(index) => {
                self.keyboard = true;
                self.focus(index + 5, cx).focus(window, cx);
            }
            Event::Dismiss => {
                if let Some(status) = self.duration.take() {
                    self.keyboard = true;
                    let index = SelfStatus::ALL
                        .iter()
                        .position(|choice| *choice == status)
                        .unwrap_or(0);
                    self.focus(index + 1, cx).focus(window, cx);
                } else {
                    self.close();
                    self.focus(0, cx).focus(window, cx);
                }
            }
            Event::Outside => self.close(),
            Event::Activate(status, duration) => {
                self.close();
                self.focus(0, cx).focus(window, cx);
                return Some((status, duration));
            }
        }
        if self.duration.is_none() {
            let mut bounds = self.bounds.get();
            bounds[2] = None;
            self.bounds.set(bounds);
            self.hovered[2] = false;
        }
        None
    }
}

impl DiscordApp {
    fn status_menu_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        let enabled = self.can_set_self_status();
        let Some(card) = self.profile.as_mut().filter(|card| card.is_account()) else {
            return;
        };
        if !enabled
            && !matches!(
                event,
                Event::Outside | Event::Dismiss | Event::Hover(_, false)
            )
        {
            return;
        }
        let selection = card.status_menu.handle(event, window, cx);
        if card.status_menu.open
            && card.status_menu.idle()
            && matches!(event, Event::Hover(_, false))
        {
            let bounds = card.status_menu.bounds.clone();
            card.status_menu.close_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CLOSE_DELAY).await;
                this.update(cx, |app, cx| {
                    if let Some(card) = app.profile.as_mut()
                        && Rc::ptr_eq(&card.status_menu.bounds, &bounds)
                        && card.status_menu.idle()
                    {
                        card.status_menu.close();
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        if let Some((status, duration)) = selection {
            self.set_self_status(status, duration, cx);
        }
        cx.notify();
    }

    pub(crate) fn dismiss_status_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self
            .profile
            .as_ref()
            .is_some_and(|card| card.status_menu.open)
        {
            return false;
        }
        self.status_menu_event(Event::Dismiss, window, cx);
        true
    }
}

struct Props<'a> {
    current: Option<SelfStatus>,
    pending: bool,
    enabled: bool,
    error: Option<&'a str>,
    palette: ProfilePalette,
}

pub(super) fn trigger(
    app: &DiscordApp,
    card: &ProfileCard,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let entity = app.entity.clone();
    let listener: Listener = Rc::new(move |event, window, cx| {
        entity
            .update(cx, |app, cx| app.status_menu_event(event, window, cx))
            .ok();
    });
    render(
        &card.status_menu,
        Props {
            current: app.selected_self_status(),
            pending: app.self_status.pending.is_some(),
            enabled: app.can_set_self_status(),
            error: app.self_status.error.as_deref(),
            palette: card.palette(),
        },
        listener,
        window,
        cx,
    )
}

fn marker(status: SelfStatus) -> impl IntoElement {
    theme::status_icon(status.into(), 10.)
}

fn record(state: &State, index: usize) -> impl IntoElement {
    let bounds = state.bounds.clone();
    let open = state.open;
    // Explicit origins keep the observer aligned to the row/surface, not its padding.
    canvas(
        move |value, window, _| {
            let mut saved = bounds.get();
            if saved[index] != Some(value) {
                saved[index] = Some(value);
                bounds.set(saved);
                if open {
                    window.refresh();
                }
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn render(
    state: &State,
    props: Props<'_>,
    listener: Listener,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let hover = listener.clone();
    let click = listener.clone();
    let key = listener.clone();
    let current = props.current;
    let palette = props.palette;
    let row = div()
        .id("account-status-trigger")
        .debug_selector(|| "account-status-trigger".into())
        .role(Role::MenuItem)
        .aria_label("Change status")
        .aria_expanded(state.open)
        .track_focus(&state.focus(0, cx))
        .flex()
        .items_center()
        .gap(rpx(12.))
        .min_h(rpx(36.))
        .px(rpx(12.))
        .py(rpx(8.))
        .rounded(rpx(8.))
        .text_size(rpx(14.))
        .line_height(rpx(20.))
        .text_color(rgb(palette.text))
        .without_text_selection()
        .child(record(state, 0))
        .when(props.enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(palette.hover).text_color(rgb(palette.strong)))
                .focus_visible(|style| style.bg(palette.hover))
                .on_hover(move |hovered, window, cx| {
                    hover(Event::Hover(Part::Trigger, *hovered), window, cx)
                })
                .on_click(move |_, window, cx| click(Event::Open(true), window, cx))
                .on_key_down(move |event, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "right" | "down") {
                        key(Event::Open(true), window, cx);
                        cx.stop_propagation();
                    }
                })
        })
        .when(!props.enabled, |this| {
            this.opacity(0.5)
                .aria_description("Status changes unavailable")
        })
        .child(
            div()
                .w(rpx(18.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .children(current.map(marker)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(current.map_or("Status", SelfStatus::label)),
        )
        .when(props.pending, |this| {
            this.child(div().text_size(rpx(12.)).child("Updating..."))
        })
        .child(
            theme::icon("chevron-down", 16., palette.muted).with_transformation(
                Transformation::rotate(radians(-std::f32::consts::FRAC_PI_2)),
            ),
        );
    let mut root = div()
        .relative()
        .child(row)
        .children(props.error.map(|error| {
            div()
                .px(rpx(12.))
                .pb(rpx(8.))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(palette.text))
                .child(error.to_owned())
        }));
    let bounds = state.bounds.get();
    let first_row_offset = |menu: usize, row: usize, fallback: f32| {
        bounds[menu]
            .zip(bounds[row])
            .map_or(rpx(fallback).to_pixels(window.rem_size()), |(menu, row)| {
                row.center().y - menu.top()
            })
    };
    if state.open
        && let Some(anchor) = bounds[0]
    {
        let width = menu_width(CHOICES_WIDTH, window);
        let height = bounds[1].map_or(rpx(205.).to_pixels(window.rem_size()), |bounds| {
            bounds.size.height
        });
        let position = flyout_position(
            anchor,
            size(width, height),
            window.viewport_size(),
            rpx(4.).to_pixels(window.rem_size()),
            rpx(8.).to_pixels(window.rem_size()),
            first_row_offset(1, 3, 26.),
        );
        let menu = choices(state, &props, listener.clone(), width, window, cx);
        root = root.child(flyout(menu, position, window, 3));
    }
    if state.open
        && let Some(status) = state.duration
    {
        let index = SelfStatus::ALL
            .iter()
            .position(|choice| *choice == status)
            .unwrap_or(0);
        if let (Some(menu), Some(row)) = (bounds[1], bounds[index + 3]) {
            let anchor = Bounds::new(
                point(menu.left(), row.top()),
                size(menu.size.width, row.size.height),
            );
            let width = menu_width(DURATION_WIDTH, window);
            let height = bounds[2].map_or(rpx(220.).to_pixels(window.rem_size()), |bounds| {
                bounds.size.height
            });
            let position = flyout_position(
                anchor,
                size(width, height),
                window.viewport_size(),
                rpx(4.).to_pixels(window.rem_size()),
                rpx(8.).to_pixels(window.rem_size()),
                first_row_offset(2, 7, 25.),
            );
            root = root.child(flyout(
                durations(state, &props, listener, status, width, window, cx),
                position,
                window,
                4,
            ));
        }
    }
    root.into_any_element()
}

fn menu_width(css: f32, window: &Window) -> Pixels {
    rpx(css)
        .to_pixels(window.rem_size())
        .min((window.viewport_size().width - rpx(16.).to_pixels(window.rem_size())).max(px(0.)))
}

fn flyout(menu: AnyElement, position: Point<Pixels>, window: &Window, priority: usize) -> Div {
    div().absolute().size_0().child(
        deferred(
            anchored()
                .anchor(Anchor::TopLeft)
                .position(position)
                .snap_to_window_with_margin(Edges::all(rpx(8.).to_pixels(window.rem_size())))
                .child(menu),
        )
        .with_priority(priority),
    )
}

fn flyout_position(
    anchor: Bounds<Pixels>,
    menu: Size<Pixels>,
    viewport: Size<Pixels>,
    gap: Pixels,
    margin: Pixels,
    first_row_offset: Pixels,
) -> Point<Pixels> {
    let right = anchor.right() + gap;
    let left = anchor.left() - gap - menu.width;
    let x = if right + menu.width <= viewport.width - margin {
        right
    } else {
        left
    };
    point(
        x.max(margin)
            .min((viewport.width - menu.width - margin).max(margin)),
        (anchor.center().y - first_row_offset)
            .max(margin)
            .min((viewport.height - menu.height - margin).max(margin)),
    )
}

fn surface(
    state: &State,
    part: Part,
    listener: Listener,
    width: Pixels,
    window: &Window,
) -> Stateful<Div> {
    let hover = listener.clone();
    let outside = listener;
    let bounds = state.bounds.clone();
    let id = if matches!(part, Part::Choices) {
        "account-status-choices"
    } else {
        "account-status-durations"
    };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Menu)
        .aria_label(if matches!(part, Part::Choices) {
            "Status"
        } else {
            "Status duration"
        })
        .flex()
        .flex_col()
        .w(width)
        .p(rpx(8.))
        .rounded(rpx(8.))
        .max_h((window.viewport_size().height - rpx(16.).to_pixels(window.rem_size())).max(px(0.)))
        .overflow_y_scroll()
        .bg(rgb(theme::elevated()))
        .text_color(rgb(theme::text()))
        .shadow_lg()
        .occlude()
        .child(record(state, part as usize))
        .on_hover(move |hovered, window, cx| hover(Event::Hover(part, *hovered), window, cx))
        .on_mouse_down_out(move |event, window, cx| {
            if !bounds.get()[..3]
                .iter()
                .flatten()
                .any(|bounds| bounds.contains(&event.position))
            {
                outside(Event::Outside, window, cx);
            }
        })
        .on_click(|_, _, cx| cx.stop_propagation())
}

fn choice_row(id: &'static str, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .min_h(rpx(36.))
        .px(rpx(12.))
        .py(rpx(8.))
        .rounded(rpx(4.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::text()))
        .without_text_selection()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(rgba(theme::hover())))
                .focus_visible(|style| style.bg(rgba(theme::hover())))
        })
        .when(!enabled, |this| {
            this.opacity(0.5)
                .aria_description("Status changes unavailable")
        })
}

fn choices(
    state: &State,
    props: &Props<'_>,
    listener: Listener,
    width: Pixels,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let mut menu = surface(state, Part::Choices, listener.clone(), width, window);
    for (index, status) in SelfStatus::ALL.into_iter().enumerate() {
        if index == 1 {
            menu = menu.child(
                div()
                    .flex_none()
                    .h(rpx(1.))
                    .my(rpx(8.))
                    .bg(rgba(theme::border())),
            );
        }
        let click = listener.clone();
        let hover = listener.clone();
        let key = listener.clone();
        let subtitle = match status {
            SelfStatus::DoNotDisturb => Some("You will not recieve desktop notifications"),
            SelfStatus::Invisible => Some("You will appear offline"),
            _ => None,
        };
        let row = choice_row(status.wire(), props.enabled)
            .role(Role::MenuItemRadio)
            .aria_label(status.label())
            .aria_toggled(if props.current == Some(status) {
                Toggled::True
            } else {
                Toggled::False
            })
            .track_focus(&state.focus(index + 1, cx))
            .child(record(state, index + 3))
            .when(props.enabled, |this| {
                this.on_hover(move |hovered, window, cx| {
                    if *hovered {
                        hover(Event::Duration(status, false), window, cx);
                    }
                })
                .on_click(move |_, window, cx| {
                    click(
                        if status == SelfStatus::Online {
                            Event::Activate(status, StatusDuration::Forever)
                        } else {
                            Event::Duration(status, true)
                        },
                        window,
                        cx,
                    );
                })
                .on_key_down(move |event, window, cx| {
                    let action = match event.keystroke.key.as_str() {
                        "up" => Event::FocusChoice((index + 3) % 4),
                        "down" => Event::FocusChoice((index + 1) % 4),
                        "right" if status != SelfStatus::Online => Event::Duration(status, true),
                        "left" | "escape" => Event::Dismiss,
                        _ => return,
                    };
                    key(action, window, cx);
                    cx.stop_propagation();
                })
            })
            .child(marker(status))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(status.label())
                    .children(subtitle.map(|text| {
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_size(rpx(12.))
                            .line_height(rpx(16.))
                            .text_color(rgb(theme::muted()))
                            .child(text)
                    })),
            )
            .when(status != SelfStatus::Online, |this| {
                this.child(
                    theme::icon("chevron-down", 16., theme::muted()).with_transformation(
                        Transformation::rotate(radians(-std::f32::consts::FRAC_PI_2)),
                    ),
                )
            });
        menu = menu.child(row);
    }
    menu.into_any_element()
}

fn durations(
    state: &State,
    props: &Props<'_>,
    listener: Listener,
    status: SelfStatus,
    width: Pixels,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let mut menu = surface(state, Part::Durations, listener.clone(), width, window);
    for (index, duration) in StatusDuration::ALL.into_iter().enumerate() {
        let click = listener.clone();
        let key = listener.clone();
        menu = menu.child(
            choice_row(duration.label(), props.enabled)
                .role(Role::MenuItem)
                .aria_label(duration.label())
                .track_focus(&state.focus(index + 5, cx))
                .when(index == 0, |this| this.child(record(state, 7)))
                .min_h(rpx(34.))
                .when(props.enabled, |this| {
                    this.on_click(move |_, window, cx| {
                        click(Event::Activate(status, duration), window, cx)
                    })
                    .on_key_down(move |event, window, cx| {
                        let action = match event.keystroke.key.as_str() {
                            "up" => Event::FocusDuration((index + 5) % 6),
                            "down" => Event::FocusDuration((index + 1) % 6),
                            "left" | "escape" => Event::Dismiss,
                            _ => return,
                        };
                        key(action, window, cx);
                        cx.stop_propagation();
                    })
                })
                .child(duration.label()),
        );
    }
    menu.into_any_element()
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{
        Bounds, Context, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, Pixels,
        Point, Render, TestAppContext, VisualTestContext, Window, div, point, prelude::*, px, size,
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
}
