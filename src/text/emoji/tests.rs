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
                assert!(image(slice, 44).is_some(), "unbundled emoji: {slice:?}");
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
    let first = image("\u{1F3E0}", 44).unwrap();
    assert!(Arc::ptr_eq(&first, &image("\u{1F3E0}", 44).unwrap()));
    assert_eq!(first.format, ImageFormat::Svg);
    assert!(first.bytes.starts_with(b"<svg width=\"24\" height=\"24\""));
    let large = image("\u{1F3E0}", 1000).unwrap();
    assert!(
        large
            .bytes
            .starts_with(b"<svg width=\"128\" height=\"128\"")
    );
    let heart = image("\u{2764}\u{FE0F}", 44).unwrap();
    assert!(Arc::ptr_eq(&heart, &image("\u{2764}", 44).unwrap()));
    let zwj = "\u{1F469}\u{200D}\u{2764}\u{FE0F}\u{200D}\u{1F469}";
    let unqualified = zwj.replace('\u{FE0F}', "");
    assert!(Arc::ptr_eq(
        &image(zwj, 44).unwrap(),
        &image(&unqualified, 44).unwrap()
    ));
    for value in ["", "hello", "1", "#", "\u{200D}", "\u{FE0F}"] {
        assert!(image(value, 44).is_none(), "{value:?}");
    }
    for value in ["\u{1F1F7}", "\u{1F3FD}", "\u{E50A}"] {
        assert!(image(value, 44).is_some(), "{value:?}");
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
        assert!(image(sequence, 44).is_some(), "{sequence:?}");
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
    assert!(corrupt.entry("\u{1F3E0}").unwrap().image(0).is_none());
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
        encode_emoji("hi :wave: :smile:", resolve),
        "hi <:wave:123> \u{1F604}"
    );
    assert_eq!(encode_emoji(":locked:", resolve), ":locked:");
    assert_eq!(encode_emoji("` :wave: `", resolve), "` :wave: `");
    assert_eq!(
        encode_emoji("```\n:wave:\n```", resolve),
        "```\n:wave:\n```"
    );
    assert_eq!(encode_emoji("<:a:1>", resolve), "<:a:1>");
    assert_eq!(
        encode_emoji("https://example.test/:wave:", resolve),
        "https://example.test/:wave:"
    );
}
