use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui::{prelude::*, *};
use gpui_component::ActiveTheme;

use crate::theme;

#[derive(Clone)]
pub enum Action {
    OpenUrl(SharedString),
    RevealSpoiler(u16),
}

#[derive(Clone, Copy, PartialEq)]
pub enum Chip {
    Code,
    Mention { color: Option<u32> },
    Spoiler { revealed: bool },
    Timestamp,
}

#[derive(Clone, Default)]
pub struct PieceStyle {
    pub weight: Option<FontWeight>,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub color: Option<Hsla>,
    pub scale: Option<f32>,
    pub mono: bool,
    pub chip: Option<Chip>,
    pub group: u32,
    pub nowrap: bool,
    pub cell: bool,
    pub action: Option<Action>,
}

pub enum Content {
    Text(SharedString),
    Emoji(Option<ImageSource>),
}

pub struct Piece {
    pub content: Content,
    pub style: PieceStyle,
}

impl Piece {
    pub fn text(text: impl Into<SharedString>, style: PieceStyle) -> Self {
        Self {
            content: Content::Text(text.into()),
            style,
        }
    }
}

type ActionHandler = Rc<dyn Fn(&Action, &mut Window, &mut App)>;

pub struct RichText {
    id: ElementId,
    pieces: Rc<Vec<Piece>>,
    line_height: Option<Rems>,
    emoji_size: Option<Rems>,
    emoji_scale: f32,
    wrap: bool,
    truncate: bool,
    on_action: Option<ActionHandler>,
}

pub fn rich_text(id: impl Into<ElementId>, pieces: Vec<Piece>) -> RichText {
    RichText {
        id: id.into(),
        pieces: Rc::new(pieces),
        line_height: None,
        emoji_size: None,
        emoji_scale: 1.375,
        wrap: true,
        truncate: false,
        on_action: None,
    }
}

impl RichText {
    pub fn line_height(mut self, line_height: Rems) -> Self {
        self.line_height = Some(line_height);
        self
    }

    pub fn emoji_size(mut self, size: Rems) -> Self {
        self.emoji_size = Some(size);
        self
    }

    pub fn emoji_scale(mut self, scale: f32) -> Self {
        self.emoji_scale = scale;
        self
    }

    pub fn single_line(mut self) -> Self {
        self.wrap = false;
        self
    }

    pub fn truncate(mut self) -> Self {
        self.wrap = false;
        self.truncate = true;
        self
    }

    pub fn on_action(mut self, handler: impl Fn(&Action, &mut Window, &mut App) + 'static) -> Self {
        self.on_action = Some(Rc::new(handler));
        self
    }
}

const SPOILER_HIDDEN: u32 = 0x2A2A30;
const SPOILER_HOVER: u32 = 0x33333A;
const LINK: u32 = 0x00A8FC;
const MENTION: u32 = 0x5865F2;
const MENTION_TEXT: u32 = 0xC9CDFB;

#[derive(Clone)]
struct Fonts {
    base: Font,
    mono: SharedString,
    color: Hsla,
    font_size: Pixels,
    line_height: Pixels,
    emoji_size: Pixels,
}

impl Fonts {
    fn run(&self, style: &PieceStyle, len: usize, hovered: bool) -> TextRun {
        let mut font = self.base.clone();
        if let Some(weight) = style.weight {
            font.weight = weight;
        }
        if style.italic {
            font.style = FontStyle::Italic;
        }
        if style.mono {
            font.family = self.mono.clone();
        }
        let link = matches!(style.action, Some(Action::OpenUrl(_)));
        let color = match style.chip {
            Some(Chip::Mention { color: Some(color) }) if !hovered => rgb(color).into(),
            Some(Chip::Mention { .. }) if hovered => white(),
            Some(Chip::Mention { .. }) => rgb(MENTION_TEXT).into(),
            _ if link => rgb(LINK).into(),
            _ => style.color.unwrap_or(self.color),
        };
        let underline = style.underline || (link && hovered);
        TextRun {
            len,
            font,
            color,
            background_color: None,
            underline: underline.then(|| UnderlineStyle {
                thickness: px(1.),
                color: Some(color),
                wavy: false,
            }),
            strikethrough: style.strikethrough.then(|| StrikethroughStyle {
                thickness: px(1.),
                color: Some(color),
            }),
        }
    }

    fn size(&self, style: &PieceStyle) -> Pixels {
        self.font_size * style.scale.unwrap_or(1.)
    }

    fn shape(
        &self,
        text: &str,
        style: &PieceStyle,
        hovered: bool,
        window: &mut Window,
    ) -> ShapedLine {
        let run = self.run(style, text.len(), hovered);
        window.text_system().shape_line(
            SharedString::from(text.to_owned()),
            self.size(style),
            &[run],
            None,
        )
    }

    fn padding(style: &PieceStyle, font_size: Pixels) -> Pixels {
        match style.chip {
            Some(Chip::Code) | Some(Chip::Timestamp) => font_size * 0.2,
            Some(Chip::Mention { .. }) => px(2.),
            _ => Pixels::ZERO,
        }
    }
}

#[derive(Clone)]
struct Fragment {
    piece: usize,
    range: Range<usize>,
    shaped: Option<ShapedLine>,
    x: Pixels,
    width: Pixels,
    pad_left: Pixels,
}

#[derive(Clone)]
struct Line {
    y: Pixels,
    height: Pixels,
    fragments: Vec<Fragment>,
}

#[derive(Clone, Default)]
struct Layout {
    wrap_width: Option<Pixels>,
    truncated_to: Option<Pixels>,
    lines: Vec<Line>,
    size: Size<Pixels>,
}

struct Atom {
    piece: usize,
    range: Range<usize>,
    shaped: Option<ShapedLine>,
    width: Pixels,
    trailing_space: Pixels,
    pad_left: Pixels,
    pad_right: Pixels,
    break_before: bool,
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF
        | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

fn word_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut previous: Option<char> = None;
    for (index, c) in text.char_indices() {
        if let Some(previous) = previous {
            let after_space = previous == ' ' && c != ' ';
            let cjk_boundary = (is_cjk(c) && previous != ' ') || is_cjk(previous) && c != ' ';
            if (after_space || cjk_boundary) && index > start {
                ranges.push(start..index);
                start = index;
            }
        }
        previous = Some(c);
    }
    if start < text.len() {
        ranges.push(start..text.len());
    }
    ranges
}

fn text_lines(text: &str) -> impl Iterator<Item = (&str, usize)> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let current = rest?;
        match current
            .char_indices()
            .find(|(_, c)| matches!(c, '\n' | '\u{2028}' | '\u{2029}'))
        {
            Some((index, separator)) => {
                rest = Some(&current[index + separator.len_utf8()..]);
                Some((&current[..index], separator.len_utf8()))
            }
            None => {
                rest = None;
                Some((current, 0))
            }
        }
    })
}

fn build_atoms(pieces: &[Piece], fonts: &Fonts, wrap: bool, window: &mut Window) -> Vec<Atom> {
    let mut atoms = Vec::new();
    let mut pending_break = false;
    for (piece_index, piece) in pieces.iter().enumerate() {
        let style = &piece.style;
        let padding = Fonts::padding(style, fonts.size(style));
        let first_atom = atoms.len();
        match &piece.content {
            Content::Emoji(_) => {
                atoms.push(Atom {
                    piece: piece_index,
                    range: 0..0,
                    shaped: None,
                    width: fonts.emoji_size + fonts.font_size * 0.1,
                    trailing_space: Pixels::ZERO,
                    pad_left: Pixels::ZERO,
                    pad_right: Pixels::ZERO,
                    break_before: std::mem::take(&mut pending_break),
                });
            }
            Content::Text(text) => {
                let mut offset = 0;
                for (line_index, (line, separator)) in text_lines(text).enumerate() {
                    if line_index > 0 {
                        if pending_break {
                            atoms.push(Atom {
                                piece: usize::MAX,
                                range: 0..0,
                                shaped: None,
                                width: Pixels::ZERO,
                                trailing_space: Pixels::ZERO,
                                pad_left: Pixels::ZERO,
                                pad_right: Pixels::ZERO,
                                break_before: true,
                            });
                        }
                        pending_break = true;
                    }
                    let ranges = if wrap && !style.nowrap {
                        word_ranges(line)
                    } else if line.is_empty() {
                        Vec::new()
                    } else {
                        vec![0..line.len()]
                    };
                    for range in ranges {
                        let word = &line[range.clone()];
                        let shaped = fonts.shape(word, style, false, window);
                        let trimmed = word.trim_end_matches(' ').len();
                        let mut width = shaped.width();
                        let trailing_space = width - shaped.x_for_index(trimmed);
                        let mut pad_left = Pixels::ZERO;
                        let cell = fonts.size(style) * word.chars().count() as f32;
                        if style.cell && width < cell {
                            pad_left = (cell - width) / 2.;
                            width = cell;
                        }
                        atoms.push(Atom {
                            piece: piece_index,
                            range: offset + range.start..offset + range.end,
                            shaped: Some(shaped),
                            width,
                            trailing_space,
                            pad_left,
                            pad_right: Pixels::ZERO,
                            break_before: std::mem::take(&mut pending_break),
                        });
                    }
                    offset += line.len() + separator;
                }
            }
        }
        if atoms.len() > first_atom && padding > Pixels::ZERO {
            atoms[first_atom].pad_left = padding;
            atoms[first_atom].width += padding;
            let last = atoms.len() - 1;
            atoms[last].pad_right = padding;
            atoms[last].width += padding;
        }
    }
    if pending_break {
        atoms.push(Atom {
            piece: usize::MAX,
            range: 0..0,
            shaped: None,
            width: Pixels::ZERO,
            trailing_space: Pixels::ZERO,
            pad_left: Pixels::ZERO,
            pad_right: Pixels::ZERO,
            break_before: true,
        });
    }
    atoms
}

fn split_atom(
    atom: Atom,
    available: Pixels,
    pieces: &[Piece],
    fonts: &Fonts,
    window: &mut Window,
) -> (Atom, Option<Atom>) {
    let (Some(shaped), Content::Text(text)) = (&atom.shaped, &pieces[atom.piece].content) else {
        return (atom, None);
    };
    let word = &text[atom.range.clone()];
    let mut split = shaped
        .index_for_x(available - atom.pad_left)
        .unwrap_or(word.len());
    while split > 0 && !word.is_char_boundary(split) {
        split -= 1;
    }
    if split == 0 {
        split = word.chars().next().map_or(word.len(), char::len_utf8);
    }
    if split >= word.len() {
        return (atom, None);
    }
    let style = &pieces[atom.piece].style;
    let head = fonts.shape(&word[..split], style, false, window);
    let tail = fonts.shape(&word[split..], style, false, window);
    let tail_trimmed = word[split..].trim_end_matches(' ').len();
    let first = Atom {
        piece: atom.piece,
        range: atom.range.start..atom.range.start + split,
        width: head.width() + atom.pad_left,
        shaped: Some(head),
        trailing_space: Pixels::ZERO,
        pad_left: atom.pad_left,
        pad_right: Pixels::ZERO,
        break_before: atom.break_before,
    };
    let second = Atom {
        piece: atom.piece,
        range: atom.range.start + split..atom.range.end,
        width: tail.width() + atom.pad_right,
        trailing_space: tail.width() - tail.x_for_index(tail_trimmed),
        shaped: Some(tail),
        pad_left: Pixels::ZERO,
        pad_right: atom.pad_right,
        break_before: false,
    };
    (first, Some(second))
}

fn layout_pieces(
    pieces: &[Piece],
    fonts: &Fonts,
    wrap: bool,
    truncate: bool,
    width_limit: Option<Pixels>,
    window: &mut Window,
) -> Layout {
    let wrap_width = width_limit.filter(|_| wrap);
    let truncated_to = width_limit.filter(|_| truncate);
    let mut atoms = build_atoms(pieces, fonts, wrap, window);
    atoms.reverse();
    let mut lines: Vec<Line> = vec![Line {
        y: Pixels::ZERO,
        height: fonts.line_height,
        fragments: Vec::new(),
    }];
    let mut x = Pixels::ZERO;
    while let Some(mut atom) = atoms.pop() {
        let mut new_line = atom.break_before;
        if let Some(limit) = wrap_width
            && x > Pixels::ZERO
            && x + atom.width - atom.trailing_space > limit
        {
            new_line = true;
        }
        if new_line {
            let previous = lines.last().expect("at least one line");
            lines.push(Line {
                y: previous.y + previous.height,
                height: fonts.line_height,
                fragments: Vec::new(),
            });
            x = Pixels::ZERO;
            atom.break_before = false;
        }
        if atom.piece == usize::MAX {
            continue;
        }
        if let Some(limit) = wrap_width
            && x == Pixels::ZERO
            && atom.width - atom.trailing_space > limit
        {
            let (head, tail) = split_atom(atom, limit, pieces, fonts, window);
            atom = head;
            if let Some(mut tail) = tail {
                tail.break_before = true;
                atoms.push(tail);
            }
        }
        let line = lines.last_mut().expect("at least one line");
        if atom.shaped.is_none() {
            line.height = line.height.max(fonts.emoji_size);
        }
        line.fragments.push(Fragment {
            piece: atom.piece,
            range: atom.range,
            shaped: atom.shaped,
            x,
            width: atom.width,
            pad_left: atom.pad_left,
        });
        x += atom.width;
    }
    if let Some(limit) = truncated_to {
        for line in &mut lines {
            truncate_line(line, limit, pieces, fonts, window);
        }
    }
    let mut y = Pixels::ZERO;
    let mut width = Pixels::ZERO;
    for line in &mut lines {
        line.y = y;
        y += line.height;
        if let Some(last) = line.fragments.last() {
            width = width.max(last.x + last.width);
        }
    }
    Layout {
        wrap_width,
        truncated_to,
        lines,
        size: size(width, y),
    }
}

fn truncate_line(
    line: &mut Line,
    limit: Pixels,
    pieces: &[Piece],
    fonts: &Fonts,
    window: &mut Window,
) {
    let end = |fragment: &Fragment| fragment.x + fragment.width;
    if line.fragments.last().is_none_or(|last| end(last) <= limit) {
        return;
    }
    let is_text = |piece: usize| matches!(pieces[piece].content, Content::Text(_));
    let cut = line
        .fragments
        .iter()
        .position(|fragment| end(fragment) > limit)
        .unwrap_or(line.fragments.len());
    let Some(style_piece) = line.fragments[..=cut.min(line.fragments.len() - 1)]
        .iter()
        .rev()
        .map(|fragment| fragment.piece)
        .chain(0..pieces.len())
        .find(|piece| is_text(*piece))
    else {
        line.fragments.truncate(cut);
        return;
    };
    let ellipsis = fonts.shape("…", &pieces[style_piece].style, false, window);
    let room = limit - ellipsis.width();
    let mut kept: Vec<Fragment> = Vec::with_capacity(cut + 1);
    for fragment in line.fragments.drain(..) {
        if end(&fragment) <= room {
            kept.push(fragment);
            continue;
        }
        if let (Some(shaped), Content::Text(text)) =
            (&fragment.shaped, &pieces[fragment.piece].content)
        {
            let word = &text[fragment.range.clone()];
            let available = room - fragment.x - fragment.pad_left;
            let mut split = if available > Pixels::ZERO {
                shaped.index_for_x(available).unwrap_or(word.len())
            } else {
                0
            };
            while split > 0 && !word.is_char_boundary(split) {
                split -= 1;
            }
            let head_text = word[..split].trim_end();
            if !head_text.is_empty() {
                let head = fonts.shape(head_text, &pieces[fragment.piece].style, false, window);
                kept.push(Fragment {
                    range: fragment.range.start..fragment.range.start + head_text.len(),
                    width: head.width() + fragment.pad_left,
                    shaped: Some(head),
                    ..fragment
                });
            }
        }
        break;
    }
    let x = kept.last().map_or(Pixels::ZERO, end);
    kept.push(Fragment {
        piece: style_piece,
        range: 0..0,
        width: ellipsis.width(),
        shaped: Some(ellipsis),
        x,
        pad_left: Pixels::ZERO,
    });
    line.fragments = kept;
}

pub struct RichTextState {
    fonts: Fonts,
    layout: Rc<RefCell<Layout>>,
}

pub struct RichTextPrepaint {
    layout: Layout,
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
        _id: Option<&GlobalElementId>,
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
            font_size: text_style.font_size.to_pixels(rem_size),
            line_height: self.line_height.map_or_else(
                || text_style.line_height_in_pixels(rem_size),
                |line_height| line_height.to_pixels(rem_size),
            ),
            emoji_size: self.emoji_size.map_or_else(
                || text_style.font_size.to_pixels(rem_size) * self.emoji_scale,
                |size| size.to_pixels(rem_size),
            ),
        };
        let layout = Rc::new(RefCell::new(Layout::default()));
        let measure = {
            let fonts = fonts.clone();
            let layout = layout.clone();
            let pieces = self.pieces.clone();
            let wrap = self.wrap;
            let truncate = self.truncate;
            move |known: Size<Option<Pixels>>,
                  available: Size<AvailableSpace>,
                  window: &mut Window,
                  _: &mut App| {
                let wrap_width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                let computed = layout_pieces(&pieces, &fonts, wrap, truncate, wrap_width, window);
                let size = computed.size;
                *layout.borrow_mut() = computed;
                size
            }
        };
        let mut style = Style::default();
        if self.truncate {
            style.min_size.width = px(0.).into();
            style.max_size.width = relative(1.).into();
            style.flex_shrink = 1.;
        }
        let layout_id = window.request_measured_layout(style, measure);
        (layout_id, RichTextState { fonts, layout })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let stale = {
            let layout = state.layout.borrow();
            (self.wrap && layout.wrap_width != Some(bounds.size.width))
                || (self.truncate && layout.truncated_to != Some(bounds.size.width))
        };
        if stale {
            let computed = layout_pieces(
                &self.pieces,
                &state.fonts,
                self.wrap,
                self.truncate,
                Some(bounds.size.width),
                window,
            );
            *state.layout.borrow_mut() = computed;
        }
        let layout = state.layout.borrow().clone();
        let mut emojis = Vec::new();
        let mut hitboxes = Vec::new();
        let emoji_size = state.fonts.emoji_size;
        for line in &layout.lines {
            for fragment in &line.fragments {
                let piece = &self.pieces[fragment.piece];
                let origin = bounds.origin + point(fragment.x, line.y);
                if piece.style.action.is_some() {
                    let fragment_bounds = Bounds::new(origin, size(fragment.width, line.height));
                    hitboxes.push((
                        window.insert_hitbox(fragment_bounds, HitboxBehavior::Normal),
                        fragment.piece,
                    ));
                }
                let hidden = matches!(piece.style.chip, Some(Chip::Spoiler { revealed: false }));
                if let Content::Emoji(Some(path)) = &piece.content
                    && !hidden
                {
                    let mut element = img(path.clone())
                        .size(emoji_size)
                        .flex_none()
                        .into_any_element();
                    let inset = (fragment.width - emoji_size) / 2.;
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
        let hovered_piece = prepaint
            .hitboxes
            .iter()
            .find(|(hitbox, _)| hitbox.is_hovered(window))
            .map(|(_, piece)| *piece);
        for (hitbox, _) in &prepaint.hitboxes {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        let pieces = self.pieces.clone();
        let group_of = |piece: usize| {
            let style = &pieces[piece].style;
            style.chip.map(|chip| (chip, style.group))
        };

        for line in &prepaint.layout.lines {
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
                    Chip::Code => (rgb(theme::SELECTED).into(), px(4.)),
                    Chip::Timestamp => (hsla(0., 0., 1., 0.06), px(4.)),
                    Chip::Mention { color: Some(color) } if !hovered => {
                        (rgba((color << 8) | 0x1A).into(), px(3.))
                    }
                    Chip::Mention { color: Some(color) } => {
                        (rgba((color << 8) | 0x4D).into(), px(3.))
                    }
                    Chip::Mention { .. } if hovered => (rgb(MENTION).into(), px(3.)),
                    Chip::Mention { .. } => (rgba((MENTION << 8) | 0x4D).into(), px(3.)),
                    Chip::Spoiler { revealed: false } if hovered => {
                        (rgb(SPOILER_HOVER).into(), px(3.))
                    }
                    Chip::Spoiler { revealed: false } => (rgb(SPOILER_HIDDEN).into(), px(3.)),
                    Chip::Spoiler { revealed: true } => (hsla(0., 0., 1., 0.1), px(3.)),
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
                        line.y + (line.height - text_height) / 2.,
                    );
                shaped
                    .paint(origin, text_height, TextAlign::Left, None, window, cx)
                    .ok();
            }
        }

        for element in &mut prepaint.emojis {
            element.paint(window, cx);
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
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if let Some((_, piece)) = targets.iter().find(|(hitbox, _)| hitbox.is_hovered(window))
                && let Some(action) = &pieces[*piece].style.action
            {
                handler(action, window, cx);
                cx.stop_propagation();
            }
        });
    }
}
