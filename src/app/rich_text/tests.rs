use std::rc::Rc;

use gpui::{App, Bounds, TestAppContext, Window, point, px, size};
use gpui_component::ActiveTheme;

use super::{Content, Fonts, LayoutCache, Line, Piece, PieceStyle, metrics};

fn fonts(window: &Window, cx: &App) -> Fonts {
    let style = window.text_style();
    Fonts {
        base: style.font(),
        mono: cx.theme().mono_font_family.clone(),
        color: style.color,
        font_size: px(14.),
        line_height: px(20.),
        emoji_size: px(20.),
    }
}

#[gpui::test]
fn repeated_sizing_probes_share_shaped_words_and_layouts(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let fonts = fonts(window, cx);
        let pieces = [Piece::text(
            "short words in a long post ".repeat(100),
            PieceStyle::default(),
        )];
        let mut cache = LayoutCache::new(true, false);
        metrics::SHAPES.set(0);
        metrics::LAYOUTS.set(0);
        let first = cache.get(&pieces, &fonts, Some(px(120.)), window);
        let shapes = metrics::SHAPES.get();
        for _ in 0..10 {
            cache.get(&pieces, &fonts, Some(px(0.)), window);
            cache.get(&pieces, &fonts, None, window);
            let repeated = cache.get(&pieces, &fonts, Some(px(120.)), window);
            assert!(Rc::ptr_eq(&first, &repeated));
        }
        assert_eq!(metrics::LAYOUTS.get(), 3);
        assert_eq!(metrics::SHAPES.get(), shapes);
        assert_eq!(shapes, 600);
        let narrower = cache.get(&pieces, &fonts, Some(px(80.)), window);
        assert!(narrower.size.height > first.size.height);
    });
}

#[gpui::test]
fn positive_widths_still_wrap_unbroken_unicode_without_losing_text(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let fonts = fonts(window, cx);
        let text = "longword\u{e9}".repeat(20);
        let pieces = [Piece::text(text.clone(), PieceStyle::default())];
        let mut cache = LayoutCache::new(true, false);
        let intrinsic = cache.get(&pieces, &fonts, Some(px(0.)), window);
        assert_eq!(intrinsic.lines.len(), 1);
        let wrapped = cache.get(&pieces, &fonts, Some(px(80.)), window);
        assert!(wrapped.lines.len() > 1);
        let reconstructed: String = wrapped
            .lines
            .iter()
            .flat_map(|line| &line.fragments)
            .map(|fragment| {
                let Content::Text(value) = &pieces[fragment.piece].content else {
                    unreachable!()
                };
                assert!(fragment.width <= px(80.));
                &value[fragment.range.clone()]
            })
            .collect();
        assert_eq!(reconstructed, text);
    });
}

#[gpui::test]
fn smaller_edited_labels_share_the_message_baseline(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        for zoom in [1., 1.25, 1.5] {
            let mut fonts = fonts(window, cx);
            fonts.font_size = px(16. * zoom);
            fonts.line_height = px(22. * zoom);
            let normal = fonts.shape("message", &PieceStyle::default(), false, window);
            let edited = fonts.shape(
                " (edited)",
                &PieceStyle {
                    scale: Some(0.625),
                    ..Default::default()
                },
                false,
                window,
            );
            let baseline = fonts.baseline(window);
            // Also cover lines enlarged by emoji and labels wrapped onto their own line.
            for height in [fonts.line_height, fonts.line_height * 2.] {
                let line = Line {
                    y: px(22. * zoom),
                    height,
                    fragments: Vec::new(),
                };
                let text_height = fonts.line_height;
                let painted_baseline = |shaped: &gpui::ShapedLine| {
                    line.text_top(shaped, text_height, baseline)
                        + (text_height - shaped.ascent - shaped.descent) / 2.
                        + shaped.ascent
                };
                assert!((painted_baseline(&normal) - painted_baseline(&edited)).abs() < px(0.01));
                assert!(
                    line.text_top(&edited, text_height, baseline)
                        > line.y + (height - text_height) / 2.
                );
            }
        }
    });
}

#[test]
fn visible_lines_follow_the_scrolled_origin_and_include_glyph_overhang() {
    let mask = Bounds::new(point(px(0.), px(0.)), size(px(200.), px(100.)));
    let line = Line {
        y: px(100.),
        height: px(20.),
        fragments: Vec::new(),
    };
    assert!(line.visible(point(px(0.), px(0.)), mask, px(14.)));
    assert!(!line.visible(point(px(0.), px(20.)), mask, px(14.)));
    assert!(line.visible(point(px(0.), px(-110.)), mask, px(14.)));
    assert!(!line.visible(point(px(0.), px(-140.)), mask, px(14.)));
}
