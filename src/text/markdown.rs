use crate::text::emoji;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Span>),
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Subtext(Vec<Span>),
    Quote(Vec<Block>),
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub spoiler: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub kind: SpanKind,
    pub style: Style,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanKind {
    Text(String),
    Code(String),
    UserMention(u64),
    ChannelMention(u64),
    MessageLink {
        channel_id: u64,
        message_id: u64,
    },
    RoleMention(u64),
    Everyone,
    Here,
    Emoji {
        id: u64,
        name: String,
        animated: bool,
    },
    Timestamp {
        unix: i64,
        style: char,
    },
    Command {
        name: String,
        id: u64,
    },
}

pub fn parse(content: &str) -> Vec<Block> {
    Parser::default().blocks(content.trim_matches('\n'), true, true)
}

pub fn is_jumbo(blocks: &[Block]) -> bool {
    let [Block::Paragraph(spans)] = blocks else {
        return false;
    };
    let mut count = 0;
    for span in spans {
        match &span.kind {
            SpanKind::Emoji { .. } => count += 1,
            SpanKind::Text(text) => {
                let chars: Vec<char> = text.chars().collect();
                let mut at = 0;
                while at < chars.len() {
                    if chars[at].is_whitespace() {
                        at += 1;
                    } else {
                        let Ok(end) = emoji_sequence(&chars, at, pictographic) else {
                            return false;
                        };
                        at = end;
                        count += 1;
                    }
                }
            }
            _ => return false,
        }
        if count > 30 {
            return false;
        }
    }
    count > 0
}

#[cfg(test)]
pub fn plain_text(blocks: &[Block]) -> String {
    plain_text_with(blocks, |_| None)
}

/// `resolve` names a user, channel or role mention or renders a timestamp; `None` keeps
/// the raw form.
pub fn plain_text_with(blocks: &[Block], resolve: impl Fn(&SpanKind) -> Option<String>) -> String {
    let mut text = String::new();
    let mut pending: Vec<&Block> = blocks.iter().rev().collect();
    while let Some(block) = pending.pop() {
        text.push(' ');
        match block {
            Block::Paragraph(spans) | Block::Heading { spans, .. } | Block::Subtext(spans) => {
                for span in spans {
                    match &span.kind {
                        SpanKind::Text(value) | SpanKind::Code(value) => text.push_str(value),
                        SpanKind::UserMention(id) => match resolve(&span.kind) {
                            Some(name) => text.push_str(&format!("@{name}")),
                            None => text.push_str(&format!("@{id}")),
                        },
                        SpanKind::ChannelMention(id) => match resolve(&span.kind) {
                            Some(name) => text.push_str(&format!("#{name}")),
                            None => text.push_str(&format!("#{id}")),
                        },
                        SpanKind::MessageLink { channel_id, .. } => {
                            match resolve(&SpanKind::ChannelMention(*channel_id)) {
                                Some(name) => text.push_str(&format!("#{name} › message")),
                                None => text.push_str(&format!("#{channel_id} › message")),
                            }
                        }
                        SpanKind::RoleMention(id) => match resolve(&span.kind) {
                            Some(name) => text.push_str(&format!("@{name}")),
                            None => text.push_str(&format!("@&{id}")),
                        },
                        SpanKind::Everyone => text.push_str("@everyone"),
                        SpanKind::Here => text.push_str("@here"),
                        SpanKind::Emoji { name, .. } => text.push_str(&format!(":{name}:")),
                        SpanKind::Timestamp { unix, style } => match resolve(&span.kind) {
                            Some(time) => text.push_str(&time),
                            None => text.push_str(&format!("<t:{unix}:{style}>")),
                        },
                        SpanKind::Command { name, .. } => text.push_str(&format!("/{name}")),
                    }
                }
            }
            Block::CodeBlock { code, .. } => text.push_str(code),
            Block::Quote(children) => pending.extend(children.iter().rev()),
            Block::List { items, .. } => {
                pending.extend(items.iter().rev().flat_map(|item| item.iter().rev()));
            }
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Default)]
struct Parser {
    next_spoiler: u16,
}

struct ListMarker<'a> {
    indent: usize,
    start: Option<u64>,
    body: &'a str,
}

struct ListFrame {
    indent: usize,
    start: Option<u64>,
    items: Vec<Vec<Block>>,
}

impl Parser {
    fn blocks(&mut self, content: &str, quotes: bool, lists: bool) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut paragraph = String::new();
        let mut at = 0;
        let mut next_fence = fence_pair(content, 0);
        let mut line_end = content.find('\n').unwrap_or(content.len());
        while at < content.len() {
            if next_fence.is_some_and(|(open, _)| open < at) {
                next_fence = fence_pair(content, at);
            }
            if let Some((open, close)) = next_fence {
                if open == at {
                    self.flush_paragraph(&mut blocks, &mut paragraph);
                    let raw = &content[open + 3..close];
                    let (language, code) = match raw.split_once('\n') {
                        Some((token, code)) if language_token(token) => {
                            (Some(token.to_owned()), code)
                        }
                        _ => (None, raw),
                    };
                    blocks.push(Block::CodeBlock {
                        language,
                        code: code.strip_suffix('\n').unwrap_or(code).to_owned(),
                    });
                    at = close + 3;
                    if content[at..].starts_with('\n') {
                        at += 1;
                    }
                    next_fence = fence_pair(content, at);
                    continue;
                }
            }
            if line_end < at {
                line_end = content[at..].find('\n').map_or(content.len(), |n| at + n);
            }
            let line = &content[at..line_end];
            let after_line = (line_end + 1).min(content.len());
            let line_start = at == 0 || content.as_bytes()[at - 1] == b'\n';
            if line_start {
                if quotes && (line.starts_with(">>> ") || quote_body(line).is_some()) {
                    self.flush_paragraph(&mut blocks, &mut paragraph);
                    let mut quoted = String::new();
                    if line.starts_with(">>> ") {
                        quoted.push_str(&content[at + 4..]);
                        at = content.len();
                    } else {
                        let mut first = true;
                        loop {
                            let end = content[at..].find('\n').map_or(content.len(), |n| at + n);
                            let Some(body) = quote_body(&content[at..end]) else {
                                break;
                            };
                            if !first {
                                quoted.push('\n');
                            }
                            first = false;
                            quoted.push_str(body);
                            at = (end + 1).min(content.len());
                            if at == content.len() {
                                break;
                            }
                        }
                    }
                    blocks.push(Block::Quote(self.blocks(&quoted, false, true)));
                    continue;
                }
                if lists && list_marker(line).is_some() {
                    self.flush_paragraph(&mut blocks, &mut paragraph);
                    let (parsed, end) = self.list(content, at, quotes);
                    blocks.extend(parsed);
                    at = end;
                    continue;
                }
                let inline_end = next_fence.map_or(line_end, |(open, _)| open.min(line_end));
                let prefix = &content[at..inline_end];
                if let Some((level, body)) = heading(prefix) {
                    self.flush_paragraph(&mut blocks, &mut paragraph);
                    blocks.push(Block::Heading {
                        level,
                        spans: self.inline(body, Style::default(), None, true),
                    });
                    at = if inline_end == line_end {
                        after_line
                    } else {
                        inline_end
                    };
                    if content[at..].starts_with('\n') {
                        at += 1;
                    }
                    continue;
                }
                if let Some(body) = prefix.strip_prefix("-# ") {
                    if body.chars().any(|c| !c.is_whitespace()) {
                        self.flush_paragraph(&mut blocks, &mut paragraph);
                        blocks.push(Block::Subtext(self.inline(
                            body,
                            Style::default(),
                            None,
                            true,
                        )));
                        at = if inline_end == line_end {
                            after_line
                        } else {
                            inline_end
                        };
                        continue;
                    }
                }
            }
            let end = next_fence.map_or(after_line, |(open, _)| open.min(after_line));
            paragraph.push_str(&content[at..end]);
            at = end;
        }
        self.flush_paragraph(&mut blocks, &mut paragraph);
        blocks
    }

    fn flush_paragraph(&mut self, blocks: &mut Vec<Block>, text: &mut String) {
        if !text.is_empty() {
            blocks.push(Block::Paragraph(self.inline(
                text,
                Style::default(),
                None,
                true,
            )));
            text.clear();
        }
    }

    fn list(&mut self, content: &str, mut at: usize, quotes: bool) -> (Vec<Block>, usize) {
        let mut result = Vec::new();
        let mut frames: Vec<ListFrame> = Vec::new();
        while at < content.len() {
            let end = content[at..].find('\n').map_or(content.len(), |n| at + n);
            let Some(marker) = list_marker(&content[at..end]) else {
                break;
            };
            while frames
                .last()
                .is_some_and(|frame| marker.indent < frame.indent)
            {
                finish_list(&mut frames, &mut result);
            }
            let nested = frames
                .last()
                .is_some_and(|frame| marker.indent >= frame.indent + 2);
            if !nested
                && frames
                    .last()
                    .is_some_and(|frame| frame.start.is_some() != marker.start.is_some())
            {
                finish_list(&mut frames, &mut result);
            }
            if frames.is_empty()
                || nested
                || frames
                    .last()
                    .is_some_and(|frame| marker.indent >= frame.indent + 2)
            {
                frames.push(ListFrame {
                    indent: if frames.is_empty() { 0 } else { marker.indent },
                    start: marker.start,
                    items: Vec::new(),
                });
            }
            let item = self.blocks(marker.body, quotes, false);
            if let Some(frame) = frames.last_mut() {
                frame.items.push(item);
            }
            at = (end + 1).min(content.len());
        }
        while !frames.is_empty() {
            finish_list(&mut frames, &mut result);
        }
        (result, at)
    }

    fn inline(
        &mut self,
        text: &str,
        inherited: Style,
        link: Option<&str>,
        links: bool,
    ) -> Vec<Span> {
        let atoms = tokenize(text, links);
        let mut output = Vec::new();
        let mut counts = [
            usize::from(inherited.bold),
            usize::from(inherited.italic),
            usize::from(inherited.underline),
            usize::from(inherited.strikethrough),
        ];
        let mut spoilers = Vec::new();
        if let Some(index) = inherited.spoiler {
            spoilers.push(index);
        }
        for atom in atoms {
            match atom {
                Atom::Open(kind) => {
                    if kind == Mark::Spoiler {
                        spoilers.push(self.next_spoiler);
                        self.next_spoiler = self.next_spoiler.saturating_add(1);
                    } else {
                        counts[kind.index()] += 1;
                    }
                }
                Atom::Close(kind) => {
                    if kind == Mark::Spoiler {
                        spoilers.pop();
                    } else {
                        counts[kind.index()] = counts[kind.index()].saturating_sub(1);
                    }
                }
                atom => {
                    let style = Style {
                        bold: counts[0] > 0,
                        italic: counts[1] > 0,
                        underline: counts[2] > 0,
                        strikethrough: counts[3] > 0,
                        spoiler: spoilers.last().copied(),
                    };
                    match atom {
                        Atom::Text(value) => {
                            push_span(&mut output, SpanKind::Text(value), style, link)
                        }
                        Atom::Value(value) => push_span(&mut output, value, style, link),
                        Atom::Pending { literal, .. } => {
                            push_span(&mut output, SpanKind::Text(literal), style, link)
                        }
                        Atom::Url(url) => {
                            let kind = match channel_url(&url) {
                                Some((channel_id, Some(message_id))) => SpanKind::MessageLink {
                                    channel_id,
                                    message_id,
                                },
                                Some((channel_id, None)) => SpanKind::ChannelMention(channel_id),
                                None => SpanKind::Text(url.clone()),
                            };
                            push_span(&mut output, kind, style, Some(&url))
                        }
                        Atom::Label { label, url } => {
                            for span in self.inline(&label, style, Some(&url), false) {
                                push_span(&mut output, span.kind, span.style, span.link.as_deref());
                            }
                        }
                        Atom::Open(_) | Atom::Close(_) => unreachable!(),
                    }
                }
            }
        }
        output
    }
}

fn finish_list(frames: &mut Vec<ListFrame>, output: &mut Vec<Block>) {
    if let Some(frame) = frames.pop() {
        let block = Block::List {
            start: frame.start,
            items: frame.items,
        };
        if let Some(item) = frames.last_mut().and_then(|parent| parent.items.last_mut()) {
            item.push(block);
        } else {
            output.push(block);
        }
    }
}

fn fence_pair(content: &str, from: usize) -> Option<(usize, usize)> {
    let mut cursor = from;
    loop {
        let open = cursor + content[cursor..].find("```")?;
        let backslashes = content.as_bytes()[..open]
            .iter()
            .rev()
            .take_while(|&&b| b == b'\\')
            .count();
        if backslashes % 2 == 0 {
            let close = open + 3 + content[open + 3..].find("```")?;
            return Some((open, close));
        }
        cursor = open + 1;
    }
}

fn language_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_+-#.".contains(&c))
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let level = line.bytes().take_while(|&c| c == b'#').count();
    if !(1..=3).contains(&level) {
        return None;
    }
    let body = line[level..].strip_prefix(' ')?;
    body.chars()
        .any(|c| !c.is_whitespace())
        .then_some((level as u8, body))
}

fn quote_body(line: &str) -> Option<&str> {
    if line == ">" {
        Some("")
    } else {
        line.strip_prefix("> ")
    }
}

fn list_marker(line: &str) -> Option<ListMarker<'_>> {
    let indent = line.bytes().take_while(|&c| c == b' ').count();
    let text = &line[indent..];
    if let Some(body) = text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")) {
        return Some(ListMarker {
            indent,
            start: None,
            body,
        });
    }
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=9).contains(&digits) {
        return None;
    }
    let body = text[digits..].strip_prefix(". ")?;
    Some(ListMarker {
        indent,
        start: Some(text[..digits].parse().ok()?),
        body,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Bold,
    Italic,
    Underline,
    Strike,
    Spoiler,
}

impl Mark {
    fn index(self) -> usize {
        match self {
            Self::Bold => 0,
            Self::Italic => 1,
            Self::Underline => 2,
            Self::Strike => 3,
            Self::Spoiler => 4,
        }
    }
}

enum Atom {
    Text(String),
    Value(SpanKind),
    Pending { literal: String, kind: Mark },
    Open(Mark),
    Close(Mark),
    Url(String),
    Label { label: String, url: String },
}

struct Opener {
    atom: usize,
    slot: usize,
    previous: Option<usize>,
}

#[derive(Default)]
struct Delimiters {
    stack: Vec<Opener>,
    heads: [Option<usize>; 8],
}

impl Delimiters {
    /// Whether an unclosed `character` run is waiting for its closer.
    fn waiting(&self, character: char, doubled: bool) -> bool {
        let group = match character {
            '*' => 0,
            '_' => 2,
            '~' => 4,
            _ => 6,
        };
        self.heads[group].is_some() || (doubled && self.heads[group + 1].is_some())
    }

    fn run(
        &mut self,
        atoms: &mut Vec<Atom>,
        character: char,
        mut count: usize,
        before: Option<char>,
        after: Option<char>,
    ) {
        let group = match character {
            '*' => 0,
            '_' => 2,
            '~' => 4,
            _ => 6,
        };
        let open = after.is_some_and(|c| character != '*' || !c.is_whitespace());
        let close = before.is_some_and(|c| character != '*' || count >= 2 || !c.is_whitespace());
        if close {
            loop {
                let single = if ((character == '*' && before.is_some_and(|c| !c.is_whitespace()))
                    || (character == '_'
                        && !after.is_some_and(word_char)
                        && before.is_some_and(|c| !c.is_whitespace())))
                    && count >= 1
                {
                    self.heads[group]
                } else {
                    None
                };
                let double = if count >= 2 {
                    self.heads[group + 1]
                } else {
                    None
                };
                let Some(target) = single.into_iter().chain(double).max() else {
                    break;
                };
                let mut matched = None;
                while self.stack.len() > target {
                    if let Some(opener) = self.stack.pop() {
                        self.heads[opener.slot] = opener.previous;
                        if self.stack.len() == target {
                            matched = Some(opener);
                        }
                    }
                }
                if let Some(opener) = matched {
                    if let Atom::Pending { kind, .. } = atoms[opener.atom] {
                        atoms[opener.atom] = Atom::Open(kind);
                        atoms.push(Atom::Close(kind));
                    }
                    count -= if opener.slot % 2 == 0 { 1 } else { 2 };
                }
            }
        }
        while count > 0 {
            let size = if count >= 2 { 2 } else { 1 };
            let literal = character.to_string().repeat(size);
            let allowed = (open || (character == '*' && size == 2 && after.is_some()))
                && (size == 2
                    || character == '*'
                    || (character == '_'
                        && !before.is_some_and(word_char)
                        && after.is_some_and(|c| !c.is_whitespace())));
            if allowed {
                let kind = match (character, size) {
                    ('*', 2) => Mark::Bold,
                    ('_', 2) => Mark::Underline,
                    ('*' | '_', 1) => Mark::Italic,
                    ('~', _) => Mark::Strike,
                    _ => Mark::Spoiler,
                };
                let slot = group + size - 1;
                let atom = atoms.len();
                atoms.push(Atom::Pending { literal, kind });
                self.stack.push(Opener {
                    atom,
                    slot,
                    previous: self.heads[slot],
                });
                self.heads[slot] = Some(self.stack.len() - 1);
            } else {
                atom_text(atoms, &literal);
            }
            count -= size;
        }
    }
}

fn atom_text(atoms: &mut Vec<Atom>, text: &str) {
    if let Some(Atom::Text(previous)) = atoms.last_mut() {
        previous.push_str(text);
    } else {
        atoms.push(Atom::Text(text.to_owned()));
    }
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn channel_url(value: &str) -> Option<(u64, Option<u64>)> {
    let url = reqwest::Url::parse(value).ok()?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some(
                "discord.com"
                    | "ptb.discord.com"
                    | "canary.discord.com"
                    | "discordapp.com"
                    | "ptb.discordapp.com"
                    | "canary.discordapp.com"
            )
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let mut segments = url.path_segments()?;
    if segments.next()? != "channels" {
        return None;
    }
    let guild = segments.next()?;
    if guild != "@me" && guild.parse::<u64>().is_err() {
        return None;
    }
    let channel = segments.next()?.parse().ok()?;
    let message = match segments.next() {
        None => None,
        Some(message) => Some(message.parse().ok()?),
    };
    segments.next().is_none().then_some((channel, message))
}

fn syntax_pairs(text: &str) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let bytes = text.as_bytes();
    let mut ticks = vec![None; bytes.len()];
    let mut runs = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'`' {
            let start = at;
            while bytes.get(at) == Some(&b'`') {
                at += 1;
            }
            if at - start <= 2 {
                runs.push((start, at - start));
            }
        } else {
            at += 1;
        }
    }
    let mut next = [None; 2];
    for (at, size) in runs.into_iter().rev() {
        ticks[at] = next[size - 1];
        next[size - 1] = Some(at);
    }
    let mut pairs = vec![None; bytes.len()];
    let mut brackets = Vec::new();
    let mut parens = Vec::new();
    at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' && bytes.get(at + 1).is_some_and(u8::is_ascii_punctuation) {
            at += 2;
            continue;
        }
        if let Some(end) = ticks[at] {
            let size = if bytes.get(at + 1) == Some(&b'`') {
                2
            } else {
                1
            };
            at = end + size;
            continue;
        }
        match bytes[at] {
            b'[' => brackets.push(at),
            b']' => {
                if let Some(open) = brackets.pop() {
                    pairs[open] = Some(at);
                }
            }
            b'(' => parens.push(at),
            b')' => {
                if let Some(open) = parens.pop() {
                    pairs[open] = Some(at);
                }
            }
            _ => {}
        }
        at += 1;
    }
    (ticks, pairs)
}

fn shortcode(rest: &str) -> Option<(&'static str, usize)> {
    let close = rest[1..].find(':')? + 1;
    let name = &rest[1..close];
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_+-".contains(&b))
    {
        return None;
    }
    let after = &rest[close + 1..];
    if let Some(tone) = after
        .strip_prefix(":skin-tone-")
        .and_then(|tail| tail.get(..2)?.strip_suffix(':'))
        .filter(|tone| matches!(*tone, "1" | "2" | "3" | "4" | "5"))
        && let Some(emoji) = emoji::shortcode(&format!("{name}_tone{tone}"))
    {
        return Some((emoji, close + 1 + ":skin-tone-N:".len()));
    }
    emoji::shortcode(name).map(|emoji| (emoji, close + 1))
}

fn tokenize(text: &str, links: bool) -> Vec<Atom> {
    let (ticks, pairs) = syntax_pairs(text);
    let mut atoms = Vec::new();
    let mut delimiters = Delimiters::default();
    let mut at = 0;
    let mut previous = None;
    while at < text.len() {
        let rest = &text[at..];
        let Some(c) = rest.chars().next() else { break };
        let mut end = at + c.len_utf8();
        if c == '\\' && rest.as_bytes().get(1).is_some_and(u8::is_ascii_punctuation) {
            atom_text(&mut atoms, &text[at + 1..at + 2]);
            end = at + 2;
        } else if c == '`' {
            let size = rest.bytes().take_while(|&b| b == b'`').count();
            if size <= 2
                && let Some(close) = ticks[at]
            {
                let mut code = &text[at + size..close];
                if size == 2 && code.len() >= 2 && code.starts_with(' ') && code.ends_with(' ') {
                    code = &code[1..code.len() - 1];
                }
                atoms.push(Atom::Value(SpanKind::Code(code.to_owned())));
                end = close + size;
            } else {
                end = at + size;
                atom_text(&mut atoms, &text[at..end]);
            }
        } else if c == '['
            && links
            && let Some(close) = pairs[at]
            && close > at + 1
            && text.as_bytes().get(close + 1) == Some(&b'(')
            && let Some(url_end) = pairs.get(close + 1).copied().flatten()
            && let Some(url) = masked_url(&text[close + 2..url_end])
        {
            atoms.push(Atom::Label {
                label: text[at + 1..close].to_owned(),
                url: url.to_owned(),
            });
            end = url_end + 1;
        } else if c == '<'
            && let Some(close) = rest[1..].find(['<', '>']).map(|n| n + 1)
            && rest.as_bytes()[close] == b'>'
        {
            let body = &rest[1..close];
            if links && valid_url(body) {
                atoms.push(Atom::Url(body.to_owned()));
                end = at + close + 1;
            } else if let Some(value) = special_span(body) {
                atoms.push(Atom::Value(value));
                end = at + close + 1;
            } else {
                atom_text(&mut atoms, "<");
            }
        } else if links && (rest.starts_with("https://") || rest.starts_with("http://")) {
            let mut size = rest
                .find(|c: char| c.is_whitespace() || c == '<')
                .unwrap_or(rest.len());
            // Discord matches `**…**` and friends before autolinks, so a URL wrapped in
            // formatting ends where the closing run starts. An underscore only closes at a word
            // boundary, so `_` inside a URL path stays part of it.
            let candidate = &rest[..size];
            if let Some((close, _)) = candidate.char_indices().find(|&(index, c)| {
                matches!(c, '*' | '_' | '~' | '|')
                    // Check a run only once; repeatedly scanning its tail would be quadratic.
                    && (index == 0 || candidate.as_bytes()[index - 1] != c as u8)
                    && delimiters.waiting(
                        c,
                        candidate.as_bytes().get(index + 1) == Some(&(c as u8)),
                    )
                    && (c != '_'
                        || !candidate[index..]
                            .trim_start_matches('_')
                            .chars()
                            .next()
                            .is_some_and(word_char))
            }) {
                size = close;
            }
            let url = trim_url(&rest[..size]);
            if valid_url(url) {
                atoms.push(Atom::Url(url.to_owned()));
                end = at + url.len();
            } else {
                atom_text(&mut atoms, &text[at..end]);
            }
        } else if c == '@'
            && !previous.is_some_and(word_char)
            && let Some((name, kind)) =
                [("@everyone", SpanKind::Everyone), ("@here", SpanKind::Here)]
                    .into_iter()
                    .find(|(name, _)| {
                        rest.starts_with(name)
                            && !rest[name.len()..].chars().next().is_some_and(word_char)
                    })
        {
            atoms.push(Atom::Value(kind));
            end = at + name.len();
        } else if c == ':'
            && let Some((emoji, size)) = shortcode(rest)
        {
            atom_text(&mut atoms, emoji);
            end = at + size;
        } else if matches!(c, '*' | '_' | '~' | '|') {
            let count = rest.bytes().take_while(|&b| b == c as u8).count();
            end = at + count;
            delimiters.run(&mut atoms, c, count, previous, text[end..].chars().next());
        } else {
            atom_text(&mut atoms, &text[at..end]);
        }
        previous = text[..end].chars().next_back();
        at = end;
    }
    atoms
}

fn push_span(spans: &mut Vec<Span>, kind: SpanKind, style: Style, link: Option<&str>) {
    if let SpanKind::Text(value) = &kind {
        if value.is_empty() {
            return;
        }
        if let Some(Span {
            kind: SpanKind::Text(previous),
            style: previous_style,
            link: previous_link,
        }) = spans.last_mut()
            && *previous_style == style
            && previous_link.as_deref() == link
        {
            previous.push_str(value);
            return;
        }
    }
    spans.push(Span {
        kind,
        style,
        link: link.map(str::to_owned),
    });
}

fn valid_url(url: &str) -> bool {
    let body = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    body.is_some_and(|body| {
        !body.is_empty()
            && !body
                .chars()
                .any(|c| c.is_whitespace() || c == '<' || c == '>')
    })
}

fn masked_url(url: &str) -> Option<&str> {
    let url = if url.starts_with('<') {
        url.strip_prefix('<')?.strip_suffix('>')?
    } else {
        url
    };
    valid_url(url).then_some(url)
}

fn trim_url(mut url: &str) -> &str {
    let mut balance = url
        .bytes()
        .fold(0i64, |n, c| n + i64::from(c == b'(') - i64::from(c == b')'));
    loop {
        match url.as_bytes().last() {
            Some(b'.' | b',' | b':' | b';' | b'!' | b'?' | b'"' | b'\'') => {
                url = &url[..url.len() - 1]
            }
            Some(b')') if balance < 0 => {
                url = &url[..url.len() - 1];
                balance += 1;
            }
            _ => return url,
        }
    }
}

fn snowflake(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn special_span(body: &str) -> Option<SpanKind> {
    if let Some(id) = body.strip_prefix("@&") {
        return Some(SpanKind::RoleMention(snowflake(id)?));
    }
    if let Some(id) = body.strip_prefix("@!").or_else(|| body.strip_prefix('@')) {
        return Some(SpanKind::UserMention(snowflake(id)?));
    }
    if let Some(id) = body.strip_prefix('#') {
        return Some(SpanKind::ChannelMention(snowflake(id)?));
    }
    if let Some(emoji) = body.strip_prefix("a:").or_else(|| body.strip_prefix(':')) {
        let (name, id) = emoji.split_once(':')?;
        if !(1..=32).contains(&name.len())
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return None;
        }
        return Some(SpanKind::Emoji {
            id: snowflake(id)?,
            name: name.to_owned(),
            animated: body.starts_with("a:"),
        });
    }
    if let Some(timestamp) = body.strip_prefix("t:") {
        let (unix, style) = timestamp.split_once(':').unwrap_or((timestamp, "f"));
        let digits = unix.strip_prefix('-').unwrap_or(unix);
        if digits.is_empty()
            || !digits.bytes().all(|b| b.is_ascii_digit())
            || style.len() != 1
            || !b"tTdDfFR".contains(&style.as_bytes()[0])
        {
            return None;
        }
        return Some(SpanKind::Timestamp {
            unix: unix.parse().ok()?,
            style: style.chars().next()?,
        });
    }
    if let Some(command) = body.strip_prefix('/') {
        let (name, id) = command.rsplit_once(':')?;
        let words: Vec<&str> = name.split(' ').collect();
        if !(1..=3).contains(&words.len())
            || words.iter().any(|word| {
                word.is_empty()
                    || !word
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            })
        {
            return None;
        }
        return Some(SpanKind::Command {
            name: name.to_owned(),
            id: snowflake(id)?,
        });
    }
    None
}

pub(crate) fn pictographic(c: char) -> bool {
    matches!(c as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2300..=0x23FF | 0x2B00..=0x2BFF | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x21AA)
}

fn emoji_component(
    chars: &[char],
    mut at: usize,
    is_pictographic: impl Fn(char) -> bool,
) -> Option<usize> {
    let c = *chars.get(at)?;
    if matches!(c, '0'..='9' | '#' | '*') {
        at += 1;
        if chars
            .get(at)
            .is_some_and(|&c| matches!(c, '\u{FE0E}' | '\u{FE0F}'))
        {
            at += 1;
        }
        return (chars.get(at) == Some(&'\u{20E3}')).then_some(at + 1);
    }
    if matches!(c as u32, 0x1F1E6..=0x1F1FF) {
        return chars
            .get(at + 1)
            .filter(|&&c| matches!(c as u32, 0x1F1E6..=0x1F1FF))
            .map(|_| at + 2);
    }
    if !is_pictographic(c) || matches!(c as u32, 0x1F3FB..=0x1F3FF) {
        return None;
    }
    at += 1;
    while chars.get(at).is_some_and(
        |&c| matches!(c as u32, 0xFE0E..=0xFE0F | 0x1F3FB..=0x1F3FF | 0xE0020..=0xE007F),
    ) {
        at += 1;
    }
    Some(at)
}

pub(crate) fn emoji_sequence(
    chars: &[char],
    at: usize,
    is_pictographic: impl Fn(char) -> bool + Copy,
) -> Result<usize, usize> {
    let mut end = emoji_component(chars, at, is_pictographic).ok_or(at + 1)?;
    while chars.get(end) == Some(&'\u{200D}') {
        end = emoji_component(chars, end + 1, is_pictographic).ok_or(end + 1)?;
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn text(value: &str) -> Span {
        Span {
            kind: SpanKind::Text(value.to_owned()),
            style: Style::default(),
            link: None,
        }
    }

    fn paragraph(value: &str) -> Block {
        Block::Paragraph(vec![text(value)])
    }

    fn spans(value: &str) -> Vec<Span> {
        let parsed = parse(value);
        let [Block::Paragraph(spans)] = parsed.as_slice() else {
            panic!("expected paragraph, got {parsed:?}");
        };
        spans.clone()
    }

    fn code(language: Option<&str>, value: &str) -> Block {
        Block::CodeBlock {
            language: language.map(str::to_owned),
            code: value.to_owned(),
        }
    }

    #[test]
    fn markdown_empty_message() {
        assert!(parse("").is_empty());
        assert!(parse("\n\n").is_empty());
        assert!(!is_jumbo(&[]));
    }

    #[test]
    fn markdown_preserves_paragraph_newlines() {
        assert_eq!(
            parse("\nhello\n\n\nworld\n"),
            vec![paragraph("hello\n\n\nworld")]
        );
        assert_eq!(parse("  hello  "), vec![paragraph("  hello  ")]);
    }

    #[test]
    fn markdown_consumes_exactly_one_newline_after_blocks() {
        assert_eq!(
            parse("# title\n\ntext"),
            vec![
                Block::Heading {
                    level: 1,
                    spans: vec![text("title")]
                },
                paragraph("text")
            ]
        );
        assert_eq!(
            parse("```code```\n\ntext"),
            vec![code(None, "code"), paragraph("\ntext")]
        );
        assert_eq!(
            parse("text\n\n# title"),
            vec![
                paragraph("text\n\n"),
                Block::Heading {
                    level: 1,
                    spans: vec![text("title")]
                }
            ]
        );
    }

    #[test]
    fn markdown_code_block_language() {
        assert_eq!(
            parse("```rust\nlet x = 1;\n```"),
            vec![code(Some("rust"), "let x = 1;")]
        );
        for lang in ["c++", "c#", "objective-c", "lang_v1.2", "123"] {
            assert_eq!(
                parse(&format!("```{lang}\nx```")),
                vec![code(Some(lang), "x")]
            );
        }
    }

    #[test]
    fn markdown_code_block_no_language() {
        assert_eq!(parse("```\nhello\n```"), vec![code(None, "\nhello")]);
        assert_eq!(
            parse("```not a language\nhello\n```"),
            vec![code(None, "not a language\nhello")]
        );
        assert_eq!(
            parse("``` rust\nhello```"),
            vec![code(None, " rust\nhello")]
        );
    }

    #[test]
    fn markdown_code_block_one_line_and_mid_line() {
        assert_eq!(parse("```hello```"), vec![code(None, "hello")]);
        assert_eq!(
            parse("before ```hello``` after"),
            vec![
                paragraph("before "),
                code(None, "hello"),
                paragraph(" after")
            ]
        );
        assert_eq!(parse("``````"), vec![code(None, "")]);
    }

    #[test]
    fn markdown_code_block_literal_and_trailing_newline() {
        assert_eq!(
            parse("```rs\n**x** <@1> ||x||\n\n```"),
            vec![code(Some("rs"), "**x** <@1> ||x||\n")]
        );
    }

    #[test]
    fn markdown_unclosed_code_fence() {
        assert_eq!(
            parse("```hello\n# heading"),
            vec![
                paragraph("```hello\n"),
                Block::Heading {
                    level: 1,
                    spans: vec![text("heading")]
                }
            ]
        );
    }

    #[test]
    fn markdown_heading_levels() {
        for level in 1..=3 {
            assert_eq!(
                parse(&format!("{} title", "#".repeat(level))),
                vec![Block::Heading {
                    level: level as u8,
                    spans: vec![text("title")]
                }]
            );
        }
    }

    #[test]
    fn markdown_heading_requires_valid_prefix_and_content() {
        for value in ["#### x", "#x", "# ", "##   ", "before # title", " # title"] {
            assert_eq!(parse(value), vec![paragraph(value)]);
        }
    }

    #[test]
    fn markdown_heading_and_subtext_inline_styles() {
        let bold = Span {
            style: Style {
                bold: true,
                ..Style::default()
            },
            ..text("title")
        };
        assert_eq!(
            parse("# **title**\n-# **title**"),
            vec![
                Block::Heading {
                    level: 1,
                    spans: vec![bold.clone()]
                },
                Block::Subtext(vec![bold])
            ]
        );
        assert_eq!(parse("-# "), vec![paragraph("-# ")]);
    }

    #[test]
    fn markdown_consecutive_quote_lines() {
        assert_eq!(
            parse("> a\n> b\ntext"),
            vec![Block::Quote(vec![paragraph("a\nb")]), paragraph("text")]
        );
        assert_eq!(
            parse("before\n> a"),
            vec![paragraph("before\n"), Block::Quote(vec![paragraph("a")])]
        );
    }

    #[test]
    fn markdown_empty_quote_lines_and_space_requirement() {
        assert_eq!(
            parse(">\n> a\n>\n> b"),
            vec![Block::Quote(vec![paragraph("\na\n\nb")])]
        );
        assert_eq!(parse(">"), vec![Block::Quote(vec![])]);
        assert_eq!(parse(">text"), vec![paragraph(">text")]);
    }

    #[test]
    fn markdown_multiline_quote() {
        assert_eq!(
            parse(">>> first\nsecond\n# heading"),
            vec![Block::Quote(vec![
                paragraph("first\nsecond\n"),
                Block::Heading {
                    level: 1,
                    spans: vec![text("heading")]
                }
            ])]
        );
    }

    #[test]
    fn markdown_quotes_do_not_nest() {
        assert_eq!(
            parse("> > inner\n> >>> still text"),
            vec![Block::Quote(vec![paragraph("> inner\n>>> still text")])]
        );
    }

    #[test]
    fn markdown_quote_contains_blocks() {
        assert_eq!(
            parse("> # title\n> - a\n> - b\n> ```rs\n> x\n> ```"),
            vec![Block::Quote(vec![
                Block::Heading {
                    level: 1,
                    spans: vec![text("title")]
                },
                Block::List {
                    start: None,
                    items: vec![vec![paragraph("a")], vec![paragraph("b")]]
                },
                code(Some("rs"), "x"),
            ])]
        );
    }

    #[test]
    fn markdown_bullet_lists() {
        assert_eq!(
            parse("- a\n* b\nend"),
            vec![
                Block::List {
                    start: None,
                    items: vec![vec![paragraph("a")], vec![paragraph("b")]]
                },
                paragraph("end")
            ]
        );
        assert_eq!(
            spans("*bold*")[0].style,
            Style {
                italic: true,
                ..Style::default()
            }
        );
    }

    #[test]
    fn markdown_ordered_list_start_and_digit_limit() {
        assert_eq!(
            parse("42. a\n99. b"),
            vec![Block::List {
                start: Some(42),
                items: vec![vec![paragraph("a")], vec![paragraph("b")]]
            }]
        );
        assert_eq!(parse("1234567890. a"), vec![paragraph("1234567890. a")]);
    }

    #[test]
    fn markdown_nested_lists_and_dedent() {
        assert_eq!(
            parse("- parent\n  3. child\n    - grandchild\n  4. sibling\n- next"),
            vec![Block::List {
                start: None,
                items: vec![
                    vec![
                        paragraph("parent"),
                        Block::List {
                            start: Some(3),
                            items: vec![
                                vec![
                                    paragraph("child"),
                                    Block::List {
                                        start: None,
                                        items: vec![vec![paragraph("grandchild")]]
                                    }
                                ],
                                vec![paragraph("sibling")],
                            ]
                        }
                    ],
                    vec![paragraph("next")]
                ],
            }]
        );
    }

    #[test]
    fn markdown_one_space_does_not_nest_lists() {
        assert_eq!(
            parse("- a\n - b"),
            vec![Block::List {
                start: None,
                items: vec![vec![paragraph("a")], vec![paragraph("b")]]
            }]
        );
    }

    #[test]
    fn markdown_list_type_changes() {
        assert_eq!(
            parse("- a\n2. b\n- c"),
            vec![
                Block::List {
                    start: None,
                    items: vec![vec![paragraph("a")]]
                },
                Block::List {
                    start: Some(2),
                    items: vec![vec![paragraph("b")]]
                },
                Block::List {
                    start: None,
                    items: vec![vec![paragraph("c")]]
                },
            ]
        );
    }

    #[test]
    fn markdown_escapes_ascii_punctuation() {
        assert_eq!(
            spans(r"\*x\* \\ \a \! \_ \|"),
            vec![text(r"*x* \ \a ! _ |")]
        );
        for c in (0u8..=127).filter(u8::is_ascii_punctuation) {
            let value = format!("\\{}", c as char);
            assert_eq!(spans(&value), vec![text(&(c as char).to_string())]);
        }
    }

    #[test]
    fn markdown_inline_code_is_literal() {
        assert_eq!(
            spans("`**x** <@1> ||x||`"),
            vec![Span {
                kind: SpanKind::Code("**x** <@1> ||x||".to_owned()),
                style: Style::default(),
                link: None
            }]
        );
        assert_eq!(spans("` x `")[0].kind, SpanKind::Code(" x ".to_owned()));
    }

    #[test]
    fn markdown_double_backtick_code() {
        assert_eq!(
            spans("`` x ` y ``")[0].kind,
            SpanKind::Code("x ` y".to_owned())
        );
        assert_eq!(spans("`` x``")[0].kind, SpanKind::Code(" x".to_owned()));
        assert_eq!(spans("``"), vec![text("``")]);
    }

    #[test]
    fn markdown_code_does_not_process_backslash_escapes() {
        assert_eq!(
            spans(r"`a\` tail")[0].kind,
            SpanKind::Code("a\\".to_owned())
        );
        assert_eq!(spans(r"\`literal\`"), vec![text("`literal`")]);
    }

    #[test]
    fn markdown_escaped_fence_is_literal() {
        assert_eq!(parse(r"\```hello```"), vec![paragraph("```hello```")]);
        assert_eq!(
            parse(r"\\```hello```"),
            vec![paragraph("\\"), code(None, "hello")]
        );
    }

    #[test]
    fn markdown_spoilers_and_strike_allow_edge_spaces() {
        assert_eq!(
            spans("|| secret ||"),
            vec![Span {
                style: Style {
                    spoiler: Some(0),
                    ..Style::default()
                },
                ..text(" secret ")
            }]
        );
        assert_eq!(
            spans("~~ strike ~~"),
            vec![Span {
                style: Style {
                    strikethrough: true,
                    ..Style::default()
                },
                ..text(" strike ")
            }]
        );
        assert_eq!(
            spans("__ under __"),
            vec![Span {
                style: Style {
                    underline: true,
                    ..Style::default()
                },
                ..text(" under ")
            }]
        );
    }

    #[test]
    fn markdown_individual_styles() {
        for (value, expected) in [
            (
                "**x**",
                Style {
                    bold: true,
                    ..Style::default()
                },
            ),
            (
                "*x*",
                Style {
                    italic: true,
                    ..Style::default()
                },
            ),
            (
                "_x_",
                Style {
                    italic: true,
                    ..Style::default()
                },
            ),
            (
                "__x__",
                Style {
                    underline: true,
                    ..Style::default()
                },
            ),
            (
                "~~x~~",
                Style {
                    strikethrough: true,
                    ..Style::default()
                },
            ),
        ] {
            assert_eq!(
                spans(value),
                vec![Span {
                    style: expected,
                    ..text("x")
                }]
            );
        }
    }

    #[test]
    fn markdown_triple_markers() {
        assert_eq!(
            spans("***x***")[0].style,
            Style {
                bold: true,
                italic: true,
                ..Style::default()
            }
        );
        assert_eq!(
            spans("___x___")[0].style,
            Style {
                underline: true,
                italic: true,
                ..Style::default()
            }
        );
    }

    #[test]
    fn markdown_nested_styles() {
        let actual = spans("**bold *both* bold**");
        assert_eq!(actual.len(), 3);
        assert!(actual.iter().all(|span| span.style.bold));
        assert_eq!(
            actual[1],
            Span {
                style: Style {
                    bold: true,
                    italic: true,
                    ..Style::default()
                },
                ..text("both")
            }
        );
        assert!(!actual[0].style.italic && !actual[2].style.italic);
        let all = spans("__**~~*x*~~**__");
        assert_eq!(
            all[0].style,
            Style {
                bold: true,
                italic: true,
                underline: true,
                strikethrough: true,
                spoiler: None
            }
        );
    }

    #[test]
    fn markdown_shared_closing_marker_run() {
        assert_eq!(
            spans("**bold *both***")[1].style,
            Style {
                bold: true,
                italic: true,
                ..Style::default()
            }
        );
        assert_eq!(
            spans("*italic **both***")[1].style,
            Style {
                bold: true,
                italic: true,
                ..Style::default()
            }
        );
    }

    #[test]
    fn markdown_snake_case_and_word_boundaries() {
        for value in ["snake_case_name", "word_x_", "_x_word", "word_x_word"] {
            assert_eq!(spans(value), vec![text(value)]);
        }
        assert!(spans("(_x_)")[1].style.italic);
    }

    #[test]
    fn markdown_markers_require_nonspace_content() {
        for value in ["* x*", "*x *", "* *", "****", "||||", "~~~~"] {
            assert_eq!(
                Parser::default().inline(value, Style::default(), None, true),
                vec![text(value)]
            );
        }
    }

    #[test]
    fn markdown_strong_allows_inner_whitespace() {
        for (value, expected) in [
            ("** x**", " x"),
            ("**x **", "x "),
            ("**5. No Self-Promotion **Please", "5. No Self-Promotion "),
        ] {
            let actual = spans(value);
            assert_eq!(actual[0].kind, SpanKind::Text(expected.into()));
            assert!(actual[0].style.bold);
        }
        let actual = spans("into the** language-specific** channels");
        assert_eq!(actual[1].kind, SpanKind::Text(" language-specific".into()));
        assert!(actual[1].style.bold);
    }

    #[test]
    fn markdown_channel_links_preserve_their_original_url() {
        for host in [
            "discord.com",
            "ptb.discord.com",
            "canary.discord.com",
            "discordapp.com",
            "ptb.discordapp.com",
            "canary.discordapp.com",
        ] {
            let url = format!("https://{host}/channels/123/456");
            for content in [url.clone(), format!("<{url}>")] {
                let actual = spans(&content);
                assert_eq!(actual[0].kind, SpanKind::ChannelMention(456));
                assert_eq!(actual[0].link.as_deref(), Some(url.as_str()));
            }
            let actual = spans(&format!("[{url}](https://example.com)"));
            assert_eq!(actual[0].kind, SpanKind::Text(url.clone()));
            assert_eq!(actual[0].link.as_deref(), Some("https://example.com"));
            let message = format!("{url}/789");
            assert_eq!(
                spans(&message)[0].kind,
                SpanKind::MessageLink {
                    channel_id: 456,
                    message_id: 789
                }
            );
            assert_eq!(spans(&message)[0].link.as_deref(), Some(message.as_str()));
            let deeper = format!("{url}/789/1");
            assert_eq!(spans(&deeper)[0].kind, SpanKind::Text(deeper));
        }
        for url in [
            "https://discord.com.evil.test/channels/123/456",
            "https://example.com/channels/123/456",
            "https://discord.com/channels/no/456",
        ] {
            assert_eq!(spans(url)[0].kind, SpanKind::Text(url.into()));
        }
    }

    #[test]
    fn markdown_formatting_fixture_mentions_strong_and_nested_lists() {
        fn collect_spans<'a>(blocks: &'a [Block], output: &mut Vec<&'a Span>) {
            for block in blocks {
                match block {
                    Block::Paragraph(spans)
                    | Block::Heading { spans, .. }
                    | Block::Subtext(spans) => output.extend(spans),
                    Block::Quote(blocks) => collect_spans(blocks, output),
                    Block::List { items, .. } => {
                        for blocks in items {
                            collect_spans(blocks, output);
                        }
                    }
                    Block::CodeBlock { .. } => {}
                }
            }
        }
        let message: crate::discord::model::Message =
            serde_json::from_str(include_str!("../store/fixtures/formatting.json")).unwrap();
        let blocks = parse(&message.content);
        let mut actual = Vec::new();
        collect_spans(&blocks, &mut actual);
        for expected in ["5. No Self-Promotion ", " language-specific"] {
            assert!(
                actual
                    .iter()
                    .any(|span| span.kind == SpanKind::Text(expected.into()) && span.style.bold)
            );
        }
        assert!(
            actual
                .iter()
                .any(|span| span.kind == SpanKind::UserMention(1191169014890496024))
        );
        assert!(
            actual
                .iter()
                .any(|span| span.kind == SpanKind::RoleMention(1191169065222144036))
        );
        assert!(actual.iter().any(|span| matches!(&span.kind, SpanKind::Emoji { id: 1191169094582272043, name, animated: false } if name == "sample_icon")));
        assert!(actual.iter().any(|span| span.kind
            == SpanKind::ChannelMention(1191169073610752038)
            && span.link.as_deref()
                == Some("https://discord.com/channels/1191169056833536034/1191169073610752038")));
        let lists: Vec<_> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::List { items, .. } => Some(items),
                _ => None,
            })
            .collect();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].len(), 2);
        assert!(plain_text(&lists[0][0]).starts_with("Sample Community Mod:"));
        assert!(
            matches!(&lists[0][0][1], Block::List { items, .. } if items.len() == 1 && plain_text(&items[0]).starts_with("Community Mod:"))
        );
        assert!(plain_text(&lists[0][1]).starts_with("For Sample Community"));
        assert!(
            matches!(&lists[0][1][1], Block::List { items, .. } if items.len() == 2 && plain_text(&items[0]).starts_with("Discuss with"))
        );
    }

    #[test]
    fn markdown_unmatched_markers_are_literal() {
        for value in [
            "**bold",
            "italic*",
            "__under",
            "~~strike",
            "||spoiler",
            "[link",
            "`code",
        ] {
            assert_eq!(spans(value), vec![text(value)]);
        }
        let actual = spans("**outer *inner*");
        assert_eq!(actual[0], text("**outer "));
        assert!(actual[1].style.italic && !actual[1].style.bold);
    }

    #[test]
    fn markdown_spoiler_indices_follow_source_order_across_blocks() {
        let parsed =
            parse("||a **b**||\n# ||c||\n> ||d||\n- ||e||\n- [||f||](https://example.com)");
        let mut indices = Vec::new();
        let mut pending: Vec<&Block> = parsed.iter().rev().collect();
        while let Some(block) = pending.pop() {
            match block {
                Block::Paragraph(spans) | Block::Heading { spans, .. } | Block::Subtext(spans) => {
                    indices.extend(spans.iter().filter_map(|span| span.style.spoiler));
                }
                Block::Quote(blocks) => pending.extend(blocks.iter().rev()),
                Block::List { items, .. } => {
                    pending.extend(items.iter().rev().flat_map(|item| item.iter().rev()))
                }
                Block::CodeBlock { .. } => {}
            }
        }
        assert_eq!(indices, vec![0, 0, 1, 2, 3, 4]);
    }

    #[test]
    fn markdown_unmatched_spoiler_does_not_reserve_index() {
        let actual = spans("||unmatched [||inside||](https://example.com)");
        assert_eq!(actual[0], text("||unmatched "));
        assert_eq!(actual[1].style.spoiler, Some(0));
    }

    #[test]
    fn markdown_autolinks_trim_punctuation() {
        let actual = spans("https://example.com.,:;!?\"'");
        assert_eq!(
            actual[0].kind,
            SpanKind::Text("https://example.com".to_owned())
        );
        assert_eq!(actual[0].link.as_deref(), Some("https://example.com"));
        assert_eq!(actual[1], text(".,:;!?\"'"));
    }

    #[test]
    fn markdown_autolinks_balance_parentheses() {
        let actual = spans("(https://example.com/a(b))).");
        assert_eq!(actual[1].link.as_deref(), Some("https://example.com/a(b)"));
        assert_eq!(actual[2], text("))."));
    }

    #[test]
    fn markdown_angle_links() {
        let actual = spans("<https://example.com> <http://example.com>");
        assert_eq!(
            actual[0].kind,
            SpanKind::Text("https://example.com".to_owned())
        );
        assert_eq!(actual[0].link.as_deref(), Some("https://example.com"));
        assert_eq!(actual[2].link.as_deref(), Some("http://example.com"));
    }

    #[test]
    fn markdown_masked_links_preserve_styles_and_emoji() {
        let actual = spans("[**label** <:face:1>](https://example.com/a(b))");
        assert_eq!(actual.len(), 3);
        assert!(actual[0].style.bold);
        assert!(matches!(actual[2].kind, SpanKind::Emoji { id: 1, .. }));
        assert!(
            actual
                .iter()
                .all(|span| span.link.as_deref() == Some("https://example.com/a(b)"))
        );
    }

    #[test]
    fn markdown_masked_angle_url_and_url_label() {
        let actual = spans("[https://label.example](<https://target.example>)");
        assert_eq!(
            actual,
            vec![Span {
                link: Some("https://target.example".to_owned()),
                ..text("https://label.example")
            }]
        );
    }

    #[test]
    fn markdown_formatting_around_bare_urls_closes_before_them() {
        let bold = spans("**https://irlcat.com**");
        assert_eq!(bold.len(), 1);
        assert!(bold[0].style.bold);
        assert_eq!(
            bold[0].kind,
            SpanKind::Text("https://irlcat.com".to_owned())
        );
        assert_eq!(bold[0].link.as_deref(), Some("https://irlcat.com"));

        let path = spans("_hi https://a.example/foo_bar");
        assert_eq!(
            path.last().and_then(|span| span.link.as_deref()),
            Some("https://a.example/foo_bar")
        );
    }

    #[test]
    fn bare_urls_keep_single_delimiters_inside_double_formatting() {
        for (marker, url) in [
            ("**", "https://a.example/a*b"),
            ("||", "https://a.example/a|b"),
            ("~~", "https://a.example/a~b"),
        ] {
            let parsed = spans(&format!("{marker}{url}{marker}"));
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].link.as_deref(), Some(url));
        }
        let url = format!("https://a.example/{}end", "_".repeat(4096));
        let parsed = spans(&format!("_hi {url}"));
        assert_eq!(
            parsed.last().and_then(|span| span.link.as_deref()),
            Some(url.as_str())
        );
    }

    #[test]
    fn markdown_link_labels_do_not_nest_links() {
        let actual = spans("[[inner](https://inner.example) <@1>](https://outer.example)");
        assert_eq!(
            actual[0].kind,
            SpanKind::Text("[inner](https://inner.example) ".to_owned())
        );
        assert!(matches!(actual[1].kind, SpanKind::UserMention(1)));
        assert!(
            actual
                .iter()
                .all(|span| span.link.as_deref() == Some("https://outer.example"))
        );
    }

    #[test]
    fn markdown_invalid_links_remain_text() {
        for value in [
            "[x](ftp://example.com)",
            "[x](https://)",
            "[](ftp://example.com)",
            "ftp://example.com",
            "https://",
        ] {
            assert_eq!(spans(value), vec![text(value)]);
        }
    }

    #[test]
    fn markdown_mentions() {
        let actual = spans("<@1><@!2><@&3><#4>");
        assert_eq!(
            actual
                .iter()
                .map(|span| span.kind.clone())
                .collect::<Vec<_>>(),
            vec![
                SpanKind::UserMention(1),
                SpanKind::UserMention(2),
                SpanKind::RoleMention(3),
                SpanKind::ChannelMention(4)
            ]
        );
    }

    #[test]
    fn markdown_mentions_require_u64_ids() {
        for value in ["<@18446744073709551616>", "<@-1>", "<@+1>", "<#x>", "<@&>"] {
            assert_eq!(spans(value), vec![text(value)]);
        }
        assert_eq!(
            spans("<@18446744073709551615>")[0].kind,
            SpanKind::UserMention(u64::MAX)
        );
    }

    #[test]
    fn markdown_everyone_and_here_word_boundaries() {
        assert_eq!(
            spans("@everyone @here")
                .iter()
                .map(|span| span.kind.clone())
                .collect::<Vec<_>>(),
            vec![
                SpanKind::Everyone,
                SpanKind::Text(" ".to_owned()),
                SpanKind::Here
            ]
        );
        for value in ["@everyonex", "@here2", "email@here", "@here_name"] {
            assert_eq!(spans(value), vec![text(value)]);
        }
    }

    #[test]
    fn markdown_custom_emoji() {
        assert_eq!(
            spans("<:face_1:42><a:wave:43>")
                .iter()
                .map(|span| span.kind.clone())
                .collect::<Vec<_>>(),
            vec![
                SpanKind::Emoji {
                    id: 42,
                    name: "face_1".to_owned(),
                    animated: false
                },
                SpanKind::Emoji {
                    id: 43,
                    name: "wave".to_owned(),
                    animated: true
                },
            ]
        );
        for value in [
            "<::1>",
            "<:bad-name:1>",
            "<:name:x>",
            "<a:name:18446744073709551616>",
        ] {
            assert_eq!(spans(value), vec![text(value)]);
        }
        let too_long = format!("<:{}:1>", "x".repeat(33));
        assert_eq!(spans(&too_long), vec![text(&too_long)]);
    }

    #[test]
    fn markdown_timestamps() {
        assert_eq!(
            spans("<t:1700000000>")[0].kind,
            SpanKind::Timestamp {
                unix: 1700000000,
                style: 'f'
            }
        );
        for style in ['t', 'T', 'd', 'D', 'f', 'F', 'R'] {
            assert_eq!(
                spans(&format!("<t:-123:{style}>"))[0].kind,
                SpanKind::Timestamp { unix: -123, style }
            );
        }
        assert_eq!(
            spans("<t:-9223372036854775808:R>")[0].kind,
            SpanKind::Timestamp {
                unix: i64::MIN,
                style: 'R'
            }
        );
    }

    #[test]
    fn markdown_invalid_timestamps() {
        for value in [
            "<t:1:X>",
            "<t:1:>",
            "<t:1:RR>",
            "<t:+1>",
            "<t:9223372036854775808>",
            "<t:->",
        ] {
            assert_eq!(spans(value), vec![text(value)]);
        }
    }

    #[test]
    fn markdown_command_mentions() {
        for name in ["cmd", "cmd sub", "cmd sub group"] {
            assert_eq!(
                spans(&format!("</{name}:42>"))[0].kind,
                SpanKind::Command {
                    name: name.to_owned(),
                    id: 42
                }
            );
        }
        for value in ["</:1>", "</cmd  sub:1>", "</cmd a b c:1>", "</cmd:no>"] {
            assert_eq!(spans(value), vec![text(value)]);
        }
    }

    #[test]
    fn markdown_special_spans_inherit_style_and_link() {
        let actual = spans(
            "[**<@1><#2><@&3>@everyone @here <:face:4><t:1:R></cmd:5>`code`**](https://example.com)",
        );
        assert!(
            actual
                .iter()
                .all(|span| span.style.bold && span.link.as_deref() == Some("https://example.com"))
        );
        assert!(
            actual
                .iter()
                .any(|span| matches!(span.kind, SpanKind::Code(_)))
        );
        assert!(
            actual
                .iter()
                .any(|span| matches!(span.kind, SpanKind::Command { .. }))
        );
    }

    #[test]
    fn markdown_merges_adjacent_text() {
        assert_eq!(spans(r"a\!b\*c"), vec![text("a!b*c")]);
        assert_eq!(
            spans("**a****b**"),
            vec![Span {
                style: Style {
                    bold: true,
                    ..Style::default()
                },
                ..text("ab")
            }]
        );
        let linked = spans("[a](https://example.com)[b](https://example.com)");
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].kind, SpanKind::Text("ab".to_owned()));
    }

    #[test]
    fn markdown_jumbo_custom_emoji() {
        assert!(is_jumbo(&parse("<:a:1> <a:b:2>")));
        assert!(is_jumbo(&parse(&"<:a:1>".repeat(30))));
        assert!(!is_jumbo(&parse(&"<:a:1>".repeat(31))));
    }

    #[test]
    fn markdown_jumbo_unicode_sequences_and_flags() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let flag = "\u{1F1EE}\u{1F1E9}";
        assert!(is_jumbo(&parse(&format!("{family} {flag}"))));
        assert!(is_jumbo(&parse(&family.repeat(30))));
        assert!(is_jumbo(&parse(&flag.repeat(30))));
        assert!(!is_jumbo(&parse(&flag.repeat(31))));
        assert!(!is_jumbo(&parse("\u{1F1EE}")));
    }

    #[test]
    fn markdown_jumbo_modifiers_keycaps_and_tags() {
        for value in [
            "\u{1F44D}\u{1F3FD}",
            "1\u{FE0F}\u{20E3}",
            "#\u{20E3}",
            "*\u{FE0F}\u{20E3}",
            "\u{00A9}\u{FE0E}",
            "\u{1F3F4}\u{E0067}\u{E0062}\u{E007F}",
        ] {
            assert!(is_jumbo(&parse(value)), "{value:?}");
        }
        for value in [
            "\u{200D}",
            "\u{FE0F}",
            "\u{1F3FD}",
            "\u{E0067}",
            "1",
            "#",
            "\u{1F600}\u{200D}",
        ] {
            assert!(!is_jumbo(&parse(value)), "{value:?}");
        }
    }

    #[test]
    fn markdown_jumbo_mixed_and_nonemoji() {
        assert!(is_jumbo(&parse("<:face:1> \u{1F600}\n\u{2764}\u{FE0F}")));
        for value in [
            "text \u{1F600}",
            "<@1>",
            "`\u{1F600}`",
            "# \u{1F600}",
            "   ",
            "\u{1F600}\n# x",
        ] {
            assert!(!is_jumbo(&parse(value)), "{value:?}");
        }
        assert!(!is_jumbo(&parse(&"\u{1F600}".repeat(31))));
    }

    #[test]
    fn markdown_plain_text_all_span_kinds() {
        let value = "**hi** `code` <@1> <@!2> <#3> <@&4> @everyone @here <:face:5> <t:6> <t:-7:R> </cmd sub:8>";
        assert_eq!(
            plain_text(&parse(value)),
            "hi code @1 @2 #3 @&4 @everyone @here :face: <t:6:f> <t:-7:R> /cmd sub"
        );
    }

    #[test]
    fn markdown_plain_text_blocks_and_whitespace() {
        assert_eq!(
            plain_text(&parse(
                "# title\n-# sub\n> quote\n- first\n  - child\n- second\n```rs\nlet x\n= 1;\n```\nend\t  here"
            )),
            "title sub quote first child second let x = 1; end here"
        );
        assert_eq!(
            plain_text(&parse("[label](https://example.com)\n\n rest")),
            "label rest"
        );
    }

    #[test]
    fn markdown_fuzz_fixed_seed() {
        let alphabet = [
            '*',
            '_',
            '|',
            '`',
            '[',
            ']',
            '(',
            ')',
            '<',
            '>',
            '\\',
            '\n',
            ' ',
            '#',
            '-',
            '~',
            '@',
            ':',
            '/',
            '1',
            'a',
            '\u{1F600}',
            '\u{200D}',
            '\u{FE0F}',
            '\0',
        ];
        let mut seed = 0x5eed_u64;
        for iteration in 0..3000 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let len = (seed >> 32) as usize % 384;
            let mut bytes = Vec::with_capacity(len);
            let mut text = String::new();
            for _ in 0..len {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                bytes.push((seed >> 32) as u8);
                text.push(alphabet[(seed >> 32) as usize % alphabet.len()]);
            }
            for value in [String::from_utf8_lossy(&bytes).into_owned(), text] {
                let blocks = parse(&value);
                let _ = is_jumbo(&blocks);
                let preview = plain_text(&blocks);
                assert!(!preview.contains('\n'), "iteration {iteration}");
            }
        }
    }

    #[test]
    fn markdown_performance_4000_char_messages() {
        for pattern in [
            "*",
            "_",
            "|",
            "`",
            "[",
            "<",
            "*_ |`[",
            "**a _b |c [",
            "[x](https://",
            "a```b```",
        ] {
            let value: String = pattern.chars().cycle().take(4000).collect();
            let started = Instant::now();
            let _ = parse(&value);
            assert!(
                started.elapsed() < Duration::from_millis(50),
                "pattern {pattern:?} took {:?}",
                started.elapsed()
            );
        }
    }
}
