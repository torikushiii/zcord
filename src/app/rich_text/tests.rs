use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use gpui::{
    App, Bounds, Context, Hsla, ImageSource, Pixels, Render, SharedString, TestAppContext, Window,
    black, div, point, prelude::*, px, rems, size, white,
};
use gpui_component::ActiveTheme;

use super::{
    Content, EmojiSource, Fonts, LayoutCache, Line, Piece, PieceStyle, emoji_inset,
    layout_fingerprint, metrics,
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
        emoji_gap: None,
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
        let mut cache = LayoutCache::new(true, false, None);
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
        let mut cache = LayoutCache::new(true, false, None);
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

fn ellipsis_count(layout: &super::Layout) -> usize {
    layout
        .lines
        .iter()
        .flat_map(|line| &line.fragments)
        .filter(|fragment| fragment.shaped.is_some() && fragment.range.is_empty())
        .count()
}

fn fragment_for(layout: &super::Layout, piece: usize) -> &super::Fragment {
    layout
        .lines
        .iter()
        .flat_map(|line| &line.fragments)
        .find(|fragment| fragment.piece == piece)
        .expect("fragment for piece")
}

#[test]
fn new_builders_are_opt_in() {
    let default = super::rich_text("default", vec![]);
    assert!(default.line_clamp.is_none());
    assert!(default.emoji_gap.is_none());

    let configured = super::rich_text("configured", vec![])
        .line_clamp(2)
        .emoji_gap(rems(0.25));
    assert_eq!(configured.line_clamp, Some(2));
    assert_eq!(configured.emoji_gap, Some(rems(0.25)));
}

#[gpui::test]
fn line_clamp_keeps_two_lines_and_ellipsizes_only_hidden_content(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let fonts = fonts(window, cx);
        let pieces = [Piece::text(
            "one two three four five six seven eight nine ten ".repeat(4),
            PieceStyle::default(),
        )];
        let mut cache = LayoutCache::new(true, false, Some(2));
        let layout = cache.get(&pieces, &fonts, Some(px(80.)), window);
        assert_eq!(layout.lines.len(), 2);
        assert_eq!(layout.size.height, fonts.line_height * 2.);
        assert!(layout.size.width <= px(80.));
        assert_eq!(ellipsis_count(&layout), 1);
        let last = layout.lines.last().unwrap().fragments.last().unwrap();
        assert!(last.shaped.is_some() && last.range.is_empty());
        assert!(last.x + last.width <= px(80.));
    });
}

#[gpui::test]
fn line_clamp_does_not_ellipsize_when_content_fits_exactly(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let fonts = fonts(window, cx);
        let pieces = [Piece::text("one\ntwo", PieceStyle::default())];
        let mut cache = LayoutCache::new(true, false, Some(2));
        let layout = cache.get(&pieces, &fonts, Some(px(200.)), window);
        assert_eq!(layout.lines.len(), 2);
        assert_eq!(ellipsis_count(&layout), 0);
    });
}

#[gpui::test]
fn line_clamp_respects_cjk_boundaries_and_empty_trailing_lines(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let fonts = fonts(window, cx);
        let text = "日本語".repeat(40);
        let pieces = [Piece::text(text.clone(), PieceStyle::default())];
        let mut cache = LayoutCache::new(true, false, Some(2));
        let layout = cache.get(&pieces, &fonts, Some(px(60.)), window);
        assert_eq!(layout.lines.len(), 2);
        assert_eq!(ellipsis_count(&layout), 1);
        for fragment in layout.lines.iter().flat_map(|line| &line.fragments) {
            if fragment.range.is_empty() {
                continue;
            }
            assert!(text.is_char_boundary(fragment.range.start));
            assert!(text.is_char_boundary(fragment.range.end));
        }

        // A trailing newline produces an empty extra line, but hiding it is not content.
        let pieces = [Piece::text("one\n", PieceStyle::default())];
        let mut cache = LayoutCache::new(true, false, Some(1));
        let layout = cache.get(&pieces, &fonts, Some(px(200.)), window);
        assert_eq!(layout.lines.len(), 1);
        assert_eq!(ellipsis_count(&layout), 0);

        // Zero is a valid zero-height clamp and must not underflow the last-line logic.
        let mut cache = LayoutCache::new(true, false, Some(0));
        let layout = cache.get(&pieces, &fonts, Some(px(200.)), window);
        assert!(layout.lines.is_empty());
        assert_eq!(layout.size.height, px(0.));
    });
}

#[gpui::test]
fn line_clamp_hides_emoji_on_later_lines(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let fonts = fonts(window, cx);
        let pieces = [
            Piece::text("visible\nhidden ", PieceStyle::default()),
            Piece {
                content: Content::Emoji(None, "\u{1f600}".into()),
                style: PieceStyle::default(),
            },
        ];
        let mut cache = LayoutCache::new(true, false, Some(1));
        let layout = cache.get(&pieces, &fonts, Some(px(200.)), window);
        assert_eq!(layout.lines.len(), 1);
        assert!(
            !layout
                .lines
                .iter()
                .flat_map(|line| &line.fragments)
                .any(|fragment| fragment.piece == 1)
        );
        assert_eq!(ellipsis_count(&layout), 1);
    });
}

#[gpui::test]
fn explicit_emoji_gap_is_trailing_and_default_slot_is_unchanged(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let fonts = fonts(window, cx);
        let pieces = [
            Piece::text("a", PieceStyle::default()),
            Piece {
                content: Content::Emoji(None, "\u{1f600}".into()),
                style: PieceStyle::default(),
            },
            Piece::text("b", PieceStyle::default()),
        ];
        let mut cache = LayoutCache::new(false, false, None);
        let layout = cache.get(&pieces, &fonts, None, window);
        let emoji = fragment_for(&layout, 1);
        assert_eq!(
            emoji.width,
            fonts.emoji_size + fonts.font_size * 0.1,
            "legacy slot width"
        );
        let legacy_inset = emoji_inset(emoji.width, fonts.emoji_size, false);
        assert_eq!(legacy_inset, (emoji.width - fonts.emoji_size) / 2.);
        assert!(legacy_inset > px(0.));
        assert_eq!(emoji_inset(emoji.width, fonts.emoji_size, true), px(0.));

        let mut gapped = fonts.clone();
        gapped.emoji_gap = Some(px(4.));
        let mut cache = LayoutCache::new(false, false, None);
        let layout = cache.get(&pieces, &gapped, None, window);
        let emoji = fragment_for(&layout, 1);
        assert_eq!(emoji.width, gapped.emoji_size + px(4.));
        assert_eq!(emoji_inset(emoji.width, gapped.emoji_size, true), px(0.));
    });
}

#[gpui::test]
fn line_clamp_and_emoji_gap_invalidate_the_layout_fingerprint(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let fonts = fonts(window, cx);
        let pieces = [Piece::text("one two", PieceStyle::default())];
        let base = layout_fingerprint(&pieces, &fonts, true, false, None);
        assert_ne!(
            base,
            layout_fingerprint(&pieces, &fonts, true, false, Some(2))
        );
        let mut gapped = fonts.clone();
        gapped.emoji_gap = Some(px(4.));
        assert_ne!(
            base,
            layout_fingerprint(&pieces, &gapped, true, false, None)
        );
    });
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
        let loading = layout_fingerprint(&[piece(None)], &fonts, true, false, None);
        let loaded = layout_fingerprint(
            &[piece(Some("emoji.png".into()))],
            &fonts,
            true,
            false,
            None,
        );
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

struct InspectableEmoji {
    image: Arc<gpui::RenderImage>,
    clicks: Rc<Cell<usize>>,
    hovers: Rc<RefCell<Vec<super::ActionHover>>>,
}

impl Render for InspectableEmoji {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        let hovers = self.hovers.clone();
        div()
            .size_full()
            .child(gpui_base::TextSelectionLayer)
            .child(
                super::rich_text(
                    "inspectable-emoji",
                    vec![Piece {
                        content: Content::Emoji(
                            Some(EmojiSource::Still(ImageSource::Render(self.image.clone()))),
                            ":wave:".into(),
                        ),
                        style: PieceStyle {
                            action: Some(super::Action::InspectEmoji {
                                id: 42,
                                name: "wave".into(),
                                animated: false,
                            }),
                            ..Default::default()
                        },
                    }],
                )
                .emoji_size(rems(2.))
                .line_height(rems(2.))
                .selectable()
                .on_action(move |action, _, _| {
                    assert!(matches!(action, super::Action::InspectEmoji { id: 42, .. }));
                    clicks.set(clicks.get() + 1);
                })
                .on_action_hover(move |event, _, _| hovers.borrow_mut().push(event)),
            )
    }
}

#[gpui::test]
fn custom_emoji_hover_and_click_preserve_drag_selection(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let clicks = Rc::new(Cell::new(0));
    let hovers = Rc::new(RefCell::new(Vec::new()));
    let image = Arc::new(gpui::RenderImage::new(
        [image::Frame::new(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([255; 4]),
        ))]
        .into_iter()
        .collect::<Vec<_>>(),
    ));
    let (_, cx) = cx.add_window_view(|_, _| InspectableEmoji {
        image,
        clicks: clicks.clone(),
        hovers: hovers.clone(),
    });
    draw(cx);
    let inside = point(px(12.), px(12.));
    let outside = point(px(80.), px(12.));
    cx.simulate_mouse_move(outside, None, gpui::Modifiers::none());
    cx.simulate_mouse_move(inside, None, gpui::Modifiers::none());
    assert!(hovers.borrow().last().is_some_and(|event| {
        event.hovered && event.piece == 0 && event.bounds.contains(&inside)
    }));
    cx.simulate_click(inside, gpui::Modifiers::none());
    assert_eq!(clicks.get(), 1);
    cx.simulate_mouse_down(inside, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(
        outside,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::none(),
    );
    cx.simulate_mouse_up(outside, gpui::MouseButton::Left, gpui::Modifiers::none());
    assert_eq!(
        clicks.get(),
        1,
        "a selection drag must not open the emoji card"
    );
    cx.update(|window, cx| {
        assert_eq!(
            gpui_base::TextSelection::selected_text(window, cx),
            ":wave:"
        );
    });
    assert!(hovers.borrow().last().is_some_and(|event| !event.hovered));
}
