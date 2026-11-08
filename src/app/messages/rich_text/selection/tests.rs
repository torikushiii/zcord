use super::super::PieceStyle;
use super::*;
use gpui::{Context, Modifiers, MouseUpEvent, Render, TestAppContext, div, prelude::*};
use std::cell::Cell;

#[test]
fn words_span_formatting_and_preserve_unicode_boundaries() {
    let pieces = [
        Piece::text("hel", PieceStyle::default()),
        Piece::text("lo cafe\u{301}", PieceStyle::default()),
    ];
    let geometry = Geometry::new(&pieces, &Rc::default(), Bounds::default());
    assert_eq!(&geometry.text[word_at(&geometry.text, 4)], "hello");
    assert_eq!(&geometry.text[word_at(&geometry.text, 8)], "cafe\u{301}");
    assert_eq!(word_at("hello", 5), 5..5);
}

#[test]
fn copying_preserves_emoji_and_original_line_breaks() {
    let pieces = [
        Piece::text("hello\n", PieceStyle::default()),
        Piece {
            content: Content::Emoji(None, "\u{1f600}".into()),
            style: PieceStyle::default(),
        },
        Piece::text(" world", PieceStyle::default()),
    ];
    let geometry = Geometry::new(&pieces, &Rc::default(), Bounds::default());
    let len = geometry.text.len();
    let data = Data {
        geometry,
        local: Some(0..len),
        ..Default::default()
    };
    assert_eq!(data.copied_text(), "hello\n\u{1f600} world");
}

#[test]
fn reading_order_handles_partially_scrolled_rows() {
    let points = [
        point(px(5.), px(-100.)),
        point(px(0.), px(-1.)),
        point(px(0.), px(1.)),
        point(px(10.), px(1.)),
    ];
    assert!(
        points
            .windows(2)
            .all(|pair| reading_order(pair[0]) < reading_order(pair[1]))
    );
}

struct TestMessages {
    actions: Rc<Cell<usize>>,
}

struct BlankLineMessage;

impl Render for BlankLineMessage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(gpui_base::TextSelectionLayer)
            .child(
                div()
                    .w(px(240.))
                    .text_size(px(16.))
                    .line_height(px(22.))
                    .child(
                        super::super::rich_text(
                            "blank-lines",
                            vec![Piece::text("one\n\nthree", PieceStyle::default())],
                        )
                        .selectable(),
                    ),
            )
    }
}

impl Render for TestMessages {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let actions = self.actions.clone();
        div()
            .size_full()
            .child(gpui_base::TextSelectionLayer)
            .child(
                div()
                    .w(px(240.))
                    .text_size(px(16.))
                    .line_height(px(22.))
                    .child(
                        super::super::rich_text(
                            "message",
                            vec![
                                Piece::text("al", PieceStyle::default()),
                                Piece::text(
                                    "pha beta\ngamma delta",
                                    PieceStyle {
                                        action: Some(super::super::Action::OpenUrl(
                                            "https://example.com".into(),
                                        )),
                                        ..Default::default()
                                    },
                                ),
                            ],
                        )
                        .selectable()
                        .on_action(move |_, _, _| actions.set(actions.get() + 1)),
                    )
                    .child(
                        super::super::rich_text(
                            "next-message",
                            vec![Piece::text("next message", PieceStyle::default())],
                        )
                        .selectable(),
                    ),
            )
    }
}

#[gpui::test]
fn pointer_drag_selects_multiple_lines_in_both_directions(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (_, cx) = cx.add_window_view(|_, _| TestMessages {
        actions: Rc::new(Cell::new(0)),
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let start = point(px(0.), px(5.));
    let end = point(px(230.), px(30.));
    for (start, end) in [(start, end), (end, start)] {
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        cx.update(|window, cx| {
            assert_eq!(
                TextSelection::selected_text(window, cx),
                "alpha beta\ngamma delta"
            );
        });
    }
}

#[gpui::test]
fn multi_click_selects_words_across_pieces_and_select_all_works(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (_, cx) = cx.add_window_view(|_, _| TestMessages {
        actions: Rc::new(Cell::new(0)),
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let position = point(px(1.), px(5.));
    for (click_count, expected) in [(2, "alpha"), (3, "alpha beta\ngamma delta")] {
        cx.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Left,
            click_count,
            ..Default::default()
        });
        cx.simulate_event(MouseUpEvent {
            position,
            button: MouseButton::Left,
            click_count,
            ..Default::default()
        });
        cx.update(|window, cx| assert_eq!(TextSelection::selected_text(window, cx), expected));
    }
    cx.simulate_event(MouseDownEvent {
        position,
        button: MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    cx.simulate_event(MouseUpEvent {
        position,
        button: MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    cx.simulate_keystrokes("ctrl-a");
    cx.update(|window, cx| {
        assert_eq!(
            TextSelection::selected_text(window, cx),
            "alpha beta\ngamma delta"
        )
    });
}

#[gpui::test]
fn dragging_from_a_blank_line_preserves_its_logical_offset(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (_, cx) = cx.add_window_view(|_, _| BlankLineMessage);
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let start = point(px(0.), px(30.));
    let end = point(px(230.), px(55.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(px(5.), px(55.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| assert_eq!(TextSelection::selected_text(window, cx), "\nthree"));
}

#[gpui::test]
fn selection_crosses_paragraphs_and_copies_through_the_app_root(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (_, cx) = cx.add_window_view(|window, cx| {
        let content = cx.new(|_| TestMessages {
            actions: Rc::new(Cell::new(0)),
        });
        gpui_component::Root::new(content, window, cx)
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let start = point(px(0.), px(5.));
    let end = point(px(230.), px(55.));
    for (start, end) in [(start, end), (end, start)] {
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        cx.simulate_keystrokes("ctrl-c");
        cx.update(|window, cx| {
            let expected = "alpha beta\ngamma delta\nnext message";
            assert_eq!(TextSelection::selected_text(window, cx), expected);
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some(expected)
            );
        });
    }
}

#[gpui::test]
fn links_open_on_release_but_not_after_a_drag_or_repaint(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let actions = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view(|_, _| TestMessages {
        actions: actions.clone(),
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let position = point(px(40.), px(5.));
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    assert_eq!(actions.get(), 0);
    cx.update(|window, cx| {
        window.refresh();
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    assert_eq!(actions.get(), 1);
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(px(130.), px(5.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_move(position, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    assert_eq!(actions.get(), 1);
}
