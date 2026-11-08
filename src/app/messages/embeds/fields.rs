use super::*;

const FIELD_GAP: f32 = 8.;
const FIELD_TRACKS: usize = 12;

pub(super) fn fields(
    app: &DiscordApp,
    fields: &[EmbedField],
    key: impl Fn(usize) -> u64,
    name_key: impl Fn(usize) -> u64,
    cx: &App,
) -> Div {
    let widths: Vec<f32> = fields.iter().map(|field| field_width(field, cx)).collect();
    field_grid(fields, &widths, |index, field| {
        div()
            .flex()
            .flex_col()
            .gap(rpx(2.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .child(markdown(app, name_key(index), &field.name, 14., 18., cx)),
            )
            .child(markdown(app, key(index), &field.value, 14., 18., cx))
            .into_any_element()
    })
}

fn field_rows(fields: &[EmbedField]) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        match rows.last_mut() {
            Some(row) if field.inline && row.len() < 3 && fields[row[0]].inline => row.push(index),
            _ => rows.push(vec![index]),
        }
    }
    rows
}

/// CSS grid sizing of twelve `auto` columns, done by hand because Taffy's grid sizes them past a
/// narrow card. Rows then grow fields in proportion to their span, which equals grid's even
/// stretch of the columns only because every row covers all twelve.
pub(super) fn field_bases(rows: &[Vec<f32>]) -> Vec<Vec<f32>> {
    let span = |row: &Vec<f32>| FIELD_TRACKS / row.len();
    let mut items: Vec<(usize, usize, f32)> = Vec::new();
    for row in rows {
        for (column, width) in row.iter().enumerate() {
            items.push((column * span(row), span(row), *width));
        }
    }
    items.sort_by_key(|(_, span, _)| *span);
    let mut tracks = [0f32; FIELD_TRACKS];
    for (start, span, width) in items {
        let spanned = start..start + span;
        let current = tracks[spanned.clone()].iter().sum::<f32>() + FIELD_GAP * (span - 1) as f32;
        if width > current {
            let extra = (width - current) / span as f32;
            tracks[spanned].iter_mut().for_each(|track| *track += extra);
        }
    }
    rows.iter()
        .map(|row| {
            (0..row.len())
                .map(|column| {
                    let start = column * span(row);
                    tracks[start..start + span(row)].iter().sum::<f32>()
                        + FIELD_GAP * (span(row) - 1) as f32
                })
                .collect()
        })
        .collect()
}

fn field_width(field: &EmbedField, cx: &App) -> f32 {
    let widest = |text: &str, weight| {
        text.lines()
            .map(|line| text_width(line, weight, cx))
            .fold(0., f32::max)
    };
    widest(&field.name, FontWeight::SEMIBOLD).max(widest(&field.value, FontWeight::NORMAL))
}

/// Markdown markers count toward the width, so this runs a little wide for styled values.
fn text_width(text: &str, weight: FontWeight, cx: &App) -> f32 {
    thread_local! {
        static WIDTHS: std::cell::RefCell<std::collections::HashMap<(String, u16), f32>> =
            Default::default();
    }
    let key = (text.to_owned(), weight.0 as u16);
    if let Some(width) = WIDTHS.with(|widths| widths.borrow().get(&key).copied()) {
        return width;
    }
    let mut font = font(gpui_component::ActiveTheme::theme(cx).font_family.clone());
    font.weight = weight;
    let text_system = cx.text_system();
    let id = text_system.resolve_font(&font);
    let width = text
        .chars()
        .map(|ch| f32::from(text_system.layout_width(id, px(14.), ch)))
        .sum();
    WIDTHS.with(|widths| {
        let mut widths = widths.borrow_mut();
        if widths.len() > 2048 {
            widths.clear();
        }
        widths.insert(key, width);
    });
    width
}

pub(super) fn field_grid(
    fields: &[EmbedField],
    widths: &[f32],
    render: impl Fn(usize, &EmbedField) -> AnyElement,
) -> Div {
    let rows = field_rows(fields);
    let bases = field_bases(
        &rows
            .iter()
            .map(|row| row.iter().map(|&index| widths[index]).collect())
            .collect::<Vec<_>>(),
    );
    div()
        .flex()
        .flex_col()
        .gap(rpx(FIELD_GAP))
        .children(rows.into_iter().zip(bases).map(|(row, bases)| {
            let span = (FIELD_TRACKS / row.len()) as f32;
            div()
                .flex()
                .gap(rpx(FIELD_GAP))
                .min_w_0()
                .children(row.into_iter().zip(bases).map(|(index, basis)| {
                    div()
                        .flex_basis(rpx(basis))
                        .flex_grow(span)
                        .flex_shrink_1()
                        .min_w_0()
                        .child(render(index, &fields[index]))
                }))
        }))
}
