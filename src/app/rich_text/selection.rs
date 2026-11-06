use std::{
    cell::RefCell,
    ops::Range,
    rc::{Rc, Weak},
};

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, FocusHandle, Hitbox, HitboxBehavior, KeyDownEvent,
    MouseButton, MouseDownEvent, Pixels, Point, Subscription, Window, WindowId, fill, point, px,
    size,
};
use gpui_base::{
    TextSelection, TextSelectionEvent, TextSelectionHandle, TextSelectionRegistration,
    TextSelectionSnapshot,
};
use gpui_component::ActiveTheme;
use unicode_segmentation::UnicodeSegmentation;

use super::{Content, Fragment, Layout, Line, Piece};

#[derive(Default)]
struct Geometry {
    text: String,
    offsets: Vec<usize>,
    layout: Rc<Layout>,
    bounds: Bounds<Pixels>,
}

impl Geometry {
    fn new(pieces: &[Piece], layout: &Rc<Layout>, bounds: Bounds<Pixels>) -> Self {
        let mut text = String::new();
        let offsets = pieces
            .iter()
            .map(|piece| {
                let offset = text.len();
                match &piece.content {
                    Content::Text(value) | Content::Emoji(_, value) => text.push_str(value),
                }
                offset
            })
            .collect();
        Self {
            text,
            offsets,
            layout: layout.clone(),
            bounds,
        }
    }

    fn offset_at(&self, position: Point<Pixels>, nearest: bool) -> usize {
        let point = position - self.bounds.origin;
        if point.y < px(0.) {
            return 0;
        }
        let Some((line_index, line)) = self
            .layout
            .lines
            .iter()
            .enumerate()
            .find(|(_, line)| point.y < line.y + line.height)
        else {
            return self.text.len();
        };
        let Some(fragment) = line
            .fragments
            .iter()
            .find(|fragment| point.x < fragment.x + fragment.width)
            .or_else(|| line.fragments.last())
        else {
            let previous = self.layout.lines[..line_index]
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, line)| line.fragments.last().map(|fragment| (index, fragment)));
            let (mut offset, breaks) = previous.map_or((0, line_index), |(index, fragment)| {
                let offset = self.offsets[fragment.piece];
                let end = if fragment.shaped.is_some() {
                    offset + fragment.range.end
                } else {
                    self.offsets
                        .get(fragment.piece + 1)
                        .copied()
                        .unwrap_or(self.text.len())
                };
                (end, line_index - index)
            });
            for _ in 0..breaks {
                if let Some(separator @ ('\n' | '\u{2028}' | '\u{2029}')) =
                    self.text[offset..].chars().next()
                {
                    offset += separator.len_utf8();
                }
            }
            return offset;
        };
        let offset = self.offsets[fragment.piece];
        let x = point.x - fragment.x - fragment.pad_left;
        match &fragment.shaped {
            Some(shaped) => {
                offset
                    + fragment.range.start
                    + if nearest {
                        shaped.closest_index_for_x(x)
                    } else {
                        shaped.index_for_x(x).unwrap_or(fragment.range.len())
                    }
                    .min(fragment.range.len())
            }
            None => {
                offset
                    + if x < fragment.width / 2. {
                        0
                    } else {
                        self.offsets
                            .get(fragment.piece + 1)
                            .copied()
                            .unwrap_or(self.text.len())
                            - offset
                    }
            }
        }
    }

    fn range(&self, snapshot: TextSelectionSnapshot) -> Option<Range<usize>> {
        let points = snapshot.window_points()?;
        let anchor = self.offset_at(points.anchor(), true);
        let cursor = self.offset_at(points.cursor(), true);
        Some(anchor.min(cursor)..anchor.max(cursor))
    }
}

#[derive(Default)]
struct Data {
    geometry: Geometry,
    snapshot: Option<TextSelectionSnapshot>,
    local: Option<Range<usize>>,
}

impl Data {
    fn range(&self) -> Option<Range<usize>> {
        self.local
            .clone()
            .or_else(|| self.geometry.range(self.snapshot?))
    }

    fn copied_text(&self) -> String {
        self.range()
            .and_then(|range| self.geometry.text.get(range))
            .unwrap_or_default()
            .to_owned()
    }
}

thread_local! {
    static SELECTIONS: RefCell<Vec<Weak<Selection>>> = const { RefCell::new(Vec::new()) };
}

/// gpui-base drops any participant that did not register during a frame, clearing its
/// selection. A reused cached view never prepaints its rich text, so a frame drawn for an
/// unrelated panel would wipe the selection. Call this from a view that renders every frame.
/// A `Selection` lives as long as its element state, which GPUI keeps for reused views and
/// drops once the text leaves the tree, so whatever still upgrades is still on screen.
pub(crate) fn keep_registered(window: &mut Window, cx: &mut App) {
    let window_id = window.window_handle().window_id();
    let live: Vec<_> = SELECTIONS.with_borrow_mut(|selections| {
        selections.retain(|selection| selection.strong_count() > 0);
        selections.iter().filter_map(Weak::upgrade).collect()
    });
    for selection in live
        .iter()
        .filter(|selection| selection.window == window_id)
    {
        selection.reregister(window, cx);
    }
}

struct Registered {
    hitbox: Hitbox,
    bounds: Bounds<Pixels>,
    text_bounds: Vec<Bounds<Pixels>>,
}

impl Registered {
    fn registration(&self) -> TextSelectionRegistration {
        TextSelectionRegistration::new(self.hitbox.clone(), self.bounds)
            .with_document_order(reading_order(self.bounds.origin))
            .with_text_bounds(self.text_bounds.clone())
    }
}

pub(super) struct Selection {
    handle: TextSelectionHandle,
    focus: FocusHandle,
    data: Rc<RefCell<Data>>,
    registered: RefCell<Option<Registered>>,
    window: WindowId,
    _subscription: Subscription,
}

impl Selection {
    pub(super) fn new(window: &Window, cx: &mut App) -> Rc<Self> {
        let selection = Rc::new(Self::build(window, cx));
        SELECTIONS.with_borrow_mut(|selections| selections.push(Rc::downgrade(&selection)));
        selection
    }

    fn build(window: &Window, cx: &mut App) -> Self {
        let handle = TextSelectionHandle::new("", cx);
        let focus = cx.focus_handle();
        let data = Rc::new(RefCell::new(Data::default()));
        let copy = data.clone();
        handle.copy_with(move |_| copy.borrow().copied_text(), cx);
        let clear = data.clone();
        handle.clear_with(move |_| clear.borrow_mut().local = None, cx);
        let target = focus.clone();
        handle.focus_with(move |window, cx| target.focus(window, cx), cx);
        let events = data.clone();
        let window = window.window_handle();
        let subscription = handle.subscribe(
            move |event, cx| {
                if let TextSelectionEvent::SelectionChanged(snapshot) = event {
                    events.borrow_mut().snapshot = *snapshot;
                    _ = window.update(cx, |_, window, _| window.refresh());
                }
            },
            cx,
        );
        Self {
            handle,
            focus,
            data,
            registered: RefCell::new(None),
            window: window.window_id(),
            _subscription: subscription,
        }
    }

    fn reregister(&self, window: &mut Window, cx: &mut App) {
        let Some(registration) = self
            .registered
            .borrow()
            .as_ref()
            .map(Registered::registration)
        else {
            return;
        };
        self.handle.register(registration, window, cx);
    }

    pub(super) fn register(
        &self,
        pieces: &[Piece],
        layout: &Rc<Layout>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let geometry = Geometry::new(pieces, layout, bounds);
        let changed = self.data.borrow().geometry.text != geometry.text;
        if changed && (self.handle.snapshot(cx).is_some() || self.handle.has_local_selection(cx)) {
            TextSelection::clear(window, cx);
        }
        let text_bounds = layout
            .lines
            .iter()
            .flat_map(|line| {
                line.fragments.iter().map(move |fragment| {
                    Bounds::new(
                        bounds.origin + point(fragment.x, line.y),
                        size(fragment.width, line.height),
                    )
                })
            })
            .collect();
        self.data.borrow_mut().geometry = geometry;
        let registered = Registered {
            hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
            bounds,
            text_bounds,
        };
        window.set_focus_handle(&self.focus, cx);
        self.handle.register(registered.registration(), window, cx);
        *self.registered.borrow_mut() = Some(registered);
    }

    pub(super) fn range(&self) -> Option<Range<usize>> {
        self.data.borrow().range()
    }

    pub(super) fn paint_fragment(
        &self,
        fragment: &Fragment,
        line: &Line,
        origin: Point<Pixels>,
        range: &Range<usize>,
        window: &mut Window,
        cx: &App,
    ) {
        let data = self.data.borrow();
        let offset = data.geometry.offsets[fragment.piece];
        let (start, end) = if let Some(shaped) = &fragment.shaped {
            let start = range.start.max(offset + fragment.range.start);
            let end = range.end.min(offset + fragment.range.end);
            if start >= end {
                return;
            }
            (
                fragment.pad_left + shaped.x_for_index(start - offset - fragment.range.start),
                fragment.pad_left + shaped.x_for_index(end - offset - fragment.range.start),
            )
        } else {
            let end = data
                .geometry
                .offsets
                .get(fragment.piece + 1)
                .copied()
                .unwrap_or(data.geometry.text.len());
            if range.start >= end || range.end <= offset {
                return;
            }
            (px(0.), fragment.width)
        };
        window.paint_quad(fill(
            Bounds::new(
                origin + point(fragment.x + start.min(end), line.y),
                size((end - start).abs(), line.height),
            ),
            cx.theme().selection,
        ));
    }

    pub(super) fn listen(self: &Rc<Self>, window: &mut Window) {
        let Some(hitbox) = self
            .registered
            .borrow()
            .as_ref()
            .map(|registered| registered.hitbox.clone())
        else {
            return;
        };
        window.set_cursor_style(CursorStyle::IBeam, &hitbox);
        let selection = self.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || event.click_count < 2
                || !hitbox.is_hovered(window)
            {
                return;
            }
            let range = {
                let data = selection.data.borrow();
                let offset = data.geometry.offset_at(event.position, false);
                if event.click_count == 2 {
                    word_at(&data.geometry.text, offset)
                } else {
                    0..data.geometry.text.len()
                }
            };
            selection.select(range, window, cx);
            cx.stop_propagation();
        });
        let selection = self.clone();
        window.on_key_event(move |event: &KeyDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !selection.focus.is_focused(window) {
                return;
            }
            let key = &event.keystroke;
            if key.modifiers.control && !key.modifiers.alt && key.key == "a" {
                let len = selection.data.borrow().geometry.text.len();
                selection.select(0..len, window, cx);
                cx.stop_propagation();
            }
        });
    }

    fn select(&self, range: Range<usize>, window: &mut Window, cx: &mut App) {
        TextSelection::clear(window, cx);
        self.data.borrow_mut().local = Some(range);
        self.handle.set_local_selection(true, cx);
        self.focus.focus(window, cx);
        window.refresh();
    }
}

fn word_at(text: &str, offset: usize) -> Range<usize> {
    text.split_word_bound_indices()
        .find_map(|(start, word)| {
            let end = start + word.len();
            (start <= offset && offset < end).then_some(start..end)
        })
        .unwrap_or(offset..offset)
}

fn reading_order(position: Point<Pixels>) -> u64 {
    // Order virtualized, currently rendered participants by their reading position.
    let ordered = |value: Pixels| {
        let bits = f32::from(value).to_bits();
        if bits & (1 << 31) == 0 {
            bits ^ (1 << 31)
        } else {
            !bits
        }
    };
    (u64::from(ordered(position.y)) << 32) | u64::from(ordered(position.x))
}

#[cfg(test)]
mod tests {
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
}
