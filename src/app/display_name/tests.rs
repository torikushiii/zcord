use gpui::TestAppContext;
use gpui_component::ElementExt;

use super::*;

fn metrics(window: &Window, wrap: bool) -> Metrics {
    Metrics {
        font: window.text_style().font(),
        size: px(16.),
        line_height: px(22.),
        tracking: px(0.48),
        wrap,
    }
}

#[test]
fn only_profiles_wrap_and_only_hovered_accounts_animate() {
    assert!(!Surface::FontOnly.wraps());
    assert!(!Surface::FontOnly.animates());
    assert!(!Surface::Account { hovered: false }.wraps());
    assert!(!Surface::Account { hovered: false }.animates());
    assert!(Surface::Account { hovered: true }.animates());
    assert!(Surface::Profile.wraps());
    assert!(Surface::Profile.animates());
}

#[gpui::test]
fn truncation_keeps_graphemes_and_accounts_for_tracking(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, _| {
        let metrics = metrics(window, false);
        let text: SharedString = "a\u{301}bc\u{65e5}\u{672c}defghijklmnopqrstuvwxyz".into();
        let width = px(90.);
        let clipped = truncated(&text, &metrics, width, window);
        assert!(clipped.ends_with('\u{2026}'));
        let prefix = clipped.trim_end_matches('\u{2026}');
        assert!(text.starts_with(prefix));
        assert!(
            text.grapheme_indices(true)
                .any(|(index, _)| index == prefix.len())
        );
        assert!(text_width(&clipped, &metrics, window) <= width);
        assert_eq!(truncated(&text, &metrics, px(0.), window).len(), 0);
        assert_eq!(truncated(&text, &metrics, px(2000.), window), text);
    });
}

#[gpui::test]
fn wrapped_geometry_resets_line_width_and_keeps_all_glyphs(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, _| {
        let metrics = metrics(window, true);
        let text: SharedString = "longer name short ending".into();
        let full = layout(&text, &metrics, None, window);
        let wrapped = layout(&text, &metrics, Some(px(90.)), window);
        assert!(wrapped.lines.len() > 1);
        assert_eq!(
            wrapped.size.height,
            metrics.line_height * wrapped.lines.len()
        );
        assert_eq!(
            full.lines
                .iter()
                .map(|line| line.glyphs.len())
                .sum::<usize>(),
            wrapped
                .lines
                .iter()
                .map(|line| line.glyphs.len())
                .sum::<usize>()
        );
        for line in &wrapped.lines {
            assert!(
                line.width <= px(90.),
                "wrapped line exceeds its measured width"
            );
        }
        assert!(wrapped.lines.last().unwrap().width < full.lines[0].width);
    });
}

#[gpui::test]
fn layout_cache_is_shared_and_bounded(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, _| {
        let metrics = metrics(window, false);
        let text: SharedString = "sample.user".into();
        let mut cache = LayoutCache::default();
        let first = cache.get(&text, &metrics, Some(px(120.)), window);
        assert!(Rc::ptr_eq(
            &first,
            &cache.get(&text, &metrics, Some(px(120.)), window)
        ));
        for width in [60., 80., 90., 100., 200.] {
            cache.get(&text, &metrics, Some(px(width)), window);
        }
        assert_eq!(cache.0.len(), 4);
    });
}

struct NameSamples {
    width: Pixels,
    bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
}

impl Render for NameSamples {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(self.width)
            .flex()
            .flex_col()
            .text_size(px(20.))
            .line_height(px(26.))
            .children((0..=8).map(|id| {
                let recorded = self.bounds.clone();
                div()
                    .on_prepaint(move |bounds, _, _| recorded.borrow_mut().push(bounds))
                    .child(DisplayName {
                        id: ("sample-name", id as usize).into(),
                        text: "sample.user".into(),
                        font: Some(DisplayFont {
                            family: ".SystemUIFont",
                            letter_spacing_em: 0.03,
                        }),
                        colors: if id == 0 {
                            Vec::new()
                        } else {
                            vec![rgb(0xffb57a), rgb(0xff936f), rgb(0xff7c78), rgb(0xf06b8a)]
                        },
                        effect: if id == 0 {
                            None
                        } else {
                            Some(Effect::from_id(Some(id)))
                        },
                        surface: if id == 0 {
                            Surface::FontOnly
                        } else {
                            Surface::Profile
                        },
                    })
            }))
    }
}

#[gpui::test]
fn all_effects_paint_across_redraws_and_narrow_layouts(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let bounds = Rc::new(RefCell::new(Vec::new()));
    let (view, cx) = cx.add_window_view(|_, _| NameSamples {
        width: px(240.),
        bounds: bounds.clone(),
    });
    for width in [240., 80., 160.] {
        view.update(cx, |view, cx| {
            view.width = px(width);
            cx.notify();
        });
        bounds.borrow_mut().clear();
        for _ in 0..2 {
            cx.update(|window, cx| {
                window.refresh();
                let _ = window.draw(cx);
            });
        }
        let recorded = bounds.borrow();
        assert!(
            recorded.len() >= 9,
            "each effect must be laid out and painted"
        );
        assert!(recorded.iter().all(|bounds| bounds.size.width <= px(width)));
        assert!(recorded.iter().all(|bounds| bounds.size.height > px(0.)));
    }
}
