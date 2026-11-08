use std::{cell::RefCell, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Element, ElementId, Font, FontFallbacks, FontFeatures,
    FontId, GlobalElementId, GlyphId, InspectorElementId, LayoutId, Pixels, Point, Rgba, Role,
    SharedString, Size, Style, TextRun, Window, WrapBoundary, WrappedLine, div, point, prelude::*,
    px, relative, rgb, size, white,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    fonts::{self, DisplayFont},
    store::NameStyle,
};

use super::messages::message_body::{label, wrapped_label};

mod effects;

use effects::{Clock, Effect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Surface {
    FontOnly,
    Account {
        hovered: bool,
    },
    Profile,
    /// Under the profile editor's name field, whose caret can't follow letter spacing, so the
    /// name is set without it.
    Field,
}

impl Surface {
    fn wraps(self) -> bool {
        matches!(self, Self::Profile | Self::Field)
    }

    fn animates(self) -> bool {
        match self {
            Self::Profile | Self::Field => true,
            Self::Account { hovered } => hovered || crate::settings::always_animate(),
            Self::FontOnly => false,
        }
    }

    fn tracks(self) -> bool {
        self != Self::Field
    }
}

pub(super) fn render(
    id: impl Into<ElementId>,
    text: &str,
    style: Option<&NameStyle>,
    surface: Surface,
    cx: &App,
) -> AnyElement {
    let id = id.into();
    let style =
        style.filter(|_| surface == Surface::Field || crate::settings::display_name_styles());
    let font = fonts::display_font(style.and_then(|style| style.font_id), cx);
    // Chat and member names inherit role/default colors, never profile colors or effects.
    let colors: Vec<Rgba> = style
        .filter(|_| surface != Surface::FontOnly)
        .map(|style| {
            style
                .colors
                .iter()
                .map(|color| rgb(color & 0xFFFFFF))
                .collect()
        })
        .unwrap_or_default();
    let effect =
        (!colors.is_empty()).then(|| Effect::from_id(style.and_then(|style| style.effect_id)));
    if font.is_none() && effect.is_none() {
        return div()
            .min_w_0()
            .max_w_full()
            .when(!surface.wraps(), |this| this.truncate())
            .child(if surface.wraps() {
                wrapped_label(id, text)
            } else {
                label(id, text)
            })
            .into_any_element();
    }
    let text: SharedString = text
        .split_ascii_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .into();
    div()
        .id(id.clone())
        .min_w_0()
        .max_w_full()
        .role(Role::Label)
        .aria_label(text.clone())
        .child(DisplayName {
            id,
            text,
            font,
            colors,
            effect,
            surface,
        })
        .into_any_element()
}

struct DisplayName {
    id: ElementId,
    text: SharedString,
    font: Option<DisplayFont>,
    colors: Vec<Rgba>,
    effect: Option<Effect>,
    surface: Surface,
}

#[derive(Clone, PartialEq)]
struct Metrics {
    font: Font,
    size: Pixels,
    line_height: Pixels,
    tracking: Pixels,
    wrap: bool,
}

struct Glyph {
    font: FontId,
    id: GlyphId,
    advance: Pixels,
    origin: Point<Pixels>,
    center: Point<Pixels>,
    cluster: usize,
    emoji: bool,
}

struct Line {
    glyphs: Vec<Glyph>,
    width: Pixels,
    baseline: Pixels,
}

struct Layout {
    lines: Vec<Line>,
    size: Size<Pixels>,
    ch_width: Pixels,
}

#[derive(Default)]
struct LayoutCache(Vec<(Option<Pixels>, Rc<Layout>)>);

impl LayoutCache {
    fn get(
        &mut self,
        text: &SharedString,
        metrics: &Metrics,
        width: Option<Pixels>,
        window: &Window,
    ) -> Rc<Layout> {
        let width = width.map(|width| width.max(px(0.)));
        if let Some((_, layout)) = self.0.iter().find(|(cached, _)| *cached == width) {
            return layout.clone();
        }
        let layout = Rc::new(layout(text, metrics, width, window));
        if self.0.len() == 4 {
            self.0.remove(0);
        }
        self.0.push((width, layout.clone()));
        layout
    }
}

struct Cached {
    text: SharedString,
    metrics: Metrics,
    layouts: Rc<RefCell<LayoutCache>>,
    clock: Rc<RefCell<Clock>>,
}

struct NameState {
    metrics: Metrics,
    layouts: Rc<RefCell<LayoutCache>>,
    clock: Rc<RefCell<Clock>>,
}

fn run(text: &str, metrics: &Metrics) -> TextRun {
    TextRun {
        len: text.len(),
        font: metrics.font.clone(),
        color: white(),
        ..Default::default()
    }
}

fn shape(
    text: SharedString,
    metrics: &Metrics,
    width: Option<Pixels>,
    window: &Window,
) -> Vec<WrappedLine> {
    let runs = [run(&text, metrics)];
    window
        .text_system()
        .shape_text(text, metrics.size, &runs, width, None)
        .map(|lines| lines.into_vec())
        .unwrap_or_default()
}

fn line_geometry(shaped: &[WrappedLine], metrics: &Metrics) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut text_offset = 0;
    for shaped in shaped {
        let clusters: Vec<usize> = shaped
            .text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .collect();
        let mut wraps = shaped.wrap_boundaries().iter().peekable();
        let mut glyphs = Vec::new();
        let mut first_x = px(0.);
        let mut tracking = px(0.);
        let mut last_cluster = None;
        let mut width = px(0.);
        let mut top = metrics.line_height * lines.len();
        let offset_y =
            (metrics.line_height - shaped.ascent() - shaped.descent()) / 2. + shaped.ascent();
        for (run_ix, run) in shaped.runs().iter().enumerate() {
            for (glyph_ix, glyph) in run.glyphs.iter().enumerate() {
                if wraps.peek() == Some(&&WrapBoundary { run_ix, glyph_ix }) {
                    wraps.next();
                    lines.push(Line {
                        glyphs,
                        width,
                        baseline: top + offset_y,
                    });
                    glyphs = Vec::new();
                    first_x = glyph.position.x;
                    width = px(0.);
                    tracking = px(0.);
                    last_cluster = None;
                    top = metrics.line_height * lines.len();
                }
                let cluster = clusters
                    .partition_point(|index| *index <= glyph.index)
                    .saturating_sub(1);
                if last_cluster.is_some_and(|previous| previous != cluster) {
                    tracking += metrics.tracking;
                }
                last_cluster = Some(cluster);
                let next_x = run
                    .glyphs
                    .get(glyph_ix + 1)
                    .map(|next| next.position.x)
                    .or_else(|| {
                        shaped
                            .runs()
                            .get(run_ix + 1)?
                            .glyphs
                            .first()
                            .map(|next| next.position.x)
                    })
                    .unwrap_or(shaped.unwrapped_layout.width);
                let x = glyph.position.x - first_x + tracking;
                let advance = (next_x - glyph.position.x).max(px(0.));
                width = (x + advance).max(width);
                glyphs.push(Glyph {
                    font: run.font_id,
                    id: glyph.id,
                    advance,
                    origin: point(x, top + offset_y + glyph.position.y),
                    center: point(x + advance / 2., top + metrics.line_height / 2.),
                    cluster: text_offset + cluster,
                    emoji: glyph.is_emoji,
                });
            }
        }
        lines.push(Line {
            glyphs,
            width,
            baseline: top + offset_y,
        });
        text_offset += clusters.len();
    }
    // Combining marks/fallback-font runs in a grapheme move with the same transform.
    for line in &mut lines {
        let mut start = 0;
        while start < line.glyphs.len() {
            let cluster = line.glyphs[start].cluster;
            let end =
                start + line.glyphs[start..].partition_point(|glyph| glyph.cluster == cluster);
            let center = line.glyphs[start].center;
            for glyph in &mut line.glyphs[start..end] {
                glyph.center = center;
            }
            start = end;
        }
    }
    lines
}

fn text_width(text: &SharedString, metrics: &Metrics, window: &Window) -> Pixels {
    line_geometry(&shape(text.clone(), metrics, None, window), metrics)
        .iter()
        .map(|line| line.width)
        .max()
        .unwrap_or(px(0.))
}

fn truncated(
    text: &SharedString,
    metrics: &Metrics,
    width: Pixels,
    window: &Window,
) -> SharedString {
    if text_width(text, metrics, window) <= width {
        return text.clone();
    }
    let ellipsis: SharedString = "\u{2026}".into();
    if text_width(&ellipsis, metrics, window) > width {
        return SharedString::default();
    }
    let ends: Vec<usize> = text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .collect();
    let mut low = 0;
    let mut high = ends.len();
    while low < high {
        let mid = (low + high).div_ceil(2);
        let end = ends.get(mid).copied().unwrap_or(text.len());
        let candidate: SharedString = format!("{}\u{2026}", &text[..end]).into();
        if text_width(&candidate, metrics, window) <= width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let end = ends.get(low).copied().unwrap_or(text.len());
    format!("{}\u{2026}", &text[..end]).into()
}

fn layout(
    text: &SharedString,
    metrics: &Metrics,
    width: Option<Pixels>,
    window: &Window,
) -> Layout {
    let text = if !metrics.wrap {
        width.map_or_else(
            || text.clone(),
            |width| truncated(text, metrics, width, window),
        )
    } else {
        text.clone()
    };
    let mut wrap_width = width
        .filter(|_| metrics.wrap)
        .map(|width| width.max(px(1.)));
    let mut lines = line_geometry(&shape(text.clone(), metrics, wrap_width, window), metrics);
    // Native wrapping knows word/Unicode boundaries. Refit its width to include CSS tracking.
    if let Some(width) = width.filter(|_| metrics.wrap && metrics.tracking > px(0.)) {
        for _ in 0..4 {
            let overflow = lines
                .iter()
                .map(|line| line.width - width)
                .max()
                .unwrap_or(px(0.));
            if overflow <= px(0.) || wrap_width == Some(px(1.)) {
                break;
            }
            wrap_width = wrap_width.map(|wrap| (wrap - overflow).max(px(1.)));
            lines = line_geometry(&shape(text.clone(), metrics, wrap_width, window), metrics);
        }
    }
    let measured = lines.iter().map(|line| line.width).max().unwrap_or(px(0.));
    let ch_width = text_width(&"0".into(), metrics, window);
    Layout {
        size: size(
            width.map_or(measured, |width| measured.min(width)),
            metrics.line_height * lines.len().max(1),
        ),
        lines,
        ch_width,
    }
}

impl IntoElement for DisplayName {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for DisplayName {
    type RequestLayoutState = NameState;
    type PrepaintState = Rc<Layout>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, NameState) {
        let style = window.text_style();
        let mut font = style.font();
        if let Some(display) = self.font {
            let mut fallbacks = vec![font.family.to_string(), "Noto Sans".to_owned()];
            if let Some(existing) = &font.fallbacks {
                fallbacks.extend_from_slice(existing.fallback_list());
            }
            font.family = display.family.into();
            font.fallbacks = Some(FontFallbacks::from_fonts(fallbacks));
        }
        if self.effect == Some(Effect::Gummy) {
            let mut features = font.features.tag_value_list().to_vec();
            let disabled = ["kern", "liga", "clig", "dlig", "hlig", "calt"];
            features.retain(|(tag, _)| !disabled.contains(&tag.as_str()));
            features.extend(disabled.map(|tag| (tag.into(), 0)));
            font.features = FontFeatures(Arc::new(features));
        }
        let size = style.font_size.to_pixels(window.rem_size());
        let metrics = Metrics {
            font,
            size,
            line_height: style.line_height_in_pixels(window.rem_size()),
            tracking: size
                * self
                    .font
                    .filter(|_| self.surface.tracks())
                    .map_or(0., |font| font.letter_spacing_em),
            wrap: self.surface.wraps(),
        };
        let (layouts, clock) = window.with_element_state(
            id.expect("display name has an id"),
            |cached: Option<Cached>, _| {
                let cached = cached
                    .filter(|cached| cached.text == self.text && cached.metrics == metrics)
                    .unwrap_or_else(|| Cached {
                        text: self.text.clone(),
                        metrics: metrics.clone(),
                        layouts: Rc::new(RefCell::new(LayoutCache::default())),
                        clock: Rc::new(RefCell::new(Clock::default())),
                    });
                ((cached.layouts.clone(), cached.clock.clone()), cached)
            },
        );
        let measure = {
            let text = self.text.clone();
            let metrics = metrics.clone();
            let layouts = layouts.clone();
            move |known: Size<Option<Pixels>>,
                  available: Size<AvailableSpace>,
                  window: &mut Window,
                  _: &mut App| {
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                layouts
                    .borrow_mut()
                    .get(&text, &metrics, width, window)
                    .size
            }
        };
        let mut layout_style = Style::default();
        layout_style.min_size.width = px(0.).into();
        layout_style.max_size.width = relative(1.).into();
        layout_style.flex_shrink = 1.;
        (
            window.request_measured_layout(layout_style, measure),
            NameState {
                metrics,
                layouts,
                clock,
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut NameState,
        window: &mut Window,
        _: &mut App,
    ) -> Rc<Layout> {
        state
            .layouts
            .borrow_mut()
            .get(&self.text, &state.metrics, Some(bounds.size.width), window)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut NameState,
        layout: &mut Rc<Layout>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if bounds.is_empty() || !bounds.intersects(&window.content_mask().bounds) {
            return;
        }
        let requested = self.surface.animates()
            && self.effect.is_some_and(Effect::animated)
            && !cx.reduce_motion();
        let playing = requested && window.is_window_active();
        let elapsed = state
            .clock
            .borrow_mut()
            .advance(Instant::now(), requested, playing);
        if playing {
            window.request_animation_frame();
        }
        // Hover decorations belong to the current paint, not the cached layout.
        let style = window.text_style();
        effects::paint(
            self.effect,
            &self.colors,
            elapsed,
            bounds,
            &state.metrics,
            layout,
            style.color,
            style.underline,
            window,
        );
    }
}

#[cfg(test)]
mod tests;
