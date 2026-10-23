use std::collections::HashSet;

use chrono::{Local, TimeZone};
use gpui::{prelude::*, *};
use gpui_component::ActiveTheme;

use crate::{
    app::{
        DiscordApp,
        rich_text::{Action, Chip, Content, Piece, PieceStyle, rich_text},
    },
    emoji::{self, Segment},
    images::{self, ImageSpec},
    markdown::{Block, Span, SpanKind, Style},
    store::Store,
    theme::{self, rpx},
};

#[derive(Clone, Copy)]
pub struct BodyColors {
    pub strong: u32,
    pub muted: u32,
    pub faint: u32,
    pub link: Option<u32>,
}

impl Default for BodyColors {
    fn default() -> Self {
        Self {
            strong: theme::STRONG,
            muted: theme::MUTED,
            faint: theme::FAINT,
            link: None,
        }
    }
}

pub struct BodyContext<'a> {
    pub store: &'a Store,
    pub guild_id: u64,
    pub message_id: u64,
    pub revealed: &'a HashSet<(u64, u16)>,
    pub entity: WeakEntity<DiscordApp>,
    pub colors: BodyColors,
}

impl BodyContext<'_> {
    fn handler(&self) -> impl Fn(&Action, &mut Window, &mut App) + 'static {
        let entity = self.entity.clone();
        let message_id = self.message_id;
        move |action, _, cx| match action {
            Action::OpenUrl(url) => cx.open_url(url),
            Action::RevealSpoiler(index) => {
                entity
                    .update(cx, |app, cx| app.reveal_spoiler(message_id, *index, cx))
                    .ok();
            }
        }
    }
}

pub fn render_body(
    blocks: &[Block],
    edited: bool,
    jumbo: bool,
    context: &BodyContext,
    cx: &App,
) -> AnyElement {
    let mut counter = 0;
    let mut children: Vec<AnyElement> = Vec::new();
    let last = blocks.len().saturating_sub(1);
    for (index, block) in blocks.iter().enumerate() {
        let edited_here = edited && index == last;
        children.push(render_block(
            block,
            index == 0,
            edited_here,
            jumbo,
            0,
            context,
            &mut counter,
            cx,
        ));
    }
    if edited && blocks.is_empty() {
        children.push(
            div()
                .text_size(rpx(10.))
                .text_color(rgb(context.colors.faint))
                .child("(edited)")
                .into_any_element(),
        );
    }
    div()
        .flex()
        .flex_col()
        .w_full()
        .min_w_0()
        .children(children)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_block(
    block: &Block,
    first: bool,
    edited: bool,
    jumbo: bool,
    depth: usize,
    context: &BodyContext,
    counter: &mut u32,
    cx: &App,
) -> AnyElement {
    match block {
        Block::Paragraph(spans) => {
            let text = inline(spans, edited, jumbo, context, counter, cx);
            if jumbo {
                text.emoji_size(rpx(48.))
                    .line_height(rpx(54.))
                    .into_any_element()
            } else {
                text.into_any_element()
            }
        }
        Block::Heading { level, spans } => {
            let size = match level {
                1 => 24.,
                2 => 20.,
                _ => 16.,
            };
            div()
                .when(!first, |this| this.mt(rpx(8.)))
                .mb(rpx(4.))
                .text_size(rpx(size))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(context.colors.strong))
                .child(
                    inline(spans, edited, false, context, counter, cx)
                        .line_height(rpx(size * 1.375)),
                )
                .into_any_element()
        }
        Block::Subtext(spans) => div()
            .text_size(rpx(13.))
            .text_color(rgb(context.colors.muted))
            .child(inline(spans, edited, false, context, counter, cx).line_height(rpx(18.)))
            .into_any_element(),
        Block::Quote(blocks) => div()
            .flex()
            .w_full()
            .child(
                div()
                    .w(rpx(4.))
                    .flex_none()
                    .rounded(rpx(4.))
                    .bg(rgb(0x4E5058)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .pl(rpx(12.))
                    .children(blocks.iter().enumerate().map(|(index, block)| {
                        let last = index + 1 == blocks.len();
                        render_block(
                            block,
                            index == 0,
                            edited && last,
                            false,
                            depth,
                            context,
                            counter,
                            cx,
                        )
                    })),
            )
            .into_any_element(),
        Block::CodeBlock { code, .. } => div()
            .mt(rpx(4.))
            .w_full()
            .p(rpx(8.))
            .rounded(rpx(4.))
            .bg(rgb(theme::ELEVATED))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .font_family(cx.theme().mono_font_family.clone())
            .text_color(rgb(theme::TEXT))
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .whitespace_normal()
            .child({
                *counter += 1;
                rich_text(
                    ("code-body", (context.message_id << 8) ^ u64::from(*counter)),
                    vec![Piece::text(code.clone(), PieceStyle::default())],
                )
                .selectable()
                .line_height(rpx(18.))
            })
            .when(edited, |this| {
                this.child(
                    div()
                        .text_size(rpx(10.))
                        .text_color(rgb(context.colors.faint))
                        .child("(edited)"),
                )
            })
            .into_any_element(),
        Block::List { start, items } => div()
            .flex()
            .flex_col()
            .w_full()
            .children(items.iter().enumerate().map(|(index, item)| {
                let marker = match start {
                    Some(start) => format!("{}.", start + index as u64),
                    None => match depth {
                        0 => "•",
                        1 => "◦",
                        _ => "▪",
                    }
                    .into(),
                };
                let last_item = index + 1 == items.len();
                div()
                    .flex()
                    .w_full()
                    .child(
                        div()
                            .flex_none()
                            .w(rpx(24.))
                            .pr(rpx(6.))
                            .flex()
                            .justify_end()
                            .line_height(rpx(22.))
                            .child(marker),
                    )
                    .child(div().flex().flex_col().flex_1().min_w_0().children(
                        item.iter().enumerate().map(|(block_index, block)| {
                            render_block(
                                block,
                                true,
                                edited && last_item && block_index + 1 == item.len(),
                                false,
                                depth + 1,
                                context,
                                counter,
                                cx,
                            )
                        }),
                    ))
            }))
            .into_any_element(),
    }
}

fn inline(
    spans: &[Span],
    edited: bool,
    jumbo: bool,
    context: &BodyContext,
    counter: &mut u32,
    cx: &App,
) -> crate::app::rich_text::RichText {
    *counter += 1;
    let id = ("body", (context.message_id << 8) ^ u64::from(*counter));
    let mut pieces = spans_to_pieces(spans, jumbo, context, counter, cx);
    if edited {
        pieces.push(Piece::text(
            " (edited)",
            PieceStyle {
                scale: Some(0.625),
                color: Some(rgb(context.colors.faint).into()),
                nowrap: true,
                ..Default::default()
            },
        ));
    }
    rich_text(id, pieces)
        .selectable()
        .on_action(context.handler())
}

pub fn spans_to_pieces(
    spans: &[Span],
    jumbo: bool,
    context: &BodyContext,
    counter: &mut u32,
    cx: &App,
) -> Vec<Piece> {
    spans
        .iter()
        .flat_map(|span| {
            *counter += 1;
            span_pieces(span, jumbo, context, *counter, cx)
        })
        .collect()
}

fn base_style(style: &Style, link: Option<&str>, context: &BodyContext) -> PieceStyle {
    let revealed = style
        .spoiler
        .is_none_or(|index| context.revealed.contains(&(context.message_id, index)));
    let action = match (style.spoiler, revealed, link) {
        (Some(index), false, _) => Some(Action::RevealSpoiler(index)),
        (_, _, Some(url)) => Some(Action::OpenUrl(url.to_owned().into())),
        _ => None,
    };
    PieceStyle {
        color: link.and_then(|_| context.colors.link.map(|color| rgb(color).into())),
        weight: style.bold.then_some(FontWeight::BOLD),
        italic: style.italic,
        underline: style.underline,
        strikethrough: style.strikethrough,
        chip: style.spoiler.map(|_| Chip::Spoiler { revealed }),
        group: style.spoiler.map_or(0, u32::from),
        action,
        ..Default::default()
    }
}

pub fn emoji_pieces(text: &str, style: &PieceStyle) -> Vec<Piece> {
    emoji::segments(text)
        .into_iter()
        .map(|segment| match segment {
            Segment::Emoji(sequence) => match emoji::image(sequence) {
                Some(image) => Piece {
                    content: Content::Emoji(Some(image.into()), sequence.to_owned().into()),
                    style: style.clone(),
                },
                None => Piece::text(sequence.to_owned(), style.clone()),
            },
            Segment::Text(text) => Piece::text(text.to_owned(), style.clone()),
        })
        .flat_map(|piece| match &piece.content {
            Content::Text(text) if text.chars().any(box_drawing) => cell_pieces(text, &piece.style),
            _ => vec![piece],
        })
        .collect()
}

fn box_drawing(c: char) -> bool {
    matches!(c, '\u{2500}'..='\u{257F}')
}

fn cell_pieces(text: &str, style: &PieceStyle) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut start = 0;
    for (index, c) in text.char_indices() {
        if !box_drawing(c) {
            continue;
        }
        if index > start {
            pieces.push(Piece::text(text[start..index].to_owned(), style.clone()));
        }
        let end = index + c.len_utf8();
        pieces.push(Piece::text(
            text[index..end].to_owned(),
            PieceStyle {
                cell: true,
                nowrap: true,
                ..style.clone()
            },
        ));
        start = end;
    }
    if start < text.len() {
        pieces.push(Piece::text(text[start..].to_owned(), style.clone()));
    }
    pieces
}

fn has_emoji(text: &str) -> bool {
    text.chars().any(box_drawing)
        || emoji::segments(text)
            .iter()
            .any(|segment| matches!(segment, Segment::Emoji(_)))
}

pub fn label(id: impl Into<ElementId>, text: &str) -> AnyElement {
    if !has_emoji(text) {
        return SharedString::from(text.to_owned()).into_any_element();
    }
    rich_text(id, emoji_pieces(text, &PieceStyle::default()))
        .emoji_scale(1.125)
        .truncate()
        .into_any_element()
}

pub fn wrapped_label(id: impl Into<ElementId>, text: &str) -> AnyElement {
    if !has_emoji(text) {
        return SharedString::from(text.to_owned()).into_any_element();
    }
    rich_text(id, emoji_pieces(text, &PieceStyle::default()))
        .emoji_scale(1.125)
        .into_any_element()
}

fn span_pieces(
    span: &Span,
    jumbo: bool,
    context: &BodyContext,
    unique: u32,
    cx: &App,
) -> Vec<Piece> {
    let mut style = base_style(&span.style, span.link.as_deref(), context);
    let hidden = matches!(style.chip, Some(Chip::Spoiler { revealed: false }));
    let chip = |chip: Chip, style: &mut PieceStyle| {
        if !hidden {
            style.chip = Some(chip);
            style.group = 0x8000_0000 | unique;
        }
        style.nowrap = true;
    };
    let store = context.store;
    let text: String = match &span.kind {
        SpanKind::Text(text) => {
            if jumbo {
                style.scale = Some(2.6);
            }
            return emoji_pieces(text, &style);
        }
        SpanKind::Code(code) => {
            style.mono = true;
            style.scale = Some(0.85);
            if !hidden {
                style.chip = Some(Chip::Code);
                style.color = Some(rgb(theme::TEXT).into());
                style.group = 0x8000_0000 | unique;
            }
            code.clone()
        }
        SpanKind::UserMention(id) => {
            chip(Chip::Mention { color: None }, &mut style);
            let name = store
                .member_name(context.guild_id, *id)
                .unwrap_or("unknown-user");
            format!("@{name}")
        }
        SpanKind::RoleMention(id) => match store.role(context.guild_id, *id) {
            Some(role) => {
                chip(Chip::Mention { color: role.color }, &mut style);
                format!("@{}", role.name)
            }
            None => {
                chip(Chip::Mention { color: None }, &mut style);
                "@deleted-role".into()
            }
        },
        SpanKind::ChannelMention(id) => {
            chip(Chip::Mention { color: None }, &mut style);
            format!("#{}", store.channel_name(*id).unwrap_or("unknown"))
        }
        SpanKind::Everyone => {
            chip(Chip::Mention { color: None }, &mut style);
            "@everyone".into()
        }
        SpanKind::Here => {
            chip(Chip::Mention { color: None }, &mut style);
            "@here".into()
        }
        SpanKind::Command { name, .. } => {
            chip(Chip::Mention { color: None }, &mut style);
            format!("/{name}")
        }
        SpanKind::Timestamp { unix, style: kind } => {
            chip(Chip::Timestamp, &mut style);
            format_timestamp(*unix, *kind)
        }
        SpanKind::Emoji { id, animated, name } => {
            let size = if jumbo { 48. } else { 22. };
            let image = images::resolve(
                &ImageSpec::Emoji {
                    id: *id,
                    animated: *animated,
                },
                theme::image_px(size),
                cx,
            );
            return vec![Piece {
                content: Content::Emoji(image.map(Into::into), format!(":{name}:").into()),
                style,
            }];
        }
    };
    vec![Piece::text(text, style)]
}

pub fn format_timestamp(unix: i64, style: char) -> String {
    let Some(time) = Local.timestamp_opt(unix, 0).single() else {
        return unix.to_string();
    };
    match style {
        't' => time.format("%-I:%M %p").to_string(),
        'T' => time.format("%-I:%M:%S %p").to_string(),
        'd' => time.format("%-m/%-d/%Y").to_string(),
        'D' => time.format("%B %-d, %Y").to_string(),
        'F' => time.format("%A, %B %-d, %Y %-I:%M %p").to_string(),
        'R' => relative(unix - Local::now().timestamp()),
        _ => time.format("%B %-d, %Y %-I:%M %p").to_string(),
    }
}

fn relative(delta: i64) -> String {
    let seconds = delta.unsigned_abs();
    let (amount, unit) = match seconds {
        0..60 => (seconds, "second"),
        60..3600 => (seconds / 60, "minute"),
        3600..86_400 => (seconds / 3600, "hour"),
        86_400..2_592_000 => (seconds / 86_400, "day"),
        2_592_000..31_536_000 => (seconds / 2_592_000, "month"),
        _ => (seconds / 31_536_000, "year"),
    };
    let unit = if amount == 1 {
        unit.to_owned()
    } else {
        format!("{unit}s")
    };
    if delta >= 0 {
        format!("in {amount} {unit}")
    } else {
        format!("{amount} {unit} ago")
    }
}

pub fn preview_pieces(
    blocks: &[Block],
    context: &BodyContext,
    counter: &mut u32,
    cx: &App,
) -> Vec<Piece> {
    let mut spans: Vec<Span> = Vec::new();
    collect_inline(blocks, &mut spans);
    let mut pieces = spans_to_pieces(&spans, false, context, counter, cx);
    for piece in &mut pieces {
        if let Content::Text(text) = &piece.content
            && text.contains('\n')
        {
            piece.content = Content::Text(text.replace('\n', " ").into());
        }
        piece.style.nowrap = true;
    }
    pieces
}

fn collect_inline(blocks: &[Block], spans: &mut Vec<Span>) {
    let space = || Span {
        kind: SpanKind::Text(" ".into()),
        style: Style::default(),
        link: None,
    };
    for block in blocks {
        if !spans.is_empty() {
            spans.push(space());
        }
        match block {
            Block::Paragraph(inner) | Block::Subtext(inner) => spans.extend(inner.iter().cloned()),
            Block::Heading { spans: inner, .. } => spans.extend(inner.iter().cloned()),
            Block::Quote(inner) => collect_inline(inner, spans),
            Block::List { items, .. } => {
                for item in items {
                    collect_inline(item, spans);
                }
            }
            Block::CodeBlock { code, .. } => spans.push(Span {
                kind: SpanKind::Code(code.clone()),
                style: Style::default(),
                link: None,
            }),
        }
    }
}
