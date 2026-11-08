use std::{cell::RefCell, collections::HashMap, ops::Range};

const EM_SPACE: char = '\u{2003}';
const SELECTOR_FIRST: u32 = 0xE0100;
const SELECTOR_LAST: u32 = 0xE01EF;
const BASE: u32 = 240;
const CAPACITY: usize = (BASE * BASE) as usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmojiToken {
    pub id: u64,
    pub name: String,
    pub animated: bool,
}

impl EmojiToken {
    pub fn wire(&self) -> String {
        if self.animated {
            format!("<a:{}:{}>", self.name, self.id)
        } else {
            format!("<:{}:{}>", self.name, self.id)
        }
    }
}

#[derive(Default)]
struct Registry {
    entries: Vec<EmojiToken>,
    indices: HashMap<u64, usize>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

pub fn is_token_char(c: char) -> bool {
    c == EM_SPACE || selector_value(c).is_some()
}

fn selector_value(c: char) -> Option<u32> {
    let value = c as u32;
    (SELECTOR_FIRST..=SELECTOR_LAST)
        .contains(&value)
        .then(|| value - SELECTOR_FIRST)
}

fn encode_index(index: usize) -> Option<String> {
    if index >= CAPACITY {
        return None;
    }
    let high = (index as u32) / BASE;
    let low = (index as u32) % BASE;
    let mut token = String::with_capacity(EM_SPACE.len_utf8() + 8);
    token.push(EM_SPACE);
    token.push(char::from_u32(SELECTOR_FIRST + high)?);
    token.push(char::from_u32(SELECTOR_FIRST + low)?);
    Some(token)
}

pub fn token_for(id: u64, name: &str, animated: bool) -> String {
    REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        let index = match registry.indices.get(&id) {
            Some(index) => *index,
            None => {
                if registry.entries.len() >= CAPACITY {
                    return format!(":{name}:");
                }
                let index = registry.entries.len();
                registry.entries.push(EmojiToken {
                    id,
                    name: name.to_owned(),
                    animated,
                });
                registry.indices.insert(id, index);
                index
            }
        };
        encode_index(index).unwrap_or_else(|| format!(":{name}:"))
    })
}

pub fn tokens_in(text: &str) -> Vec<(Range<usize>, EmojiToken)> {
    let chars: Vec<_> = text.char_indices().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at + 2 < chars.len() {
        if chars[at].1 == EM_SPACE
            && let Some(high) = selector_value(chars[at + 1].1)
            && let Some(low) = selector_value(chars[at + 2].1)
        {
            let index = (high * BASE + low) as usize;
            let token = REGISTRY.with(|registry| registry.borrow().entries.get(index).cloned());
            if let Some(token) = token {
                let start = chars[at].0;
                let end = chars.get(at + 3).map_or(text.len(), |(offset, _)| *offset);
                tokens.push((start..end, token));
                at += 3;
                continue;
            }
        }
        at += 1;
    }
    tokens
}

pub fn repair_with_removed(text: &str) -> Option<(String, Vec<Range<usize>>)> {
    let chars: Vec<_> = text.char_indices().collect();
    let mut output = String::with_capacity(text.len());
    let mut removed = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let (start, c) = chars[at];
        if c == EM_SPACE {
            let mut selectors = 0;
            while at + 1 + selectors < chars.len()
                && selector_value(chars[at + 1 + selectors].1).is_some()
            {
                selectors += 1;
            }
            if selectors == 0 {
                output.push(c);
                at += 1;
                continue;
            }
            if selectors == 1 {
                let end = chars.get(at + 2).map_or(text.len(), |(offset, _)| *offset);
                removed.push(start..end);
                at += 2;
                continue;
            }
            let end = chars.get(at + 3).map_or(text.len(), |(offset, _)| *offset);
            output.push_str(&text[start..end]);
            at += 3;
            continue;
        }
        if selector_value(c).is_some() {
            let end = chars.get(at + 1).map_or(text.len(), |(offset, _)| *offset);
            removed.push(start..end);
            at += 1;
            continue;
        }
        output.push(c);
        at += 1;
    }
    (!removed.is_empty()).then_some((output, removed))
}

pub fn repair(text: &str) -> Option<String> {
    repair_with_removed(text).map(|(text, _)| text)
}

pub fn to_wire(text: &str) -> String {
    let tokens = tokens_in(text);
    if tokens.is_empty() {
        return text.to_owned();
    }
    let mut output = String::with_capacity(text.len());
    let mut at = 0;
    for (range, token) in tokens {
        output.push_str(&text[at..range.start]);
        output.push_str(&token.wire());
        at = range.end;
    }
    output.push_str(&text[at..]);
    output
}

fn wire_emoji(inner: &str) -> Option<(bool, &str, u64)> {
    let (animated, rest) = if let Some(rest) = inner.strip_prefix("a:") {
        (true, rest)
    } else {
        (false, inner.strip_prefix(':')?)
    };
    let (name, id) = rest.rsplit_once(':')?;
    if name.is_empty()
        || id.is_empty()
        || !id.bytes().all(|byte| byte.is_ascii_digit())
        || name.contains([':', '<', '>'])
    {
        return None;
    }
    Some((animated, name, id.parse().ok()?))
}

pub fn from_wire(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(relative) = text[at..].find('<') {
        let start = at + relative;
        let Some(relative_end) = text[start..].find('>') else {
            break;
        };
        let end = start + relative_end;
        output.push_str(&text[at..start]);
        if let Some((animated, name, id)) = wire_emoji(&text[start + 1..end]) {
            output.push_str(&token_for(id, name, animated));
        } else {
            output.push_str(&text[start..=end]);
        }
        at = end + 1;
    }
    output.push_str(&text[at..]);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use gpui_component::input::TextareaState;

    fn reset_registry() {
        REGISTRY.with(|registry| *registry.borrow_mut() = Registry::default());
    }

    #[test]
    fn tokens_round_trip_at_base_240_boundaries() {
        reset_registry();
        for (index, expected) in [
            (0, "\u{2003}\u{E0100}\u{E0100}"),
            (239, "\u{2003}\u{E0100}\u{E01EF}"),
            (240, "\u{2003}\u{E0101}\u{E0100}"),
            (57599, "\u{2003}\u{E01EF}\u{E01EF}"),
        ] {
            assert_eq!(encode_index(index).as_deref(), Some(expected));
        }
        assert!(encode_index(57600).is_none());

        let first = token_for(1, "first", false);
        let animated = token_for(2, "party", true);
        assert_eq!(token_for(1, "first", false), first);
        let text = format!("a {first} b {animated} \u{1F600}");
        let tokens = tokens_in(&text);
        assert_eq!(tokens.len(), 2);
        assert_eq!(tokens[0].1.name, "first");
        assert_eq!(tokens[1].1.name, "party");
        assert_eq!(to_wire(&text), "a <:first:1> b <a:party:2> \u{1F600}");
        assert_eq!(tokens_in(&to_wire(&text)).len(), 0);
    }

    #[test]
    fn every_registry_index_is_reachable() {
        reset_registry();
        for id in 0..CAPACITY as u64 {
            token_for(id, "bulk", false);
        }
        for index in [0, 239, 240, 57599] {
            assert_eq!(
                token_for(index as u64, "unused", false),
                encode_index(index).unwrap()
            );
        }
        assert_eq!(token_for(CAPACITY as u64, "overflow", false), ":overflow:");
    }

    #[test]
    fn repair_removes_broken_remnants_only() {
        reset_registry();
        assert_eq!(repair("plain \u{2003} space"), None);
        let broken = "\u{2003}\u{E0100}";
        assert_eq!(repair(broken).as_deref(), Some(""));
        assert_eq!(repair(&format!("a{broken}b")).as_deref(), Some("ab"));
        assert_eq!(repair("\u{E0100}stray\u{E01EF}").as_deref(), Some("stray"));
        let token = token_for(7, "ok", false);
        assert_eq!(repair(&token), None);
        let extra = format!("{token}\u{E0100}");
        assert_eq!(repair(&extra).as_deref(), Some(token.as_str()));
        assert_eq!(
            repair(&format!("a{token}\u{E0100}b")).as_deref(),
            Some(format!("a{token}b").as_str())
        );
    }

    #[test]
    fn wire_conversion_mixes_custom_and_unicode_emoji() {
        reset_registry();
        let static_token = token_for(10, "cat", false);
        let animated_token = token_for(11, "dance", true);
        let text = format!("hi {static_token} and {animated_token}! \u{1F44D}\u{1F3FD}");
        assert_eq!(
            to_wire(&text),
            "hi <:cat:10> and <a:dance:11>! \u{1F44D}\u{1F3FD}"
        );
        assert_eq!(
            from_wire("hi <:cat:10> and <a:dance:11>!"),
            format!("hi {static_token} and {animated_token}!")
        );
        assert_eq!(from_wire("x <:bad> y"), "x <:bad> y");
    }

    #[gpui::test]
    fn textarea_moves_and_deletes_inside_tokens(cx: &mut TestAppContext) {
        reset_registry();
        cx.update(gpui_component::init);
        let token = token_for(900, "test", false);
        let text = format!("a{token}b");
        let before = 1;
        let after = before + token.len();
        let (input, cx) = cx.add_window_view(move |window, cx| {
            let mut input = TextareaState::new(window, cx).auto_grow(1, 15);
            input.set_value(text.clone(), window, cx);
            input.set_selected_range(after..after, cx);
            input.focus(window, cx);
            input
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        cx.simulate_keystrokes("left");
        let left = input.update(cx, |input, _| input.cursor());
        println!("left from {after} -> {left}, token is {before}..{after}");
        assert!(
            before < left && left < after,
            "one left stops inside the token at {left}"
        );

        input.update(cx, |input, cx| input.set_selected_range(before..before, cx));
        cx.simulate_keystrokes("right");
        let right = input.update(cx, |input, _| input.cursor());
        println!("right from {before} -> {right}, token is {before}..{after}");
        assert!(
            before < right && right < after,
            "one right stops inside the token at {right}"
        );

        input.update(cx, |input, cx| input.set_selected_range(after..after, cx));
        cx.simulate_keystrokes("backspace");
        let (remaining, cursor) =
            input.update(cx, |input, _| (input.value().to_string(), input.cursor()));
        println!("backspace from {after} -> cursor {cursor}, text {remaining:?}");
        assert_ne!(remaining, format!("a{token}b").replace(&token, ""));
        assert_eq!(repair(&remaining).as_deref(), Some("ab"));
    }
}
