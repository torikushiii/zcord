use super::search_box::row_chip;
use super::*;

const POPOVER_WIDTH: f32 = 356.;
const ROW_HEIGHT: f32 = 44.;

fn search_row(
    app: &DiscordApp,
    search: &MessageSearch,
    text: String,
    cx: &App,
) -> impl IntoElement {
    let run = app.entity.clone();
    div()
        .id("search-run")
        .flex()
        .flex_none()
        .items_center()
        .min_h(rpx(32.))
        .px(rpx(8.))
        .py(rpx(4.))
        .rounded(rpx(8.))
        .overflow_hidden()
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::hover())))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .on_click(move |_, _, cx| {
            run.update(cx, |app, cx| app.run_search(cx)).ok();
        })
        .child(theme::icon("search", 20., theme::faint()).mr(rpx(8.)))
        .child(div().flex_none().mr(rpx(4.)).child("Search for"))
        .children(search.chips.iter().map(|filter| row_chip(app, filter, cx)))
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .child(text),
        )
}

fn has_icon(name: &str) -> &'static str {
    match name {
        "link" => "link",
        "embed" => "app-window",
        "poll" => "chart-column",
        "video" => "video",
        "image" => "image",
        "sound" => "volume-2",
        "sticker" => "sticker",
        "forward" => "forward",
        _ => "file",
    }
}

fn popover_row(
    app: &DiscordApp,
    index: usize,
    row: Row,
    highlighted: bool,
    cx: &App,
) -> AnyElement {
    let choose = app.entity.clone();
    let chosen = row.clone();
    let base = div()
        .id(("search-row", index))
        .flex()
        .items_center()
        .gap(rpx(8.))
        .min_h(rpx(32.))
        .px(rpx(8.))
        .py(rpx(4.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .when(highlighted, |this| this.bg(rgba(theme::selected())))
        .hover(|style| style.bg(rgba(theme::hover())))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            choose
                .update(cx, |app, cx| {
                    app.apply_search_row(chosen.clone(), window, cx)
                })
                .ok();
        });
    let title = |text: String| {
        div()
            .min_w_0()
            .truncate()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::strong()))
            .child(text)
    };
    match row {
        Row::Filter(filter) => base
            .min_h(rpx(ROW_HEIGHT))
            .child(theme::icon(filter.icon, 20., theme::faint()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(title(filter.title.to_owned()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rpx(4.))
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::faint()))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::muted()))
                                    .child(format!("{}:", filter.key)),
                            )
                            .child(filter.hint),
                    ),
            ),
        Row::User {
            id, name, username, ..
        } => base
            .child(theme::user_avatar(
                app.store.user(id),
                id,
                24.,
                None,
                false,
                cx,
            ))
            .child(title(name))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(rpx(12.))
                    .text_color(rgb(theme::muted()))
                    .child(username),
            ),
        Row::Channel { name, .. } => base
            .child(theme::icon("hash", 20., theme::muted()))
            .child(title(name)),
        Row::Has { name, .. } => base
            .child(theme::icon(has_icon(name), 20., theme::faint()))
            .child(title(name.to_owned())),
    }
    .into_any_element()
}

pub(crate) fn popover(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let search = app.active_search().filter(|search| search.popover)?;
    let text = search.text(cx);
    let active = search_query::active_filter(&text);
    // Date and `pinned:` filters are typed by hand, so they have nothing to suggest.
    if active.is_some_and(|(key, _)| filter_key(key).is_none()) {
        return None;
    }
    let heading = match active.and_then(|(key, _)| filter_key(key)) {
        Some("from") => "From User",
        Some("mentions") => "Mentions User",
        Some("in") => "In Channel",
        Some("has") => "Has",
        _ => "Filters",
    };
    let rows = app.search_rows(cx);
    let close = app.entity.clone();
    let empty = rows.is_empty();
    let ready = active.is_none() && (!text.trim().is_empty() || !search.chips.is_empty());
    Some(
        div()
            .id("search-popover")
            .absolute()
            .top(rpx(titlebar::HEIGHT + chat::HEADER_HEIGHT - 1.))
            .right(rpx(12.))
            .flex()
            .flex_col()
            .w(rpx(POPOVER_WIDTH))
            .px(rpx(8.))
            .py(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgb(theme::surface_high()))
            .border_1()
            .border_color(rgba(theme::outline()))
            .shadow_lg()
            .occlude()
            .without_text_selection()
            .on_mouse_down_out(move |_, _, cx| {
                close
                    .update(cx, |app, cx| {
                        if let Some(search) = app.active_search_mut() {
                            search.popover = false;
                            cx.notify();
                        }
                    })
                    .ok();
            })
            .when(ready, |this| {
                this.child(search_row(app, search, text.trim().to_owned(), cx))
                    .child(
                        div()
                            .h(px(1.))
                            .my(rpx(8.))
                            .mx(rpx(-8.))
                            .bg(rgba(theme::outline())),
                    )
            })
            .child(
                div()
                    .px(rpx(8.))
                    .py(rpx(4.))
                    .mb(rpx(4.))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::muted()))
                    .child(heading),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rpx(4.))
                    .children(rows.into_iter().enumerate().map(|(index, row)| {
                        popover_row(app, index, row, search.highlighted == Some(index), cx)
                    })),
            )
            .when(empty, |this| {
                this.child(
                    div()
                        .px(rpx(8.))
                        .py(rpx(6.))
                        .text_size(rpx(14.))
                        .text_color(rgb(theme::muted()))
                        .child("No matches"),
                )
            })
            .into_any_element(),
    )
}
