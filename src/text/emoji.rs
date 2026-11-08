use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, OnceLock},
};

use flate2::read::ZlibDecoder;
use gpui::{Image, ImageFormat};

static BUNDLE: &[u8] = include_bytes!("../../assets/twemoji.bin");
static SHORTCODES: &str = include_str!("../../assets/emoji_shortcodes.tsv");
static CATEGORY_DATA: &str = include_str!("../../assets/emoji_categories.tsv");
const MAX_SVG_SIZE: u64 = 1024 * 1024;
/// SVG sizes each emoji can be drawn at. gpui rasterizes an SVG at twice its size and the GPU
/// shrinks it to fit without mipmaps, so a raster much bigger than the emoji on screen comes
/// out jagged; each size here serves emoji shown at up to twice it.
const RASTER_SIZES: [u32; 6] = [16, 24, 32, 48, 64, 128];
/// The size the bundle's SVGs are packed with; see `scripts/pack_twemoji.py`.
const PACKED_SIZE: &str = r#"<svg width="128" height="128""#;

struct Entry<'a> {
    compressed: &'a [u8],
    images: [OnceLock<Option<Arc<Image>>>; RASTER_SIZES.len()],
}

fn raster_index(px: u32) -> usize {
    RASTER_SIZES
        .iter()
        .position(|&size| size * 2 >= px)
        .unwrap_or(RASTER_SIZES.len() - 1)
}

impl Entry<'_> {
    fn image(&self, index: usize) -> Option<&Arc<Image>> {
        self.images[index]
            .get_or_init(|| {
                let mut svg = Vec::new();
                ZlibDecoder::new(self.compressed)
                    .take(MAX_SVG_SIZE + 1)
                    .read_to_end(&mut svg)
                    .ok()?;
                if svg.len() as u64 > MAX_SVG_SIZE
                    || !svg.starts_with(b"<svg")
                    || !std::str::from_utf8(&svg)
                        .ok()?
                        .trim_end()
                        .ends_with("</svg>")
                {
                    return None;
                }
                let size = RASTER_SIZES[index];
                if let Some(rest) = svg.strip_prefix(PACKED_SIZE.as_bytes()) {
                    let mut sized = format!(r#"<svg width="{size}" height="{size}""#).into_bytes();
                    sized.extend_from_slice(rest);
                    svg = sized;
                }
                Some(Arc::new(Image::from_bytes(ImageFormat::Svg, svg)))
            })
            .as_ref()
    }
}

struct Bundle<'a> {
    exact: HashMap<String, usize>,
    unqualified: HashMap<String, usize>,
    display: HashMap<String, usize>,
    entries: Vec<Entry<'a>>,
}

fn take<const N: usize>(cursor: &mut &[u8]) -> Option<[u8; N]> {
    let value = cursor.get(..N)?.try_into().ok()?;
    *cursor = &cursor[N..];
    Some(value)
}

impl<'a> Bundle<'a> {
    fn parse(bytes: &'a [u8]) -> Option<Self> {
        let mut cursor = bytes;
        if take::<4>(&mut cursor)? != *b"TWE1" {
            return None;
        }
        let count = u32::from_le_bytes(take(&mut cursor)?) as usize;
        if count > cursor.len() / 11 {
            return None;
        }
        let mut exact = HashMap::with_capacity(count);
        let mut unqualified = HashMap::with_capacity(count);
        let mut display = HashMap::with_capacity(count * 2);
        let mut ranges = Vec::with_capacity(count);
        let mut previous = "";
        for index in 0..count {
            let name_len = u16::from_le_bytes(take(&mut cursor)?) as usize;
            let name = std::str::from_utf8(cursor.get(..name_len)?).ok()?;
            if name <= previous {
                return None;
            }
            previous = name;
            cursor = &cursor[name_len..];
            let offset = u32::from_le_bytes(take(&mut cursor)?) as usize;
            let length = u32::from_le_bytes(take(&mut cursor)?) as usize;
            let mut sequence = String::new();
            for part in name.split('-') {
                if part.is_empty()
                    || !part
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                {
                    return None;
                }
                sequence.push(char::from_u32(u32::from_str_radix(part, 16).ok()?)?);
            }
            unqualified
                .entry(sequence.replace('\u{FE0F}', ""))
                .or_insert(index);
            if presented(&sequence.chars().collect::<Vec<_>>()) {
                display.insert(sequence.clone(), index);
            }
            let mut qualified = String::new();
            let mut chars = sequence.chars().peekable();
            while let Some(c) = chars.next() {
                qualified.push(c);
                if text_default(c) && chars.peek() != Some(&'\u{FE0F}') {
                    qualified.push('\u{FE0F}');
                }
            }
            display.insert(qualified, index);
            if exact.insert(sequence, index).is_some() {
                return None;
            }
            ranges.push((offset, length));
        }
        let mut entries = Vec::with_capacity(count);
        for (offset, length) in ranges {
            entries.push(Entry {
                compressed: cursor.get(offset..offset.checked_add(length)?)?,
                images: Default::default(),
            });
        }
        Some(Self {
            exact,
            unqualified,
            display,
            entries,
        })
    }

    fn entry(&self, sequence: &str) -> Option<&Entry<'a>> {
        // Twemoji preserves FE0F in ZWJ filenames: https://github.com/jdecked/twemoji#utilities
        let index = self.exact.get(sequence).or_else(|| {
            if sequence.contains('\u{FE0F}') {
                self.unqualified.get(&sequence.replace('\u{FE0F}', ""))
            } else {
                self.unqualified.get(sequence)
            }
        })?;
        self.entries.get(*index)
    }
}

fn bundle() -> Option<&'static Bundle<'static>> {
    static BUNDLE_INDEX: OnceLock<Option<Bundle<'static>>> = OnceLock::new();
    BUNDLE_INDEX.get_or_init(|| Bundle::parse(BUNDLE)).as_ref()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment<'a> {
    Text(&'a str),
    Emoji(&'a str),
}

pub fn segments(text: &str) -> Vec<Segment<'_>> {
    segment_with_bundle(text, bundle())
}

fn segment_with_bundle<'a>(text: &'a str, bundle: Option<&Bundle<'_>>) -> Vec<Segment<'a>> {
    if text.is_empty() {
        return Vec::new();
    }
    // ASCII has no emoji bases; keycaps also need non-ASCII selectors or combining marks.
    if text.is_ascii() {
        return vec![Segment::Text(text)];
    }
    let Some(bundle) = bundle else {
        return vec![Segment::Text(text)];
    };
    let mut chars = Vec::with_capacity(text.len());
    let mut offsets = Vec::with_capacity(text.len() + 1);
    for (offset, c) in text.char_indices() {
        offsets.push(offset);
        chars.push(c);
    }
    offsets.push(text.len());
    let mut output = Vec::new();
    let mut text_start = 0;
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        // Skip non-emoji bases and bare text defaults: https://unicode.org/reports/tr51/#Emoji_Properties
        let skip = if c.is_ascii() {
            !matches!(c, '0'..='9' | '#' | '*')
                || !matches!(chars.get(at + 1), Some('\u{FE0F}' | '\u{20E3}'))
        } else {
            (c < '\u{2300}' && chars.get(at + 1) != Some(&'\u{FE0F}'))
                || matches!(c as u32, 0x3300..=0x1EFFF | 0x1FB00..)
        };
        if skip {
            at += 1;
            continue;
        }
        let end = match crate::text::markdown::emoji_sequence(&chars, at, |c| {
            crate::text::markdown::pictographic(c)
                || matches!(c as u32, 0x24C2 | 0x25AA..=0x25AB | 0x25B6 | 0x25C0 | 0x25FB..=0x25FE | 0x2934..=0x2935)
        }) {
            Ok(end) => end,
            Err(next) => {
                at = next;
                continue;
            }
        };
        let sequence = &text[offsets[at]..offsets[end]];
        let entry = bundle
            .display
            .get(sequence)
            .and_then(|&index| bundle.entries.get(index))
            .or_else(|| {
                (sequence.contains('\u{FE0F}') && presented(&chars[at..end]))
                    .then(|| bundle.entry(sequence))
                    .flatten()
            });
        if entry.and_then(|entry| entry.image(0)).is_some() {
            if text_start < offsets[at] {
                output.push(Segment::Text(&text[text_start..offsets[at]]));
            }
            output.push(Segment::Emoji(sequence));
            text_start = offsets[end];
        }
        at = end;
    }
    if text_start < text.len() {
        output.push(Segment::Text(&text[text_start..]));
    }
    output
}

/// The emoji's image, rasterized to suit `px` physical pixels on screen.
pub fn image(sequence: &str, px: u32) -> Option<Arc<Image>> {
    bundle()?.entry(sequence)?.image(raster_index(px)).cloned()
}

pub struct EmojiCategory {
    pub name: &'static str,
    pub emojis: Vec<(&'static str, &'static str)>,
}

struct ShortcodeTable {
    by_name: HashMap<&'static str, &'static str>,
    entries: Vec<(&'static str, &'static str)>,
    by_emoji: HashMap<String, &'static str>,
}

fn shortcode_table() -> &'static ShortcodeTable {
    static TABLE: OnceLock<ShortcodeTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        let entries: Vec<_> = SHORTCODES
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .collect();
        let mut by_name = HashMap::with_capacity(entries.len());
        let mut by_emoji = HashMap::with_capacity(entries.len());
        for &(name, emoji) in &entries {
            by_name.insert(name, emoji);
            by_emoji
                .entry(emoji.replace('\u{FE0F}', ""))
                .or_insert(name);
        }
        ShortcodeTable {
            by_name,
            entries,
            by_emoji,
        }
    })
}

pub fn shortcode(name: &str) -> Option<&'static str> {
    shortcode_table().by_name.get(name).copied()
}

pub fn shortcodes() -> &'static [(&'static str, &'static str)] {
    &shortcode_table().entries
}

pub fn shortcode_for_emoji(emoji: &str) -> Option<&'static str> {
    shortcode_table()
        .by_emoji
        .get(&emoji.replace('\u{FE0F}', ""))
        .copied()
}

pub fn categories() -> &'static [EmojiCategory] {
    static CATEGORIES: OnceLock<Vec<EmojiCategory>> = OnceLock::new();
    const ORDER: [&str; 8] = [
        "People",
        "Nature",
        "Food",
        "Activities",
        "Travel",
        "Objects",
        "Symbols",
        "Flags",
    ];
    CATEGORIES.get_or_init(|| {
        let mut categories: Vec<_> = ORDER
            .iter()
            .map(|&name| EmojiCategory {
                name,
                emojis: Vec::new(),
            })
            .collect();
        for line in CATEGORY_DATA.lines() {
            let Some((category, emoji)) = line.split_once('\t') else {
                continue;
            };
            let Some(category) = categories
                .iter_mut()
                .find(|candidate| candidate.name == category)
            else {
                continue;
            };
            if bundle().is_none_or(|bundle| bundle.entry(emoji).is_none()) {
                continue;
            }
            if let Some(shortcode) = shortcode_for_emoji(emoji) {
                category.emojis.push((emoji, shortcode));
            }
        }
        categories
    })
}

fn is_emoji_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

fn url_end(chars: &[char], start: usize) -> Option<usize> {
    if !chars[start].is_ascii_alphabetic() {
        return None;
    }
    let boundary = start == 0
        || chars[start - 1].is_whitespace()
        || matches!(chars[start - 1], '(' | '[' | '{' | '"' | '\'');
    if !boundary {
        return None;
    }
    let mut at = start + 1;
    while at < chars.len()
        && (chars[at].is_ascii_alphanumeric() || matches!(chars[at], '+' | '-' | '.'))
    {
        at += 1;
    }
    if at + 2 >= chars.len() || chars[at] != ':' || chars[at + 1] != '/' || chars[at + 2] != '/' {
        return None;
    }
    at += 3;
    while at < chars.len() && !chars[at].is_whitespace() {
        at += 1;
    }
    Some(at)
}

pub fn encode_emoji(content: &str, resolve: impl Fn(&str) -> Option<String>) -> String {
    let chars: Vec<char> = content.chars().collect();
    let mut output = String::with_capacity(content.len());
    let mut inline_code = false;
    let mut fence: Option<usize> = None;
    let mut at = 0;
    while at < chars.len() {
        if chars[at] == '`' {
            let mut end = at + 1;
            while end < chars.len() && chars[end] == '`' {
                end += 1;
            }
            output.extend(&chars[at..end]);
            let run = end - at;
            if run >= 3 {
                match fence {
                    None => fence = Some(run),
                    Some(open) if run >= open => fence = None,
                    Some(_) => {}
                }
            } else if fence.is_none() {
                inline_code = !inline_code;
            }
            at = end;
            continue;
        }
        if inline_code || fence.is_some() {
            output.push(chars[at]);
            at += 1;
            continue;
        }
        if chars[at] == '<'
            && let Some(end) = chars[at + 1..].iter().position(|&c| c == '>')
        {
            let end = at + 1 + end;
            output.extend(&chars[at..=end]);
            at = end + 1;
            continue;
        }
        if let Some(end) = url_end(&chars, at) {
            output.extend(&chars[at..end]);
            at = end;
            continue;
        }
        if chars[at] == ':' {
            let mut end = at + 1;
            while end < chars.len() && is_emoji_name_char(chars[end]) {
                end += 1;
            }
            if end > at + 1 && chars.get(end) == Some(&':') {
                let name: String = chars[at + 1..end].iter().collect();
                if let Some(value) = resolve(&name) {
                    output.push_str(&value);
                    at = end + 1;
                    continue;
                }
            }
        }
        output.push(chars[at]);
        at += 1;
    }
    output
}

fn presented(chars: &[char]) -> bool {
    !chars.contains(&'\u{FE0E}')
        && chars.iter().enumerate().all(|(at, &c)| {
            !text_default(c)
                || chars.get(at + 1) == Some(&'\u{FE0F}')
                || (matches!(c, '0'..='9' | '#' | '*') && chars.get(at + 1) == Some(&'\u{20E3}'))
        })
}

fn text_default(c: char) -> bool {
    // Emoji_Presentation=No requires VS16: https://unicode.org/reports/tr51/#Presentation_Style
    matches!(c as u32,
        0x23 | 0x2A | 0x30..=0x39 | 0xA9 | 0xAE | 0x203C | 0x2049 | 0x2122 | 0x2139
        | 0x2194..=0x2199 | 0x21A9..=0x21AA | 0x2328 | 0x23CF | 0x23ED..=0x23EF
        | 0x23F1..=0x23F2 | 0x23F8..=0x23FA | 0x24C2 | 0x25AA..=0x25AB | 0x25B6
        | 0x25C0 | 0x25FB..=0x25FC | 0x2600..=0x2604 | 0x260E | 0x2611 | 0x2618
        | 0x261D | 0x2620 | 0x2622..=0x2623 | 0x2626 | 0x262A | 0x262E..=0x262F
        | 0x2638..=0x263A | 0x2640 | 0x2642 | 0x265F..=0x2660 | 0x2663
        | 0x2665..=0x2666 | 0x2668 | 0x267B | 0x267E | 0x2692 | 0x2694..=0x2697
        | 0x2699 | 0x269B..=0x269C | 0x26A0 | 0x26A7 | 0x26B0..=0x26B1 | 0x26C8
        | 0x26CF | 0x26D1 | 0x26D3 | 0x26E9 | 0x26F0..=0x26F1 | 0x26F4
        | 0x26F7..=0x26F9 | 0x2702 | 0x2708..=0x2709 | 0x270C..=0x270D | 0x270F
        | 0x2712 | 0x2714 | 0x2716 | 0x271D | 0x2721 | 0x2733..=0x2734 | 0x2744
        | 0x2747 | 0x2763..=0x2764 | 0x27A1 | 0x2934..=0x2935 | 0x2B05..=0x2B07
        | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x1F170..=0x1F171 | 0x1F17E..=0x1F17F
        | 0x1F202 | 0x1F237 | 0x1F321 | 0x1F324..=0x1F32C | 0x1F336 | 0x1F37D
        | 0x1F396..=0x1F397 | 0x1F399..=0x1F39B | 0x1F39E..=0x1F39F
        | 0x1F3CB..=0x1F3CE | 0x1F3D4..=0x1F3DF | 0x1F3F3 | 0x1F3F5 | 0x1F3F7
        | 0x1F43F | 0x1F441 | 0x1F4FD | 0x1F549..=0x1F54A | 0x1F56F..=0x1F570
        | 0x1F573..=0x1F579 | 0x1F587 | 0x1F58A..=0x1F58D | 0x1F590 | 0x1F5A5
        | 0x1F5A8 | 0x1F5B1..=0x1F5B2 | 0x1F5BC | 0x1F5C2..=0x1F5C4
        | 0x1F5D1..=0x1F5D3 | 0x1F5DC..=0x1F5DE | 0x1F5E1 | 0x1F5E3 | 0x1F5E8
        | 0x1F5EF | 0x1F5F3 | 0x1F5FA | 0x1F6CB | 0x1F6CD..=0x1F6CF
        | 0x1F6E0..=0x1F6E5 | 0x1F6E9 | 0x1F6F0 | 0x1F6F3
    )
}

#[cfg(test)]
mod tests;
