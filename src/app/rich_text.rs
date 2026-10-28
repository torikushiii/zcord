use std::{
    cell::RefCell,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    ops::Range,
    rc::Rc,
    sync::Arc,
};

use gpui::{prelude::*, *};
use gpui_component::ActiveTheme;

use crate::theme;

mod selection;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) mod metrics {
    use std::cell::Cell;
    thread_local! {
        pub static SHAPES: Cell<usize> = const { Cell::new(0) };
        pub static LAYOUTS: Cell<usize> = const { Cell::new(0) };
        pub static PAINTS: Cell<usize> = const { Cell::new(0) };
        pub static EMOJIS: Cell<usize> = const { Cell::new(0) };
    }
}

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

pub enum EmojiSource {
    Still(ImageSource),
    Animated(Arc<RenderImage>),
}

pub enum Content {
    Text(SharedString),
    Emoji(Option<EmojiSource>, SharedString),
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
    selectable: bool,
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
        selectable: false,
        on_action: None,
    }
}

impl RichText {
    pub fn selectable(mut self) -> Self {
        self.selectable = true;
        self
    }

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
const LINK: u32 = theme::LINK;
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
            _ if link => style.color.unwrap_or_else(|| rgb(LINK).into()),
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

    /// Where unscaled body text puts its baseline. Measured from a shaped run rather than the
    /// font's reported metrics, which can differ from what shaping uses and push text low.
    fn baseline(&self, window: &mut Window) -> Pixels {
        let shaped = self.shape("x", &PieceStyle::default(), false, window);
        (self.line_height - shaped.ascent - shaped.descent) / 2. + shaped.ascent
    }

    fn shape(
        &self,
        text: &str,
        style: &PieceStyle,
        hovered: bool,
        window: &mut Window,
    ) -> ShapedLine {
        #[cfg(test)]
        metrics::SHAPES.set(metrics::SHAPES.get() + 1);
        let run = self.run(style, text.len(), hovered);
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        window.text_system().shape_line_by_hash(
            hasher.finish(),
            text.len(),
            self.size(style),
            &[run],
            None,
            || SharedString::from(text.to_owned()),
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

fn hash_chip(chip: Option<Chip>, hasher: &mut impl Hasher) {
    match chip {
        None => 0u8.hash(hasher),
        Some(Chip::Code) => 1u8.hash(hasher),
        Some(Chip::Mention { color }) => {
            2u8.hash(hasher);
            color.hash(hasher);
        }
        Some(Chip::Spoiler { revealed }) => {
            3u8.hash(hasher);
            revealed.hash(hasher);
        }
        Some(Chip::Timestamp) => 4u8.hash(hasher),
    }
}

/// Everything that can alter shaped atoms or their size. Shaped lines carry their decoration
/// runs, so colours/underlines are included too; re-shaping on those rare changes is cheaper than
/// recolouring every run at paint. Emoji image loading is paint-time, and `group` only affects
/// chip painting, so neither belongs here.
fn layout_fingerprint(pieces: &[Piece], fonts: &Fonts, wrap: bool, truncate: bool) -> u64 {
    let mut hasher = DefaultHasher::new();
    wrap.hash(&mut hasher);
    truncate.hash(&mut hasher);
    fonts.base.hash(&mut hasher);
    fonts.mono.hash(&mut hasher);
    fonts.color.hash(&mut hasher);
    fonts.font_size.hash(&mut hasher);
    fonts.line_height.hash(&mut hasher);
    fonts.emoji_size.hash(&mut hasher);
    for piece in pieces {
        match &piece.content {
            Content::Text(text) => {
                0u8.hash(&mut hasher);
                text.hash(&mut hasher);
            }
            Content::Emoji(_, sequence) => {
                1u8.hash(&mut hasher);
                sequence.hash(&mut hasher);
            }
        }
        let style = &piece.style;
        style.weight.hash(&mut hasher);
        style.italic.hash(&mut hasher);
        style.underline.hash(&mut hasher);
        style.strikethrough.hash(&mut hasher);
        style.color.hash(&mut hasher);
        style.scale.map(f32::to_bits).hash(&mut hasher);
        style.mono.hash(&mut hasher);
        hash_chip(style.chip, &mut hasher);
        style.nowrap.hash(&mut hasher);
        style.cell.hash(&mut hasher);
        matches!(style.action, Some(Action::OpenUrl(_))).hash(&mut hasher);
    }
    hasher.finish()
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

impl Line {
    fn text_top(&self, shaped: &ShapedLine, text_height: Pixels, baseline: Pixels) -> Pixels {
        // GPUI centers each shaped run internally; compensate to share a baseline.
        self.y + (self.height - text_height) / 2. + baseline
            - (text_height - shaped.ascent - shaped.descent) / 2.
            - shaped.ascent
    }

    fn visible(&self, origin: Point<Pixels>, mask: Bounds<Pixels>, overdraw: Pixels) -> bool {
        let top = origin.y + self.y;
        top < mask.bottom() + overdraw && top + self.height > mask.top() - overdraw
    }
}

#[derive(Clone, Default)]
struct Layout {
    wrap_width: Option<Pixels>,
    truncated_to: Option<Pixels>,
    lines: Vec<Line>,
    size: Size<Pixels>,
}

struct LayoutCache {
    wrap: bool,
    truncate: bool,
    atoms: Option<Vec<Atom>>,
    layouts: Vec<Rc<Layout>>,
}

struct CachedLayout {
    fingerprint: u64,
    cache: Rc<RefCell<LayoutCache>>,
    baseline: Pixels,
}

const MAX_LAYOUTS: usize = 4;

impl LayoutCache {
    fn new(wrap: bool, truncate: bool) -> Self {
        Self {
            wrap,
            truncate,
            atoms: None,
            layouts: Vec::new(),
        }
    }

    fn get(
        &mut self,
        pieces: &[Piece],
        fonts: &Fonts,
        width: Option<Pixels>,
        window: &mut Window,
    ) -> Rc<Layout> {
        let wrap_width = width.filter(|_| self.wrap);
        let truncated_to = width.filter(|_| self.truncate);
        if let Some(index) = self.layouts.iter().position(|layout| {
            layout.wrap_width == wrap_width && layout.truncated_to == truncated_to
        }) {
            let layout = self.layouts.remove(index);
            self.layouts.push(layout.clone());
            return layout;
        }
        let atoms = self
            .atoms
            .get_or_insert_with(|| build_atoms(pieces, fonts, self.wrap, window));
        let layout = Rc::new(layout_pieces(
            pieces,
            fonts,
            atoms,
            self.wrap,
            self.truncate,
            width,
            window,
        ));
        if self.layouts.len() >= MAX_LAYOUTS {
            self.layouts.remove(0);
        }
        self.layouts.push(layout.clone());
        layout
    }
}

#[derive(Clone)]
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
            Content::Emoji(..) => {
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
    atoms: &[Atom],
    wrap: bool,
    truncate: bool,
    width_limit: Option<Pixels>,
    window: &mut Window,
) -> Layout {
    #[cfg(test)]
    metrics::LAYOUTS.set(metrics::LAYOUTS.get() + 1);
    let wrap_width = width_limit.filter(|_| wrap);
    let truncated_to = width_limit.filter(|_| truncate);
    let mut atoms = atoms.to_vec();
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
            // Zero-width intrinsic probes measure whole words. Splitting every
            // word into glyphs here repeatedly shapes thousands of unused tails.
            && limit > Pixels::ZERO
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
        let fingerprint = layout_fingerprint(&self.pieces, &fonts, self.wrap, self.truncate);
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
                        let cache =
                            Rc::new(RefCell::new(LayoutCache::new(self.wrap, self.truncate)));
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
                let cache = Rc::new(RefCell::new(LayoutCache::new(self.wrap, self.truncate)));
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
                        .then(|| Rc::new(selection::Selection::new(window, cx))),
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
                }
                let hidden = matches!(piece.style.chip, Some(Chip::Spoiler { revealed: false }));
                if let Content::Emoji(Some(source), _) = &piece.content
                    && !hidden
                {
                    let mut element = match source {
                        EmojiSource::Still(path) => img(path.clone())
                            .id(("emoji", fragment.piece))
                            .size(emoji_size)
                            .flex_none()
                            .into_any_element(),
                        EmojiSource::Animated(image) => {
                            crate::media::animated::image(image.clone())
                                .size(emoji_size)
                                .flex_none()
                                .id(("emoji", fragment.piece))
                                .into_any_element()
                        }
                    };
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
        for (hitbox, _) in &prepaint.hitboxes {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
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
                    Chip::Code => (rgba(theme::CODE_BACKGROUND).into(), px(4.)),
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
