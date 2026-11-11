use super::search_box::{history_chip, row_chip};
use super::*;

const POPOVER_WIDTH: f32 = 356.;
const ROW_HEIGHT: f32 = 44.;
const SHORT_ROW_HEIGHT: f32 = 28.;

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

fn title(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .child(text.into())
}

fn keyed(key: &str, value: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .min_w_0()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::muted()))
                .child(format!("{key}:")),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::faint()))
                .child(value.into()),
        )
}

fn popover_row(
    app: &DiscordApp,
    search: &MessageSearch,
    text: &str,
    history: &[SavedSearch],
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
        .min_h(rpx(SHORT_ROW_HEIGHT))
        .px(rpx(8.))
        .py(rpx(4.))
        .rounded(rpx(8.))
        .overflow_hidden()
        .cursor_pointer()
        .when(highlighted, |this| this.bg(rgba(theme::hover())))
        .hover(|style| style.bg(rgba(theme::hover())))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            choose
                .update(cx, |app, cx| {
                    app.apply_search_row(chosen.clone(), window, cx)
                })
                .ok();
        });
    let avatar = |id: u64| theme::user_avatar(app.store.user(id), id, 20., None, false, cx);
    match row {
        Row::Run => base
            .child(theme::icon("search", 20., theme::faint()))
            .child(div().flex_none().child(title("Search for")))
            .children(search.chips.iter().map(|chip| row_chip(app, chip, cx)))
            .child(title(text.trim().to_owned()).font_weight(FontWeight::SEMIBOLD)),
        Row::AddFilters => base
            .child(theme::icon("sliders-horizontal", 20., theme::faint()))
            .child(title("Add search filters")),
        Row::Filter(filter) => base
            .min_h(rpx(ROW_HEIGHT))
            .child(theme::icon(filter.icon, 20., theme::faint()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(title(filter.title))
                    .child(keyed(filter.key, filter.hint)),
            ),
        Row::MoreFilters => base
            .min_h(rpx(ROW_HEIGHT))
            .child(theme::icon("sliders-horizontal", 20., theme::faint()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(title("More filters"))
                    .child(title("dates, author type, and more").text_color(rgb(theme::faint()))),
            ),
        Row::History(index) => {
            let entry = history.get(index);
            base.child(theme::icon("search", 20., theme::faint()))
                .children(
                    entry
                        .into_iter()
                        .flat_map(|entry| &entry.filters)
                        .map(history_chip),
                )
                .children(
                    entry.map(|entry| title(entry.text.clone()).font_weight(FontWeight::SEMIBOLD)),
                )
        }
        Row::User {
            key,
            id,
            name,
            username,
            suggested: true,
        } => base
            .min_h(rpx(ROW_HEIGHT))
            .child(theme::icon(
                if key == "from" { "user" } else { "at-sign" },
                20.,
                theme::faint(),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rpx(4.))
                            .min_w_0()
                            .child(theme::user_avatar(
                                app.store.user(id),
                                id,
                                16.,
                                None,
                                false,
                                cx,
                            ))
                            .child(title(name).font_weight(FontWeight::SEMIBOLD)),
                    )
                    .child(keyed(key, username)),
            ),
        Row::User {
            id, name, username, ..
        } => base
            .child(avatar(id))
            .child(
                title(name)
                    .flex_none()
                    .max_w(rpx(180.))
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(title(username).text_color(rgb(theme::muted()))),
        Row::Channel {
            name,
            suggested: true,
            ..
        } => base
            .child(theme::icon("hash", 20., theme::faint()))
            .child(
                div()
                    .flex_none()
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::strong()))
                    .child("in:"),
            )
            .child(theme::icon("hash", 14., theme::strong()))
            .child(title(name).font_weight(FontWeight::SEMIBOLD)),
        Row::Channel { name, .. } => base
            .child(theme::icon("hash", 20., theme::faint()))
            .child(title(name)),
        Row::Has { name, .. } => base
            .child(theme::icon(has_icon(name), 20., theme::faint()))
            .child(title(name)),
    }
    .into_any_element()
}

fn divider() -> Div {
    div()
        .flex_none()
        .h(px(1.))
        .my(rpx(4.))
        .mx(rpx(-8.))
        .bg(rgba(theme::outline()))
}

fn group_header(app: &DiscordApp, heading: &'static str) -> Div {
    let clear = app.entity.clone();
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .h(rpx(24.))
        .px(rpx(8.))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::muted()))
        .child(heading)
        .when(heading == "History", |this| {
            this.child(
                theme::icon_button("search-history-clear", "trash-2", 16., theme::muted())
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        clear
                            .update(cx, |app, cx| app.clear_search_history(cx))
                            .ok();
                    }),
            )
        })
}

pub(crate) fn popover(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let search = app
        .active_search()
        .filter(|search| search.popover && search.filters.is_none())?;
    let text = search.text(cx);
    let active = search_query::active_filter(&text);
    let rows = app.search_rows(cx);
    let empty = rows.is_empty();
    let listed = active.and_then(|(key, _)| filter_key(key));
    if empty && active.is_some_and(|(_, partial)| partial.is_empty() || listed.is_none()) {
        return None;
    }
    let highlighted = search.chosen_index(cx);
    let history = if rows.iter().any(|row| matches!(row, Row::History(_))) {
        app.search_history(search.scope, cx)
    } else {
        Vec::new()
    };
    let close = app.entity.clone();
    let mut children: Vec<AnyElement> = Vec::new();
    let mut group = None;
    for (index, row) in rows.into_iter().enumerate() {
        let next = row.group();
        if index == 0 || next != group {
            if index > 0 {
                children.push(divider().into_any_element());
            }
            if let Some(heading) = next {
                children.push(group_header(app, heading).into_any_element());
            }
            group = next;
        }
        children.push(popover_row(
            app,
            search,
            &text,
            &history,
            index,
            row,
            highlighted == Some(index),
            cx,
        ));
    }
    if empty {
        if let Some(key) = listed {
            children.push(
                group_header(
                    app,
                    match key {
                        "from" => "From User",
                        "mentions" => "Mentions User",
                        "in" => "In Channel",
                        _ => "Message Contains",
                    },
                )
                .into_any_element(),
            );
        }
        children.push(
            div()
                .px(rpx(8.))
                .py(rpx(6.))
                .text_size(rpx(14.))
                .text_color(rgb(theme::muted()))
                .child("No matches")
                .into_any_element(),
        );
    }
    Some(
        div()
            .id("search-popover")
            .absolute()
            .top(rpx(titlebar::HEIGHT + chat::HEADER_HEIGHT - 1.))
            .right(rpx(12.))
            .flex()
            .flex_col()
            .gap(rpx(4.))
            .w(rpx(POPOVER_WIDTH))
            .max_h(rpx(640.))
            .px(rpx(8.))
            .py(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgb(theme::surface_high()))
            .border_1()
            .border_color(rgba(theme::outline()))
            .shadow_lg()
            .overflow_y_scroll()
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
            .children(children)
            .into_any_element(),
    )
}
