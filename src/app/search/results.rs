use super::*;

const RESULT_LIMIT: u64 = 10_000;
const SPINNER_SIZE: f32 = 16.;
const SORT_MENU_WIDTH: f32 = 148.;
const CARD_WIDTH: f32 = RESULTS_WIDTH - 1. - 32. - 2.;
const SORT_REOPEN_GUARD: Duration = Duration::from_millis(200);
const SORTS: [(SearchSort, &str); 3] = [
    (SearchSort::Newest, "Newest"),
    (SearchSort::Oldest, "Oldest"),
    (SearchSort::Relevant, "Most Relevant"),
];

fn sort_control(app: &DiscordApp, search: &MessageSearch, sort: SearchSort) -> impl IntoElement {
    let toggle = app.entity.clone();
    let close = app.entity.clone();
    let open = search.sort_menu;
    let option = |choice: SearchSort, text: &'static str| {
        let entity = app.entity.clone();
        div()
            .id(text)
            .flex()
            .items_center()
            .justify_between()
            .gap(rpx(12.))
            .h(rpx(36.))
            .px(rpx(8.))
            .rounded(rpx(6.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::hover())))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| {
                        if let Some(search) = app.active_search_mut() {
                            search.sort_menu = false;
                        }
                        app.set_search_sort(choice, cx);
                        cx.notify();
                    })
                    .ok();
            })
            .child(text)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(rpx(20.))
                    .rounded_full()
                    .map(|this| {
                        if choice == sort {
                            this.bg(rgb(theme::BLURPLE))
                                .child(div().size(rpx(8.)).rounded_full().bg(white()))
                        } else {
                            this.border_2().border_color(rgb(theme::muted()))
                        }
                    }),
            )
    };
    let menu = open.then(|| {
        div().absolute().top(rpx(36.)).right_0().size_0().child(
            deferred(
                anchored().anchor(Anchor::TopRight).child(
                    div()
                        .id("search-sort-menu")
                        .flex()
                        .flex_col()
                        .w(rpx(SORT_MENU_WIDTH))
                        .p(rpx(8.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::surface_high()))
                        .border_1()
                        .border_color(rgba(theme::outline()))
                        .shadow_lg()
                        .occlude()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .on_mouse_down_out(move |_, _, cx| {
                            close
                                .update(cx, |app, cx| {
                                    if let Some(search) = app.active_search_mut() {
                                        search.sort_menu = false;
                                        search.sort_closed = Some(Instant::now());
                                        cx.notify();
                                    }
                                })
                                .ok();
                        })
                        .children(SORTS.into_iter().map(|(choice, text)| option(choice, text))),
                ),
            )
            .with_priority(1),
        )
    });
    div().relative().flex_none().children(menu).child(
        div()
            .id("search-sort")
            .flex()
            .items_center()
            .gap(rpx(6.))
            .h(rpx(32.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgba(theme::control_bg()))
            .border_1()
            .border_color(rgba(theme::border_soft()))
            .cursor_pointer()
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::strong()))
            .on_click(move |_, _, cx| {
                toggle
                    .update(cx, |app, cx| {
                        if let Some(search) = app.active_search_mut() {
                            let guarded = search
                                .sort_closed
                                .is_some_and(|at| at.elapsed() < SORT_REOPEN_GUARD);
                            search.sort_menu = !guarded && !search.sort_menu;
                            cx.notify();
                        }
                    })
                    .ok();
            })
            .child(theme::icon("arrow-up-down", 16., theme::strong()))
            .child("Sort"),
    )
}

fn with_commas(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn note(text: String) -> Div {
    div()
        .py(rpx(32.))
        .px(rpx(16.))
        .text_center()
        .text_size(rpx(14.))
        .text_color(rgb(theme::muted()))
        .child(text)
}

fn result_card(
    app: &DiscordApp,
    channel_id: u64,
    message: &Message,
    terms: &Rc<[String]>,
    cx: &App,
) -> impl IntoElement {
    let open = app.entity.clone();
    let id = message.id;
    let group = SharedString::from(format!("search-result-{id}"));
    div()
        .id(("search-result", id))
        .group(group.clone())
        .relative()
        .flex_none()
        .mb(rpx(8.))
        .py(rpx(6.))
        .rounded(rpx(8.))
        .bg(rgb(theme::bg()))
        .border_1()
        .border_color(rgba(theme::outline()))
        .overflow_hidden()
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            open.update(cx, |app, cx| {
                app.open_search_result(channel_id, id, window, cx)
            })
            .ok();
        })
        .child(render_search_message(
            app,
            message,
            CARD_WIDTH,
            terms.clone(),
            cx,
        ))
        .child(
            div()
                .absolute()
                .top(rpx(8.))
                .right(rpx(8.))
                .flex()
                .items_center()
                .h(rpx(24.))
                .p(rpx(4.))
                .rounded(rpx(3.))
                .bg(rgb(theme::bg()))
                .opacity(0.)
                .group_hover(group, |style| style.opacity(1.))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::text()))
                .child("Jump"),
        )
}

fn channel_heading(app: &DiscordApp, channel_id: u64, key: u64) -> AnyElement {
    let name = app.store.channel_name(channel_id).unwrap_or_default();
    let category = app.guild().and_then(|guild| {
        guild
            .categories
            .iter()
            .find(|category| {
                category
                    .channels
                    .iter()
                    .any(|channel| channel.id == channel_id)
            })
            .map(|category| category.name.clone())
            .filter(|name| !name.is_empty())
    });
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(20.))
        .mb(rpx(8.))
        .min_w_0()
        .child(theme::icon("hash", 16., theme::strong()).mr(rpx(4.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .child(label(("search-channel", key), name)),
        )
        .children(category.map(|category| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .max_w(rpx(160.))
                .gap(rpx(4.))
                .ml(rpx(4.))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::text()))
                .child(theme::icon("chevron-right", 12., theme::text()))
                .child(div().min_w_0().truncate().child(category))
        }))
        .into_any_element()
}

fn pager_slots(page: u64, pages: u64) -> Vec<Option<u64>> {
    if pages <= 5 {
        return (1..=pages).map(Some).collect();
    }
    if page <= 3 {
        vec![Some(1), Some(2), Some(3), None, Some(pages)]
    } else if page + 2 >= pages {
        vec![Some(1), None, Some(pages - 2), Some(pages - 1), Some(pages)]
    } else {
        vec![Some(1), None, Some(page), None, Some(pages)]
    }
}

fn pager(app: &DiscordApp, page: u64, pages: u64) -> Div {
    let step = |key: &'static str, text: &'static str, target: u64, enabled: bool, back: bool| {
        let entity = app.entity.clone();
        let color = if enabled {
            theme::strong()
        } else {
            theme::faint()
        };
        let chevron = || {
            theme::icon(
                if back {
                    "chevron-left"
                } else {
                    "chevron-right"
                },
                16.,
                color,
            )
        };
        div()
            .id(key)
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(4.))
            .h(rpx(40.))
            .px(rpx(12.))
            .rounded(rpx(8.))
            .text_size(rpx(16.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(color))
            .when(back, |this| this.child(chevron()))
            .child(text)
            .when(!back, |this| this.child(chevron()))
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(rgba(theme::hover())))
                    .on_click(move |_, _, cx| {
                        entity
                            .update(cx, |app, cx| app.set_search_page(target, cx))
                            .ok();
                    })
            })
    };
    let number = |slot: usize, number: u64| {
        let entity = app.entity.clone();
        let current = number == page;
        div()
            .id(("search-page", slot))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(rpx(28.))
            .min_w(rpx(28.))
            .px(rpx(6.))
            .rounded_full()
            .text_size(rpx(16.))
            .font_weight(FontWeight::SEMIBOLD)
            .map(|this| {
                if current {
                    this.bg(rgb(theme::BLURPLE)).text_color(white())
                } else {
                    this.text_color(rgb(theme::strong()))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgba(theme::hover())))
                        .on_click(move |_, _, cx| {
                            entity
                                .update(cx, |app, cx| app.set_search_page(number, cx))
                                .ok();
                        })
                }
            })
            .child(number.to_string())
            .into_any_element()
    };
    let gap = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(rpx(36.))
            .h(rpx(44.))
            .text_size(rpx(16.))
            .text_color(rgb(theme::strong()))
            .child("\u{2026}")
            .into_any_element()
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .h(rpx(57.))
        .px(rpx(16.))
        .child(step(
            "search-back",
            "Previous",
            page.saturating_sub(1),
            page > 1,
            true,
        ))
        .child(div().flex().items_center().gap(rpx(4.)).children(
            pager_slots(page, pages).into_iter().enumerate().map(
                |(slot, number_at)| match number_at {
                    Some(value) => number(slot, value),
                    None => gap(),
                },
            ),
        ))
        .child(step("search-next", "Next", page + 1, page < pages, false))
}

fn filters_button(app: &DiscordApp, count: usize) -> impl IntoElement {
    let open = app.entity.clone();
    div()
        .id("search-filters")
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(32.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .bg(rgba(theme::control_bg()))
        .border_1()
        .border_color(rgba(theme::border_soft()))
        .cursor_pointer()
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .on_click(move |_, window, cx| {
            open.update(cx, |app, cx| app.open_search_filters(window, cx))
                .ok();
        })
        .child(theme::icon("sliders-horizontal", 16., theme::strong()))
        .child(if count > 0 {
            format!("Filters ({count})")
        } else {
            "Filters".to_owned()
        })
}

pub(crate) fn render_results(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let search = app.active_search()?;
    let results = search.results.as_ref()?;
    let close = app.entity.clone();
    let heading = match results.status {
        Status::Loading => "Searching\u{2026}".to_owned(),
        Status::Indexing => "Indexing\u{2026}".to_owned(),
        _ if results.total == 1 => "1 Result".to_owned(),
        _ => format!("{} Results", with_commas(results.total)),
    };
    let busy = matches!(results.status, Status::Loading | Status::Indexing);
    let sort = results.query.sort;
    let show_channels = matches!(search.scope, SearchScope::Guild(_));
    let mut groups: Vec<(u64, Vec<&Message>)> = Vec::new();
    for (channel_id, message) in &results.messages {
        match groups.last_mut() {
            Some((channel, messages)) if channel == channel_id => messages.push(message),
            _ => groups.push((*channel_id, vec![message])),
        }
    }
    let list = groups.into_iter().map(|(channel_id, messages)| {
        let key = messages.first().map_or(channel_id, |message| message.id);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .mb(rpx(24.))
            .when(show_channels, |this| {
                this.child(channel_heading(app, channel_id, key))
            })
            .children(
                messages
                    .into_iter()
                    .map(|message| result_card(app, channel_id, message, &results.terms, cx)),
            )
    });
    let body = match &results.status {
        Status::Failed(error) => Some(note(error.clone())),
        Status::Ready if results.messages.is_empty() => Some(note("No results found.".to_owned())),
        _ => None,
    };
    let page = (results.query.offset / PAGE) as u64 + 1;
    let pages = results.total.min(RESULT_LIMIT).div_ceil(PAGE as u64);
    Some(
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(theme::bg()))
            .border_l_1()
            .border_color(rgba(theme::border()))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(rpx(8.))
                    .h(rpx(48.))
                    .px(rpx(16.))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .gap(rpx(6.))
                            .min_w_0()
                            .text_size(rpx(16.))
                            .line_height(rpx(20.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::strong()))
                            .child(div().min_w_0().truncate().child(heading))
                            .when(busy, |this| {
                                this.child(theme::spinner(SPINNER_SIZE, theme::muted()))
                            }),
                    )
                    .child(filters_button(app, filter_count(search, cx)))
                    .child(sort_control(app, search, sort))
                    .child(
                        theme::icon_button("search-close", "x", 20., theme::muted()).on_click(
                            move |_, _, cx| {
                                close
                                    .update(cx, |app, cx| {
                                        if let Some(search) = app.active_search_mut() {
                                            search.popover = false;
                                            search.results = None;
                                            cx.notify();
                                        }
                                    })
                                    .ok();
                            },
                        ),
                    ),
            )
            .child(
                div()
                    .id(("search-results", page))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .px(rpx(16.))
                    .pt(rpx(16.))
                    .children(list)
                    .children(body),
            )
            .when(pages > 1, |this| this.child(pager(app, page, pages)))
            .into_any_element(),
    )
}
