use std::{rc::Rc, sync::Arc};

use gpui::{
    App, Bounds, Context, Hsla, ImageSource, Pixels, Render, SharedString, TestAppContext, Window,
    black, div, point, prelude::*, px, size, white,
};
use gpui_component::ActiveTheme;

use super::{
    Content, EmojiSource, Fonts, LayoutCache, Line, Piece, PieceStyle, layout_fingerprint, metrics,
};

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

struct TextCase {
    text: SharedString,
    color: Hsla,
    width: Pixels,
}

impl Render for TextCase {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(self.width).child(super::rich_text(
            "cached-text",
            vec![Piece::text(
                self.text.clone(),
                PieceStyle {
                    color: Some(self.color),
                    ..Default::default()
                },
            )],
        ))
    }
}

fn reset_metrics() {
    metrics::SHAPES.set(0);
    metrics::LAYOUTS.set(0);
    metrics::PAINTS.set(0);
    metrics::EMOJIS.set(0);
}

fn draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        let _ = window.draw(cx);
    });
}

#[gpui::test]
fn unchanged_text_shapes_nothing_on_a_later_frame(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| TextCase {
        text: "one two three four five six".into(),
        color: black(),
        width: px(240.),
    });
    assert!(metrics::SHAPES.get() > 0);

    view.update(cx, |_, cx| cx.notify());
    reset_metrics();
    draw(cx);
    assert!(metrics::PAINTS.get() > 0);
    assert_eq!(metrics::SHAPES.get(), 0);
}

#[gpui::test]
fn edited_text_reshapes(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| TextCase {
        text: "one two three four five six".into(),
        color: black(),
        width: px(240.),
    });
    draw(cx);

    reset_metrics();
    view.update(cx, |view, cx| {
        view.text = "different words entirely".into();
        cx.notify();
    });
    draw(cx);
    assert!(metrics::SHAPES.get() > 0);
}

#[gpui::test]
fn width_changes_rewrap_without_reshaping(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| TextCase {
        text: "one two three four five six seven eight nine ten".into(),
        color: black(),
        width: px(240.),
    });
    draw(cx);

    reset_metrics();
    view.update(cx, |view, cx| {
        view.width = px(120.);
        cx.notify();
    });
    draw(cx);
    assert!(metrics::PAINTS.get() > 0);
    assert_eq!(metrics::SHAPES.get(), 0);
    assert!(metrics::LAYOUTS.get() > 0);
}

#[gpui::test]
fn colour_changes_reshape_because_runs_carry_decoration(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| TextCase {
        text: "one two three four five six".into(),
        color: black(),
        width: px(240.),
    });
    draw(cx);

    reset_metrics();
    view.update(cx, |view, cx| {
        view.color = white();
        cx.notify();
    });
    draw(cx);
    assert!(metrics::SHAPES.get() > 0);
}

#[gpui::test]
fn emoji_image_loading_does_not_invalidate_layout(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let fonts = fonts(window, cx);
        let piece = |image: Option<ImageSource>| Piece {
            content: Content::Emoji(image.map(EmojiSource::Still), "\u{1f600}".into()),
            style: PieceStyle::default(),
        };
        let loading = layout_fingerprint(&[piece(None)], &fonts, true, false);
        let loaded = layout_fingerprint(&[piece(Some("emoji.png".into()))], &fonts, true, false);
        assert_eq!(loading, loaded);
    });
}

struct EmojiCase {
    image: Option<ImageSource>,
}

impl Render for EmojiCase {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(200.)).child(super::rich_text(
            "cached-emoji",
            vec![Piece {
                content: Content::Emoji(
                    self.image.clone().map(EmojiSource::Still),
                    "\u{1f600}".into(),
                ),
                style: PieceStyle::default(),
            }],
        ))
    }
}

#[gpui::test]
fn loaded_emoji_paints_with_a_cached_layout(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|_, _| EmojiCase { image: None });
    draw(cx);

    reset_metrics();
    view.update(cx, |view, cx| {
        view.image = Some(ImageSource::Custom(Arc::new(
            |_: &mut Window, _: &mut App| None,
        )));
        cx.notify();
    });
    draw(cx);
    assert!(metrics::EMOJIS.get() > 0);
    assert_eq!(metrics::SHAPES.get(), 0);
}
