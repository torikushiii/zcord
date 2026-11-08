use super::{composer_style, overlays_style};
use gpui_component::input::{Textarea, TextareaState};
use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Anchor, AppContext, Context, Entity, Modifiers, Pixels, Render, StyleRefinement, Subscription,
    TestAppContext, WeakEntity, Window, anchored, canvas, deferred, div, point, prelude::*, px,
};

struct Panel {
    _host: WeakEntity<Host>,
    renders: Rc<Cell<usize>>,
    _observe: Subscription,
}

impl Panel {
    fn new(host: Entity<Host>, renders: Rc<Cell<usize>>, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&host, |_, _, cx| cx.notify()),
            _host: host.downgrade(),
            renders,
        }
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().size_full()
    }
}

struct Host {
    left: Entity<Panel>,
    right: Entity<Panel>,
}

impl Host {
    fn new(
        left_renders: Rc<Cell<usize>>,
        right_renders: Rc<Cell<usize>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let host = cx.entity();
        let left = cx.new(|cx| Panel::new(host.clone(), left_renders, cx));
        let right = cx.new(|cx| Panel::new(host, right_renders, cx));
        Self { left, right }
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .child(
                self.left
                    .clone()
                    .cached(StyleRefinement::default().flex_1().h_full()),
            )
            .child(
                self.right
                    .clone()
                    .cached(StyleRefinement::default().flex_1().h_full()),
            )
    }
}

struct Harness {
    host: Entity<Host>,
}

impl Harness {
    fn new(
        left_renders: Rc<Cell<usize>>,
        right_renders: Rc<Cell<usize>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            host: cx.new(|cx| Host::new(left_renders, right_renders, cx)),
        }
    }
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.host.clone().into_any_element()
    }
}

#[gpui::test]
fn cached_siblings_isolate_renders_until_the_observed_model_notifies(cx: &mut TestAppContext) {
    let left_renders = Rc::new(Cell::new(0));
    let right_renders = Rc::new(Cell::new(0));
    let (harness, cx) =
        cx.add_window_view(|_, cx| Harness::new(left_renders.clone(), right_renders.clone(), cx));
    let host = harness.read_with(cx, |harness, _| harness.host.clone());
    let left = host.read_with(cx, |host, _| host.left.clone());

    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let left_initial = left_renders.get();
    let right_initial = right_renders.get();
    assert!(left_initial > 0 && right_initial > 0);

    left.update(cx, |_, cx| cx.notify());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(left_renders.get(), left_initial + 1);
    assert_eq!(right_renders.get(), right_initial);

    host.update(cx, |_, cx| cx.notify());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(left_renders.get(), left_initial + 2);
    assert_eq!(right_renders.get(), right_initial + 1);
}

struct MeasuredContent {
    measured: Rc<Cell<Option<Pixels>>>,
    height: Rc<Cell<Pixels>>,
}

impl Render for MeasuredContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let measured = self.measured.clone();
        div()
            .w_full()
            .child(div().w_full().h(self.height.get()))
            .child(
                canvas(
                    move |bounds, _, _| measured.set(Some(bounds.size.height)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}

struct MeasuredHost {
    content: Entity<MeasuredContent>,
    measured: Rc<Cell<Option<Pixels>>>,
}

impl Render for MeasuredHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let height = self.measured.get().unwrap_or(px(64.));
        div().size_full().child(
            self.content
                .clone()
                .cached(StyleRefinement::default().w_full().h(height)),
        )
    }
}

#[gpui::test]
fn a_cached_view_measures_its_intrinsic_height_for_the_next_frame(cx: &mut TestAppContext) {
    let measured = Rc::new(Cell::new(None));
    let content = measured.clone();
    let host = measured.clone();
    let height = Rc::new(Cell::new(px(100.)));
    let content_height = height.clone();
    let (host, cx) = cx.add_window_view(move |_, cx| MeasuredHost {
        content: cx.new(|_| MeasuredContent {
            measured: content,
            height: content_height,
        }),
        measured: host,
    });
    let content = host.read_with(cx, |host, _| host.content.clone());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(measured.get(), Some(px(100.)));
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(measured.get(), Some(px(100.)));

    content.update(cx, |_, cx| {
        height.set(px(40.));
        cx.notify();
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(measured.get(), Some(px(40.)));
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(measured.get(), Some(px(40.)));
}

struct DeferredOverlay {
    renders: Rc<Cell<usize>>,
    clicks: Rc<Cell<usize>>,
}

impl Render for DeferredOverlay {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let clicks = self.clicks.clone();
        div().absolute().inset_0().child(deferred(
            anchored()
                .position(point(px(100.), px(80.)))
                .anchor(Anchor::TopLeft)
                .child(
                    div()
                        .id("cached-popup")
                        .occlude()
                        .w(px(120.))
                        .h(px(60.))
                        .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
                ),
        ))
    }
}

struct DeferredHost {
    overlay: Entity<DeferredOverlay>,
}

impl Render for DeferredHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(self.overlay.clone().cached(overlays_style()))
    }
}

#[gpui::test]
fn cached_views_replay_deferred_popups(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let clicks = Rc::new(Cell::new(0));
    let (made_renders, made_clicks) = (renders.clone(), clicks.clone());
    let (_host, cx) = cx.add_window_view(move |_, cx| DeferredHost {
        overlay: cx.new(|_| DeferredOverlay {
            renders: made_renders,
            clicks: made_clicks,
        }),
    });
    let draw = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    };
    draw(cx);
    let initial = renders.get();
    assert!(initial > 0);

    draw(cx);
    assert_eq!(renders.get(), initial);
    cx.simulate_click(point(px(160.), px(110.)), Modifiers::none());
    assert_eq!(clicks.get(), 1);
}
struct OverflowPanel {
    clicks: Rc<Cell<usize>>,
}

impl Render for OverflowPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        div().relative().w_full().h_full().child(
            div()
                .id("overflow-popup")
                .absolute()
                .left_0()
                .bottom_0()
                .w(px(100.))
                .h(px(80.))
                .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
        )
    }
}

struct OverflowHost {
    panel: Entity<OverflowPanel>,
}

impl Render for OverflowHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().pt(px(100.)).child(
            self.panel
                .clone()
                .cached(composer_style(Some(px(50.)), false)),
        )
    }
}

#[gpui::test]
fn a_cached_composer_does_not_clip_popups_that_overflow_upward(cx: &mut TestAppContext) {
    let clicks = Rc::new(Cell::new(0));
    let made_clicks = clicks.clone();
    let (_host, cx) = cx.add_window_view(move |_, cx| OverflowHost {
        panel: cx.new(|_| OverflowPanel {
            clicks: made_clicks,
        }),
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_click(point(px(50.), px(110.)), Modifiers::none());
    assert_eq!(clicks.get(), 1);
}
struct TypedPanel {
    input: Entity<TextareaState>,
    renders: Rc<Cell<usize>>,
}

impl Render for TypedPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().w_full().h(px(50.)).child(Textarea::new(&self.input))
    }
}

struct TypedHost {
    panel: Entity<TypedPanel>,
}

impl Render for TypedHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            self.panel
                .clone()
                .cached(composer_style(Some(px(64.)), false)),
        )
    }
}

#[gpui::test]
fn a_cached_composer_rerenders_when_the_textarea_changes(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let renders = Rc::new(Cell::new(0));
    let counted = renders.clone();
    let (host, cx) = cx.add_window_view(move |window, cx| {
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 15));
        input.update(cx, |input, cx| input.focus(window, cx));
        TypedHost {
            panel: cx.new(|_| TypedPanel {
                input,
                renders: counted,
            }),
        }
    });
    let _panel = host.read_with(cx, |host, _| host.panel.clone());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let initial = renders.get();
    cx.simulate_input("hello");
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(
        renders.get() > initial,
        "typing in a cached composer must re-render its textarea"
    );
}
