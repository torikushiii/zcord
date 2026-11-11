use super::layout::{
    CachedLayout, Fonts, Layout, LayoutCache, MENTION, emoji_inset, layout_fingerprint,
};
use super::*;

pub struct RichTextState {
    fonts: Fonts,
    layout: Rc<RefCell<LayoutCache>>,
    baseline: Pixels,
    interaction: Option<Interaction>,
}

#[derive(Clone)]
struct Interaction {
    selection: Option<Rc<selection::Selection>>,
    pressed: Rc<RefCell<Option<ActionPress>>>,
}

struct ActionPress {
    piece: usize,
    position: Point<Pixels>,
}

pub struct RichTextPrepaint {
    layout: Rc<Layout>,
    emojis: Vec<AnyElement>,
    hitboxes: Vec<(Hitbox, usize)>,
}

impl IntoElement for RichText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RichText {
    type RequestLayoutState = RichTextState;
    type PrepaintState = RichTextPrepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let text_style = window.text_style();
        let rem_size = window.rem_size();
        let fonts = Fonts {
            base: text_style.font(),
            mono: cx.theme().mono_font_family.clone(),
            color: text_style.color,
            underline: text_style.underline,
            font_size: text_style.font_size.to_pixels(rem_size),
            line_height: self.line_height.map_or_else(
                || text_style.line_height_in_pixels(rem_size),
                |line_height| line_height.to_pixels(rem_size),
            ),
            emoji_size: self.emoji_size.map_or_else(
                || text_style.font_size.to_pixels(rem_size) * self.emoji_scale,
                |size| size.to_pixels(rem_size),
            ),
            emoji_gap: self.emoji_gap.map(|gap| gap.to_pixels(rem_size)),
        };
        let fingerprint = layout_fingerprint(
            &self.pieces,
            &fonts,
            self.wrap,
            self.truncate,
            self.line_clamp,
        );
        // Taffy probes the same text at several widths during one layout pass, and the element
        // lives across frames. Share shaped words and completed layouts until content changes;
        // GPUI drops this state in frames where the element is not drawn (virtualized rows).
        let (layout, baseline) = match id {
            Some(id) => window.with_element_state(id, |cached: Option<CachedLayout>, window| {
                let (cache, baseline) = match cached {
                    Some(cached) if cached.fingerprint == fingerprint => {
                        (cached.cache, cached.baseline)
                    }
                    _ => {
                        let cache = Rc::new(RefCell::new(LayoutCache::new(
                            self.wrap,
                            self.truncate,
                            self.line_clamp,
                        )));
                        let baseline = fonts.baseline(window);
                        (cache, baseline)
                    }
                };
                (
                    (cache.clone(), baseline),
                    CachedLayout {
                        fingerprint,
                        cache,
                        baseline,
                    },
                )
            }),
            None => {
                let cache = Rc::new(RefCell::new(LayoutCache::new(
                    self.wrap,
                    self.truncate,
                    self.line_clamp,
                )));
                let baseline = fonts.baseline(window);
                (cache, baseline)
            }
        };
        let measure = {
            let fonts = fonts.clone();
            let layout = layout.clone();
            let pieces = self.pieces.clone();
            move |known: Size<Option<Pixels>>,
                  available: Size<AvailableSpace>,
                  window: &mut Window,
                  _: &mut App| {
                let wrap_width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                layout
                    .borrow_mut()
                    .get(&pieces, &fonts, wrap_width, window)
                    .size
            }
        };
        let mut style = Style::default();
        if self.wrap || self.truncate {
            style.min_size.width = px(0.).into();
        }
        if self.truncate {
            style.max_size.width = relative(1.).into();
            style.flex_shrink = 1.;
        }
        let layout_id = window.request_measured_layout(style, measure);
        (
            layout_id,
            RichTextState {
                fonts,
                layout,
                baseline,
                interaction: None,
            },
        )
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let layout = state.layout.borrow_mut().get(
            &self.pieces,
            &state.fonts,
            Some(bounds.size.width),
            window,
        );
        let interaction =
            window.with_element_state(id.expect("rich text has a stable id"), |state, window| {
                let state: Interaction = state.unwrap_or_else(|| Interaction {
                    selection: self
                        .selectable
                        .then(|| selection::Selection::new(window, cx)),
                    pressed: Rc::new(RefCell::new(None)),
                });
                (state.clone(), state)
            });
        if let Some(selection) = &interaction.selection {
            selection.register(&self.pieces, &layout, bounds, window, cx);
        }
        state.interaction = Some(interaction);
        let mut emojis = Vec::new();
        let mut hitboxes = Vec::new();
        let emoji_size = state.fonts.emoji_size;
        let mask = window.content_mask().bounds;
        for line in layout
            .lines
            .iter()
            .filter(|line| line.visible(bounds.origin, mask, state.fonts.font_size))
        {
            for fragment in &line.fragments {
                let piece = &self.pieces[fragment.piece];
                let origin = bounds.origin + point(fragment.x, line.y);
                if piece.style.action.is_some() {
                    let fragment_bounds = Bounds::new(origin, size(fragment.width, line.height));
                    hitboxes.push((
                        window.insert_hitbox(fragment_bounds, HitboxBehavior::Normal),
                        fragment.piece,
                    ));
                    if let (Some(handler), Some(action)) =
                        (&self.on_action_hover, &piece.style.action)
                    {
                        let handler = handler.clone();
                        let action = action.clone();
                        let piece = fragment.piece;
                        let mut hover = div()
                            .id(("action-hover", hitboxes.len()))
                            .w(fragment_bounds.size.width)
                            .h(fragment_bounds.size.height)
                            .on_hover(move |hovered, window, cx| {
                                handler(
                                    ActionHover {
                                        action: action.clone(),
                                        piece,
                                        hovered: *hovered,
                                        bounds: fragment_bounds,
                                    },
                                    window,
                                    cx,
                                );
                            })
                            .into_any_element();
                        hover.prepaint_as_root(
                            fragment_bounds.origin,
                            size(
                                AvailableSpace::Definite(fragment_bounds.size.width),
                                AvailableSpace::Definite(fragment_bounds.size.height),
                            ),
                            window,
                            cx,
                        );
                        emojis.push(hover);
                    }
                }
                let hidden = matches!(piece.style.chip, Some(Chip::Spoiler { revealed: false }));
                if let Content::Emoji(Some(source), _) = &piece.content
                    && !hidden
                {
                    let icon = piece.style.icon;
                    let emoji_size = if icon.is_some() {
                        state.fonts.font_size
                    } else {
                        emoji_size
                    };
                    let round = icon == Some(IconShape::Round);
                    let mut element = match source {
                        EmojiSource::Still(path) => img(path.clone())
                            .id(("emoji", fragment.piece))
                            .size(emoji_size)
                            .flex_none()
                            .when(round, |this| this.rounded_full())
                            .into_any_element(),
                        EmojiSource::Animated(image) => {
                            crate::media::animated::image(image.clone())
                                .size(emoji_size)
                                .flex_none()
                                .when(round, |this| this.rounded_full())
                                .id(("emoji", fragment.piece))
                                .into_any_element()
                        }
                        EmojiSource::Icon(path) => svg()
                            .path(path.clone())
                            .size(emoji_size)
                            .flex_none()
                            .text_color(state.fonts.color(&piece.style, false))
                            .into_any_element(),
                    };
                    let inset = if icon.is_some() {
                        (fragment.width - emoji_size) / 2.
                    } else {
                        emoji_inset(fragment.width, emoji_size, state.fonts.emoji_gap.is_some())
                    };
                    let top = (line.height - emoji_size) / 2.;
                    element.prepaint_as_root(
                        origin + point(inset, top),
                        size(
                            AvailableSpace::Definite(emoji_size),
                            AvailableSpace::Definite(emoji_size),
                        ),
                        window,
                        cx,
                    );
                    emojis.push(element);
                    #[cfg(test)]
                    metrics::EMOJIS.set(metrics::EMOJIS.get() + 1);
                }
            }
        }
        RichTextPrepaint {
            layout,
            emojis,
            hitboxes,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let fonts = &state.fonts;
        let interaction = state
            .interaction
            .as_ref()
            .expect("rich text was prepainted");
        let selection_range = interaction
            .selection
            .as_ref()
            .and_then(|selection| selection.range());
        let hovered_piece = prepaint
            .hitboxes
            .iter()
            .find(|(hitbox, _)| hitbox.is_hovered(window))
            .map(|(_, piece)| *piece);
        let pieces = self.pieces.clone();
        let group_of = |piece: usize| {
            let style = &pieces[piece].style;
            style.chip.map(|chip| (chip, style.group))
        };

        let mask = window.content_mask().bounds;
        let baseline = state.baseline;
        for line in prepaint
            .layout
            .lines
            .iter()
            .filter(|line| line.visible(bounds.origin, mask, fonts.font_size))
        {
            let chip_height = fonts.line_height.min(line.height);
            let chip_top = line.y + (line.height - chip_height) / 2.;
            let mut index = 0;
            while index < line.fragments.len() {
                let first = &line.fragments[index];
                let Some((chip, group)) = group_of(first.piece) else {
                    index += 1;
                    continue;
                };
                let mut end = index + 1;
                while end < line.fragments.len()
                    && group_of(line.fragments[end].piece) == Some((chip, group))
                {
                    end += 1;
                }
                let last = &line.fragments[end - 1];
                let hovered = (index..end).any(|i| Some(line.fragments[i].piece) == hovered_piece);
                let chip_bounds = Bounds::new(
                    bounds.origin + point(first.x, chip_top),
                    size(last.x + last.width - first.x, chip_height),
                );
                let (background, radius): (Hsla, Pixels) = match chip {
                    Chip::Code => (rgba(theme::code_background()).into(), px(4.)),
                    Chip::Timestamp => (rgb(theme::timestamp_bg()).opacity(0.06).into(), px(4.)),
                    Chip::Mention { color: Some(color) } if !hovered => {
                        (rgba((color << 8) | 0x1A).into(), px(3.))
                    }
                    Chip::Mention { color: Some(color) } => {
                        (rgba((color << 8) | 0x4D).into(), px(3.))
                    }
                    Chip::Mention { .. } if hovered => (rgb(MENTION).into(), px(3.)),
                    Chip::Mention { .. } => (rgba(theme::mention_bg()).into(), px(3.)),
                    Chip::Spoiler { revealed: false } if hovered => {
                        (rgb(theme::spoiler_hover()).into(), px(3.))
                    }
                    Chip::Spoiler { revealed: false } => (rgb(theme::spoiler_bg()).into(), px(3.)),
                    Chip::Spoiler { revealed: true } => (
                        rgb(theme::spoiler_revealed_bg()).opacity(0.1).into(),
                        px(3.),
                    ),
                    Chip::SearchHit => (rgba(0xFDB8334D).into(), px(0.)),
                };
                window.paint_quad(quad(
                    chip_bounds,
                    radius,
                    background,
                    Edges::default(),
                    transparent_black(),
                    BorderStyle::default(),
                ));
                index = end;
            }

            for fragment in &line.fragments {
                let piece = &pieces[fragment.piece];
                if matches!(piece.style.chip, Some(Chip::Spoiler { revealed: false })) {
                    continue;
                }
                if let (Some(selection), Some(range)) = (&interaction.selection, &selection_range) {
                    selection.paint_fragment(fragment, line, bounds.origin, range, window, cx);
                }
                let Some(shaped) = &fragment.shaped else {
                    continue;
                };
                let Content::Text(text) = &piece.content else {
                    continue;
                };
                let hovered = Some(fragment.piece) == hovered_piece;
                let reshaped;
                let shaped = if hovered && !fragment.range.is_empty() {
                    reshaped =
                        fonts.shape(&text[fragment.range.clone()], &piece.style, true, window);
                    &reshaped
                } else {
                    shaped
                };
                let text_height = fonts.line_height.min(line.height);
                let origin = bounds.origin
                    + point(
                        fragment.x + fragment.pad_left,
                        line.text_top(shaped, text_height, baseline),
                    );
                shaped
                    .paint(origin, text_height, TextAlign::Left, None, window, cx)
                    .ok();
                #[cfg(test)]
                metrics::PAINTS.set(metrics::PAINTS.get() + 1);
            }
        }

        for element in &mut prepaint.emojis {
            element.paint(window, cx);
        }

        if let Some(selection) = &interaction.selection {
            selection.listen(window);
        }
        // Later cursor registrations override the selectable text's general I-beam.
        for (hitbox, _) in &prepaint.hitboxes {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        if prepaint.hitboxes.is_empty() {
            return;
        }
        let targets: Rc<Vec<(Hitbox, usize)>> = Rc::new(prepaint.hitboxes.clone());
        let move_targets = targets.clone();
        window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, _| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let now = move_targets
                .iter()
                .find(|(hitbox, _)| hitbox.is_hovered(window))
                .map(|(_, piece)| *piece);
            if now != hovered_piece {
                window.refresh();
            }
        });
        let Some(handler) = self.on_action.clone() else {
            return;
        };
        let pressed = interaction.pressed.clone();
        let down = pressed.clone();
        let down_targets = targets.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            *down.borrow_mut() = (event.click_count == 1)
                .then(|| {
                    down_targets
                        .iter()
                        .find(|(hitbox, _)| hitbox.is_hovered(window))
                        .map(|(_, piece)| ActionPress {
                            piece: *piece,
                            position: event.position,
                        })
                })
                .flatten();
        });
        let moved = pressed.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, _| {
            if phase == DispatchPhase::Bubble
                && moved.borrow().as_ref().is_some_and(|press| {
                    (event.position.x - press.position.x).abs() > px(3.)
                        || (event.position.y - press.position.y).abs() > px(3.)
                })
            {
                moved.borrow_mut().take();
            }
        });
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            let Some(press) = pressed.borrow_mut().take() else {
                return;
            };
            if let Some((_, piece)) = targets.iter().find(|(hitbox, _)| hitbox.is_hovered(window))
                && *piece == press.piece
                && let Some(action) = &pieces[*piece].style.action
            {
                gpui_base::TextSelection::end(window, cx);
                handler(action, window, cx);
                cx.stop_propagation();
            }
        });
    }
}
