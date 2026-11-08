use super::*;

pub(super) const MENTION: u32 = 0x5865F2;

#[derive(Clone)]
pub(super) struct Fonts {
    pub(super) base: Font,
    pub(super) mono: SharedString,
    pub(super) color: Hsla,
    pub(super) underline: Option<UnderlineStyle>,
    pub(super) font_size: Pixels,
    pub(super) line_height: Pixels,
    pub(super) emoji_size: Pixels,
    /// `None` keeps the legacy centered 0.1em emoji slot.
    pub(super) emoji_gap: Option<Pixels>,
}

impl Fonts {
    pub(super) fn run(&self, style: &PieceStyle, len: usize, hovered: bool) -> TextRun {
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
        let color = self.color(style, hovered);
        let underline = style.underline || (link && hovered);
        TextRun {
            len,
            font,
            color,
            background_color: None,
            underline: if underline {
                Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(color),
                    wavy: false,
                })
            } else {
                self.underline.map(|mut style| {
                    style.color = Some(style.color.unwrap_or(color));
                    style
                })
            },
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
    pub(super) fn baseline(&self, window: &mut Window) -> Pixels {
        let shaped = self.shape("x", &PieceStyle::default(), false, window);
        (self.line_height - shaped.ascent - shaped.descent) / 2. + shaped.ascent
    }

    pub(super) fn shape(
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

    pub(super) fn color(&self, style: &PieceStyle, hovered: bool) -> Hsla {
        let link = matches!(style.action, Some(Action::OpenUrl(_)));
        match style.chip {
            Some(Chip::Mention { color: Some(color) }) if !hovered => rgb(color).into(),
            Some(Chip::Mention { .. }) if hovered => white(),
            Some(Chip::Mention { .. }) => rgb(theme::mention_text()).into(),
            _ if link => style.color.unwrap_or_else(|| rgb(theme::link()).into()),
            _ => style.color.unwrap_or(self.color),
        }
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
pub(super) fn layout_fingerprint(
    pieces: &[Piece],
    fonts: &Fonts,
    wrap: bool,
    truncate: bool,
    line_clamp: Option<usize>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    wrap.hash(&mut hasher);
    truncate.hash(&mut hasher);
    line_clamp.hash(&mut hasher);
    fonts.base.hash(&mut hasher);
    fonts.mono.hash(&mut hasher);
    fonts.color.hash(&mut hasher);
    fonts.underline.hash(&mut hasher);
    fonts.font_size.hash(&mut hasher);
    fonts.line_height.hash(&mut hasher);
    fonts.emoji_size.hash(&mut hasher);
    fonts.emoji_gap.hash(&mut hasher);
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
        style.icon.hash(&mut hasher);
        matches!(style.action, Some(Action::OpenUrl(_))).hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Clone)]
pub(super) struct Fragment {
    pub(super) piece: usize,
    pub(super) range: Range<usize>,
    pub(super) shaped: Option<ShapedLine>,
    pub(super) x: Pixels,
    pub(super) width: Pixels,
    pub(super) pad_left: Pixels,
}

#[derive(Clone)]
pub(super) struct Line {
    pub(super) y: Pixels,
    pub(super) height: Pixels,
    pub(super) fragments: Vec<Fragment>,
}

impl Line {
    pub(super) fn text_top(
        &self,
        shaped: &ShapedLine,
        text_height: Pixels,
        baseline: Pixels,
    ) -> Pixels {
        // GPUI centers each shaped run internally; compensate to share a baseline.
        self.y + (self.height - text_height) / 2. + baseline
            - (text_height - shaped.ascent - shaped.descent) / 2.
            - shaped.ascent
    }

    pub(super) fn visible(
        &self,
        origin: Point<Pixels>,
        mask: Bounds<Pixels>,
        overdraw: Pixels,
    ) -> bool {
        let top = origin.y + self.y;
        top < mask.bottom() + overdraw && top + self.height > mask.top() - overdraw
    }
}

#[derive(Clone, Default)]
pub(super) struct Layout {
    wrap_width: Option<Pixels>,
    truncated_to: Option<Pixels>,
    line_clamp: Option<usize>,
    pub(super) lines: Vec<Line>,
    pub(super) size: Size<Pixels>,
}

pub(super) struct LayoutCache {
    wrap: bool,
    truncate: bool,
    line_clamp: Option<usize>,
    atoms: Option<Vec<Atom>>,
    layouts: Vec<Rc<Layout>>,
}

pub(super) struct CachedLayout {
    pub(super) fingerprint: u64,
    pub(super) cache: Rc<RefCell<LayoutCache>>,
    pub(super) baseline: Pixels,
}

const MAX_LAYOUTS: usize = 4;

impl LayoutCache {
    pub(super) fn new(wrap: bool, truncate: bool, line_clamp: Option<usize>) -> Self {
        Self {
            wrap,
            truncate,
            line_clamp,
            atoms: None,
            layouts: Vec::new(),
        }
    }

    pub(super) fn get(
        &mut self,
        pieces: &[Piece],
        fonts: &Fonts,
        width: Option<Pixels>,
        window: &mut Window,
    ) -> Rc<Layout> {
        let wrap_width = width.filter(|_| self.wrap);
        let truncated_to = width.filter(|_| self.truncate);
        if let Some(index) = self.layouts.iter().position(|layout| {
            layout.wrap_width == wrap_width
                && layout.truncated_to == truncated_to
                && layout.line_clamp == self.line_clamp
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
            self.line_clamp,
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

pub(super) fn emoji_inset(
    fragment_width: Pixels,
    emoji_size: Pixels,
    explicit_gap: bool,
) -> Pixels {
    if explicit_gap {
        Pixels::ZERO
    } else {
        (fragment_width - emoji_size) / 2.
    }
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
                let width = if style.icon.is_some() {
                    fonts.font_size
                } else {
                    fonts.emoji_size + fonts.emoji_gap.unwrap_or(fonts.font_size * 0.1)
                };
                atoms.push(Atom {
                    piece: piece_index,
                    range: 0..0,
                    shaped: None,
                    width,
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
    line_clamp: Option<usize>,
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
    if let Some(clamp) = line_clamp
        && lines.len() > clamp
    {
        let hidden = lines[clamp..]
            .iter()
            .any(|line| line_has_content(line, pieces));
        lines.truncate(clamp);
        if hidden && clamp > 0 {
            let limit = truncated_to.or(wrap_width);
            truncate_line(
                lines.last_mut().expect("clamped to a visible line"),
                limit,
                true,
                pieces,
                fonts,
                window,
            );
        }
    }
    if let Some(limit) = truncated_to {
        for line in &mut lines {
            truncate_line(line, Some(limit), false, pieces, fonts, window);
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
        line_clamp,
        lines,
        size: size(width, y),
    }
}

fn line_has_content(line: &Line, pieces: &[Piece]) -> bool {
    line.fragments
        .iter()
        .any(|fragment| match &pieces[fragment.piece].content {
            Content::Text(text) => !text[fragment.range.clone()].trim().is_empty(),
            Content::Emoji(..) => true,
        })
}

fn truncate_line(
    line: &mut Line,
    limit: Option<Pixels>,
    force: bool,
    pieces: &[Piece],
    fonts: &Fonts,
    window: &mut Window,
) {
    let end = |fragment: &Fragment| fragment.x + fragment.width;
    if !force
        && limit.is_none_or(|limit| line.fragments.last().is_none_or(|last| end(last) <= limit))
    {
        return;
    }
    let is_text = |piece: usize| matches!(pieces[piece].content, Content::Text(_));
    let cut = limit.map_or(line.fragments.len(), |limit| {
        line.fragments
            .iter()
            .position(|fragment| end(fragment) > limit)
            .unwrap_or(line.fragments.len())
    });
    let search_end = cut.saturating_add(1).min(line.fragments.len());
    let Some(style_piece) = line.fragments[..search_end]
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
    let room = limit.map(|limit| limit - ellipsis.width());
    let mut kept: Vec<Fragment> = Vec::with_capacity(cut + 1);
    for fragment in line.fragments.drain(..) {
        if room.is_none_or(|room| end(&fragment) <= room) {
            kept.push(fragment);
            continue;
        }
        let room = room.expect("a missing room keeps every fragment");
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
