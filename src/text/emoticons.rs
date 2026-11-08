use std::borrow::Cow;

use crate::text::emoji;

pub fn convert_emoticons(text: &str) -> Cow<'_, str> {
    let mut output: Option<String> = None;
    let mut copied = 0;
    let mut at = 0;
    let mut fence = next_fence(text, 0);
    while at < text.len() {
        if let Some((open, end)) = fence
            && at == open
        {
            at = end;
            fence = next_fence(text, at);
            continue;
        }
        let rest = &text[at..];
        let c = rest.chars().next().unwrap();
        if c == '\\' && rest.as_bytes().get(1).is_some_and(u8::is_ascii_punctuation) {
            at += 2;
            if rest.as_bytes()[1] == b'`' {
                while fence.is_none_or(|(open, _)| at < open)
                    && text.as_bytes().get(at) == Some(&b'`')
                {
                    at += 1;
                }
            }
            continue;
        }
        if c == '`' {
            let limit = fence.map_or(text.len(), |(open, _)| open);
            let size = rest.bytes().take_while(|&b| b == b'`').count();
            at = inline_code_end(&text[..limit], at, size).unwrap_or(at + size);
            continue;
        }
        if at == 0
            || text[..at]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            let matched = EMOTICONS
                .iter()
                .filter(|(face, _)| {
                    rest.starts_with(face)
                        && matches!(rest.as_bytes().get(face.len()), None | Some(b' ' | b'\n'))
                })
                .max_by_key(|(face, _)| face.len());
            if let Some(&(face, name)) = matched
                && let Some(value) = emoji::shortcode(name)
            {
                let output = output.get_or_insert_with(|| String::with_capacity(text.len()));
                output.push_str(&text[copied..at]);
                output.push_str(value);
                at += face.len();
                copied = at;
                continue;
            }
        }
        at += c.len_utf8();
    }
    match output {
        Some(mut output) => {
            output.push_str(&text[copied..]);
            Cow::Owned(output)
        }
        None => Cow::Borrowed(text),
    }
}

fn next_fence(text: &str, mut at: usize) -> Option<(usize, usize)> {
    loop {
        let open = at + text[at..].find("```")?;
        let escapes = text.as_bytes()[..open]
            .iter()
            .rev()
            .take_while(|&&b| b == b'\\')
            .count();
        if escapes % 2 == 0 {
            let close = open + 3 + text[open + 3..].find("```")?;
            return Some((open, close + 3));
        }
        at = open + 1;
    }
}

fn inline_code_end(text: &str, at: usize, size: usize) -> Option<usize> {
    if size > 2 {
        return None;
    }
    let mut cursor = at + size;
    while let Some(offset) = text[cursor..].find('`') {
        cursor += offset;
        let run = text[cursor..].bytes().take_while(|&b| b == b'`').count();
        if run == size {
            return Some(cursor + run);
        }
        cursor += run;
    }
    None
}

const EMOTICONS: &[(&str, &str)] = &[
    (",:(", "sweat"),
    (",:)", "sweat_smile"),
    (",:-(", "sweat"),
    (",:-)", "sweat_smile"),
    (",=(", "sweat"),
    (",=)", "sweat_smile"),
    (",=-(", "sweat"),
    (",=-)", "sweat_smile"),
    ("0:)", "innocent"),
    ("0:-)", "innocent"),
    ("0=)", "innocent"),
    ("0=-)", "innocent"),
    ("8-)", "sunglasses"),
    (":\")", "blush"),
    (":$", "unamused"),
    (":'(", "cry"),
    (":')", "smiling_face_with_tear"),
    (":'-(", "cry"),
    (":'-)", "smiling_face_with_tear"),
    (":'-D", "joy"),
    (":'D", "joy"),
    (":(", "slight_frown"),
    (":)", "slight_smile"),
    (":*", "kissing"),
    (":,'(", "sob"),
    (":,'-(", "sob"),
    (":,(", "cry"),
    (":,)", "smiling_face_with_tear"),
    (":,-(", "cry"),
    (":,-)", "smiling_face_with_tear"),
    (":,-D", "joy"),
    (":,D", "joy"),
    (":-\")", "blush"),
    (":-$", "unamused"),
    (":-(", "slight_frown"),
    (":-)", "slight_smile"),
    (":-*", "kissing"),
    (":-/", "confused"),
    (":-@", "rage"),
    (":-D", "smile"),
    (":-O", "open_mouth"),
    (":-P", "stuck_out_tongue"),
    (":-S", "unamused"),
    (":-Z", "unamused"),
    (":-\\", "confused"),
    (":-o", "open_mouth"),
    (":-|", "neutral_face"),
    (":@", "rage"),
    (":D", "smile"),
    (":O", "open_mouth"),
    (":P", "stuck_out_tongue"),
    (":o", "open_mouth"),
    (":s", "unamused"),
    (":z", "unamused"),
    (":|", "neutral_face"),
    (";(", "sob"),
    (";)", "wink"),
    (";-(", "sob"),
    (";-)", "wink"),
    ("</3", "broken_heart"),
    ("<3", "heart"),
    ("<\\3", "broken_heart"),
    ("=\")", "blush"),
    ("=$", "unamused"),
    ("='(", "cry"),
    ("=')", "smiling_face_with_tear"),
    ("='-(", "cry"),
    ("='-)", "smiling_face_with_tear"),
    ("='-D", "joy"),
    ("='D", "joy"),
    ("=(", "slight_frown"),
    ("=)", "slight_smile"),
    ("=*", "kissing"),
    ("=,'(", "sob"),
    ("=,'-(", "sob"),
    ("=,(", "cry"),
    ("=,)", "smiling_face_with_tear"),
    ("=,-(", "cry"),
    ("=,-)", "smiling_face_with_tear"),
    ("=,-D", "joy"),
    ("=,D", "joy"),
    ("=-\")", "blush"),
    ("=-$", "unamused"),
    ("=-(", "slight_frown"),
    ("=-)", "slight_smile"),
    ("=-*", "kissing"),
    ("=-/", "confused"),
    ("=-@", "rage"),
    ("=-D", "smile"),
    ("=-O", "open_mouth"),
    ("=-P", "stuck_out_tongue"),
    ("=-S", "unamused"),
    ("=-Z", "unamused"),
    ("=-\\", "confused"),
    ("=-o", "open_mouth"),
    ("=-|", "neutral_face"),
    ("=@", "rage"),
    ("=D", "smile"),
    ("=O", "open_mouth"),
    ("=P", "stuck_out_tongue"),
    ("=o", "open_mouth"),
    ("=s", "unamused"),
    ("=z", "unamused"),
    ("=|", "neutral_face"),
    (">:(", "angry"),
    (">:-(", "angry"),
    (">=(", "angry"),
    (">=-(", "angry"),
    ("B-)", "sunglasses"),
    ("O:)", "innocent"),
    ("O:-)", "innocent"),
    ("O=)", "innocent"),
    ("O=-)", "innocent"),
    ("X-)", "laughing"),
    ("]:(", "imp"),
    ("]:)", "smiling_imp"),
    ("]:-(", "imp"),
    ("]:-)", "smiling_imp"),
    ("]=(", "imp"),
    ("]=)", "smiling_imp"),
    ("]=-(", "imp"),
    ("]=-)", "smiling_imp"),
    ("o:)", "innocent"),
    ("o:-)", "innocent"),
    ("o=)", "innocent"),
    ("o=-)", "innocent"),
    ("x-)", "laughing"),
    ("\u{2661}", "heart"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_and_whitespace_boundaries() {
        for prefix in ["", " ", "\t", "\n", "\r", "\u{2003}"] {
            assert_eq!(
                convert_emoticons(&format!("{prefix}:)")),
                format!("{prefix}\u{1F642}")
            );
        }
        for text in ["a:)", "(:)", "\u{4E2D}:)", "http://x", "http://x/:)"] {
            assert!(
                matches!(convert_emoticons(text), Cow::Borrowed(_)),
                "{text:?}"
            );
        }
    }

    #[test]
    fn trailing_boundary() {
        for suffix in ["", " ", "\n"] {
            assert_eq!(
                convert_emoticons(&format!(":){suffix}")),
                format!("\u{1F642}{suffix}")
            );
        }
        for text in [":)abc", ":),", ":)\t", ":)\r", ":)\u{2003}"] {
            assert!(
                matches!(convert_emoticons(text), Cow::Borrowed(_)),
                "{text:?}"
            );
        }
    }

    #[test]
    fn longest_match_and_multiple_faces() {
        assert_eq!(
            convert_emoticons(":-) >:( :'-D :) :("),
            "\u{1F642} \u{1F620} \u{1F602} \u{1F642} \u{1F641}"
        );
    }

    #[test]
    fn newlines_and_unicode_are_preserved() {
        assert_eq!(
            convert_emoticons("\u{4E2D}\n:)\n\t:( \u{2661}\n"),
            "\u{4E2D}\n\u{1F642}\n\t\u{1F641} \u{2764}\u{FE0F}\n"
        );
    }

    #[test]
    fn inline_code_is_untouched() {
        assert_eq!(
            convert_emoticons("` :) ` :) `` :( ` :) `` :("),
            "` :) ` \u{1F642} `` :( ` :) `` \u{1F641}"
        );
        assert!(matches!(convert_emoticons("` :)\n:( `"), Cow::Borrowed(_)));
    }

    #[test]
    fn fenced_code_is_untouched() {
        assert_eq!(
            convert_emoticons(":) ```rust\n:) ` :(\n```\n:("),
            "\u{1F642} ```rust\n:) ` :(\n```\n\u{1F641}"
        );
        assert!(matches!(
            convert_emoticons("``` :) ``` ``` :( ```"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn escaped_and_unclosed_delimiters() {
        assert_eq!(
            convert_emoticons(r"\```` :) ``` :("),
            "\\```` :) ``` \u{1F641}"
        );
        assert_eq!(convert_emoticons(r"\``` :) ``"), "\\``` \u{1F642} ``");
        assert_eq!(convert_emoticons(r"\` :) \`"), "\\` \u{1F642} \\`");
        assert_eq!(convert_emoticons("` :)"), "` \u{1F642}");
        assert_eq!(convert_emoticons("``` :)"), "``` \u{1F642}");
        assert_eq!(convert_emoticons(r"` :) \` :)"), "` :) \\` \u{1F642}");
    }

    #[test]
    fn no_op_is_borrowed() {
        for text in ["", "hello \u{4E2D}", "http://x", "` :) `", "``` :) ```"] {
            assert!(
                matches!(convert_emoticons(text), Cow::Borrowed(_)),
                "{text:?}"
            );
        }
    }

    #[test]
    fn every_table_entry_resolves_and_converts() {
        assert_eq!(EMOTICONS.len(), 128);
        for &(face, name) in EMOTICONS {
            let value =
                emoji::shortcode(name).unwrap_or_else(|| panic!("missing shortcode: {name}"));
            assert_eq!(convert_emoticons(face), value, "{face:?} -> {name}");
        }
    }
}
