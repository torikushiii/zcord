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

struct Entry<'a> {
    compressed: &'a [u8],
    image: OnceLock<Option<Arc<Image>>>,
}

impl Entry<'_> {
    fn image(&self) -> Option<&Arc<Image>> {
        self.image
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
                image: OnceLock::new(),
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
        if entry.and_then(Entry::image).is_some() {
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

pub fn image(sequence: &str) -> Option<Arc<Image>> {
    bundle()?.entry(sequence)?.image().cloned()
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
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn round_trip(value: &str) {
        let pieces = segments(value);
        let mut restored = String::new();
        let mut previous_text = false;
        for piece in pieces {
            let slice = match piece {
                Segment::Text(slice) => {
                    assert!(!previous_text, "adjacent text: {value:?}");
                    previous_text = true;
                    slice
                }
                Segment::Emoji(slice) => {
                    assert!(image(slice).is_some(), "unbundled emoji: {slice:?}");
                    previous_text = false;
                    slice
                }
            };
            assert!(!slice.is_empty());
            assert_eq!(slice.as_ptr(), value[restored.len()..].as_ptr());
            restored.push_str(slice);
        }
        assert_eq!(restored.as_bytes(), value.as_bytes());
    }

    #[test]
    fn emoji_channel_name() {
        assert_eq!(
            segments("\u{1F3E0}\u{250A}announcements"),
            vec![
                Segment::Emoji("\u{1F3E0}"),
                Segment::Text("\u{250A}announcements")
            ]
        );
    }

    #[test]
    fn emoji_family_skin_tone_flag_and_keycaps() {
        for value in [
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            "\u{1F44D}\u{1F3FD}",
            "\u{1F1F7}\u{1F1FA}",
            "1\u{FE0F}\u{20E3}",
            "1\u{20E3}",
            "#\u{20E3}",
            "*\u{FE0F}\u{20E3}",
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}",
        ] {
            assert_eq!(segments(value), vec![Segment::Emoji(value)], "{value:?}");
        }
    }

    #[test]
    fn ascii_text_is_never_emoji() {
        for value in ["#1 * 0123456789", "hello # world * 42", "#*0123 general"] {
            assert_eq!(segments(value), vec![Segment::Text(value)], "{value:?}");
        }
    }

    #[test]
    fn emoji_text_defaults_and_selectors() {
        for value in [
            "#1 \u{00A9} 2026\u{2122}",
            "0123456789 # * \u{00A9} \u{00AE} \u{2122} \u{2764} \u{2197}",
            "\u{00A9}\u{FE0E}",
            "\u{1F3E0}\u{FE0E}",
            "1\u{FE0F} #\u{FE0F} *\u{FE0F}",
        ] {
            assert_eq!(segments(value), vec![Segment::Text(value)], "{value:?}");
        }
        for value in [
            "\u{00A9}\u{FE0F}",
            "\u{00AE}\u{FE0F}",
            "\u{2122}\u{FE0F}",
            "\u{2764}\u{FE0F}",
            "\u{2197}\u{FE0F}",
            "\u{24C2}\u{FE0F}",
            "\u{25B6}\u{FE0F}",
            "\u{2934}\u{FE0F}",
            "\u{1F441}\u{FE0F}",
        ] {
            assert_eq!(segments(value), vec![Segment::Emoji(value)], "{value:?}");
        }
        assert!(segments("").is_empty());
    }

    #[test]
    fn emoji_image_cache_and_fe0f_lookup() {
        let first = image("\u{1F3E0}").unwrap();
        assert!(Arc::ptr_eq(&first, &image("\u{1F3E0}").unwrap()));
        assert_eq!(first.format, ImageFormat::Svg);
        assert!(
            first
                .bytes
                .starts_with(b"<svg width=\"128\" height=\"128\"")
        );
        let heart = image("\u{2764}\u{FE0F}").unwrap();
        assert!(Arc::ptr_eq(&heart, &image("\u{2764}").unwrap()));
        let zwj = "\u{1F469}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F469}";
        let unqualified = zwj.replace('\u{FE0F}', "");
        assert!(Arc::ptr_eq(
            &image(zwj).unwrap(),
            &image(&unqualified).unwrap()
        ));
        for value in ["", "hello", "1", "#", "\u{200D}", "\u{FE0F}"] {
            assert!(image(value).is_none(), "{value:?}");
        }
        for value in ["\u{1F1F7}", "\u{1F3FD}", "\u{E50A}"] {
            assert!(image(value).is_some(), "{value:?}");
            assert_eq!(segments(value), vec![Segment::Text(value)]);
        }
    }

    #[test]
    fn emoji_unsupported_sequences_and_lone_components() {
        for value in [
            "\u{1F3E0}\u{200D}\u{1F3E0}",
            "\u{1F3E0}\u{1F3FD}",
            "\u{1F1E6}\u{1F1E6}",
            "\u{200D}\u{FE0F}\u{1F3FD}\u{1F1F7}",
        ] {
            assert_eq!(segments(value), vec![Segment::Text(value)], "{value:?}");
        }
        for value in [
            "\u{1F3E0}\u{200D}",
            "\u{1F600}\u{200D}x",
            "x\u{FE0F}\u{1F3E0}",
        ] {
            round_trip(value);
        }
        let dangling = "\u{1F600}\u{200D}".repeat(2000);
        assert_eq!(segments(&dangling), vec![Segment::Text(&dangling)]);
    }

    #[test]
    fn emoji_round_trip_fuzz_mixed_unicode() {
        let alphabet = [
            'a',
            ' ',
            '\n',
            '\0',
            '#',
            '*',
            '1',
            '\u{4E2D}',
            '\u{6587}',
            '\u{1F3E0}',
            '\u{1F600}',
            '\u{1F468}',
            '\u{1F469}',
            '\u{2764}',
            '\u{00A9}',
            '\u{2122}',
            '\u{200D}',
            '\u{FE0F}',
            '\u{FE0E}',
            '\u{1F3FB}',
            '\u{1F3FD}',
            '\u{1F1F7}',
            '\u{1F1FA}',
            '\u{20E3}',
            '\u{E0067}',
            '\u{E007F}',
            '\u{10FFFF}',
        ];
        let mut state = 0x1234_5678_u64;
        for length in 0..512 {
            let mut value = String::new();
            for _ in 0..length {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                value.push(alphabet[(state >> 32) as usize % alphabet.len()]);
            }
            round_trip(&value);
        }
        for &left in &alphabet {
            for &right in &alphabet {
                round_trip(&[left, right].iter().collect::<String>());
            }
        }
    }

    #[test]
    fn emoji_bundle_all_entries_are_images_and_segmentable() {
        let bundle = bundle().unwrap();
        assert!(bundle.entries.len() >= 3700);
        for sequence in bundle.exact.keys() {
            assert!(image(sequence).is_some(), "{sequence:?}");
            if sequence
                .chars()
                .all(|c| matches!(c as u32, 0x1F1E6..=0x1F1FF | 0x1F3FB..=0x1F3FF | 0xE50A))
                && sequence.chars().count() == 1
            {
                continue;
            }
            let mut qualified = String::new();
            let mut chars = sequence.chars().peekable();
            while let Some(c) = chars.next() {
                qualified.push(c);
                if text_default(c) && chars.peek() != Some(&'\u{FE0F}') {
                    qualified.push('\u{FE0F}');
                }
            }
            assert_eq!(
                segments(&qualified),
                vec![Segment::Emoji(&qualified)],
                "{sequence:?}"
            );
        }
    }

    #[test]
    fn emoji_corrupt_bundle_is_safe_and_all_text() {
        for bytes in [b"".as_slice(), b"TWE2", b"TWE1\xFF\xFF\xFF\xFF"] {
            assert!(Bundle::parse(bytes).is_none());
        }
        for length in 0..128 {
            assert!(Bundle::parse(&BUNDLE[..length]).is_none());
        }
        let mut invalid = BUNDLE.to_vec();
        let first_name_len = u16::from_le_bytes([invalid[8], invalid[9]]) as usize;
        invalid[10 + first_name_len..14 + first_name_len].fill(0xFF);
        assert!(Bundle::parse(&invalid).is_none());
        invalid = BUNDLE.to_vec();
        let compressed = bundle().unwrap().entry("\u{1F3E0}").unwrap().compressed;
        let offset = compressed.as_ptr() as usize - BUNDLE.as_ptr() as usize;
        invalid[offset] ^= 0xFF;
        let corrupt = Bundle::parse(&invalid).unwrap();
        assert!(corrupt.entry("\u{1F3E0}").unwrap().image().is_none());
        for data in [None, Some(&corrupt)] {
            let value = "before \u{1F3E0} after";
            assert_eq!(segment_with_bundle(value, data), vec![Segment::Text(value)]);
        }
    }

    #[test]
    // Timing budgets assume an optimized build; run with `cargo test --release`.
    #[cfg_attr(debug_assertions, ignore)]
    fn emoji_performance_4000_char_messages_1000_times() {
        let pattern = "hello \u{4E2D}\u{6587} \u{1F3E0} \u{1F44D}\u{1F3FD} \u{1F1F7}\u{1F1FA} \u{2764}\u{FE0F} 1\u{FE0F}\u{20E3} #1 \u{00A9} 2026\u{2122} ";
        let value: String = pattern.chars().cycle().take(4000).collect();
        round_trip(&value);
        let started = Instant::now();
        for _ in 0..1000 {
            std::hint::black_box(segments(std::hint::black_box(&value)));
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(750),
            "1000 segmentations took {elapsed:?}"
        );
        println!("1000 segmentations of 4000 mixed characters took {elapsed:?}");
    }

    #[test]
    fn emoji_api_categories_are_ordered_tone_free_and_shortcoded() {
        let categories = categories();
        assert_eq!(
            categories.first().map(|category| category.name),
            Some("People")
        );
        assert!(
            categories
                .iter()
                .all(|category| !category.emojis.is_empty())
        );
        for category in categories {
            for &(emoji, shortcode) in &category.emojis {
                assert!(!shortcode.is_empty());
                assert!(super::shortcode(shortcode).is_some(), "{shortcode}");
                assert_eq!(super::shortcode_for_emoji(emoji), Some(shortcode));
                assert!(
                    emoji
                        .chars()
                        .all(|c| !(0x1F3FB..=0x1F3FF).contains(&(c as u32))),
                    "{emoji:?}"
                );
            }
        }
        assert!(categories.iter().any(|category| category.name == "Flags"));
    }

    #[test]
    fn emoji_api_encode_emoji_skips_locked_code_and_tokens() {
        let resolve = |name: &str| match name {
            "wave" => Some("<:wave:123>".to_owned()),
            "smile" => Some("\u{1F604}".to_owned()),
            _ => None,
        };
        assert_eq!(
            encode_emoji("hi :wave: :smile:", &resolve),
            "hi <:wave:123> \u{1F604}"
        );
        assert_eq!(encode_emoji(":locked:", &resolve), ":locked:");
        assert_eq!(encode_emoji("` :wave: `", &resolve), "` :wave: `");
        assert_eq!(
            encode_emoji("```\n:wave:\n```", &resolve),
            "```\n:wave:\n```"
        );
        assert_eq!(encode_emoji("<:a:1>", &resolve), "<:a:1>");
        assert_eq!(
            encode_emoji("https://example.test/:wave:", &resolve),
            "https://example.test/:wave:"
        );
    }
}
