use super::*;

fn search_placeholder(app: &DiscordApp) -> String {
    let name = if app.is_home() {
        app.store.private_channel_name(app.selected_channel_id)
    } else {
        app.guild()
            .map_or_else(String::new, |guild| guild.name.clone())
    };
    format!("Search {name}")
}

pub(crate) fn search_box(app: &DiscordApp, cx: &App) -> AnyElement {
    let open = app.entity.clone();
    let clear = app.entity.clone();
    let search = app.active_search();
    let typed = search.is_some_and(|search| !search.text(cx).is_empty());
    let chips = search.map_or(&[][..], |search| &search.chips);
    let empty = !typed && chips.is_empty();
    let backspace = app.entity.clone();
    let placeholder = search_placeholder(app);
    div()
        .id("search-box")
        .flex()
        .items_center()
        .w_full()
        .h(rpx(32.))
        .rounded(rpx(8.))
        .bg(rgba(theme::recessed()))
        .border_1()
        .border_color(rgba(theme::border_emphasis()))
        .overflow_hidden()
        .cursor_text()
        .text_size(rpx(14.))
        .line_height(rpx(22.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::faint()))
        .on_click(move |_, window, cx| {
            open.update(cx, |app, cx| app.open_search(window, cx)).ok();
        })
        .capture_action(move |_: &Backspace, _, cx| {
            let removed = backspace
                .update(cx, |app, cx| app.remove_last_chip(cx))
                .unwrap_or(false);
            if removed {
                cx.stop_propagation();
            }
        })
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .px(rpx(6.))
                .children(chips.iter().flat_map(box_chip))
                .child(div().relative().flex_1().min_w_0().map(|this| {
                    let field = search.map(|search| {
                        Input::new(&search.input)
                            .appearance(false)
                            .px_0()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::text()))
                    });
                    this.when(empty, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .top_0()
                                .bottom_0()
                                .flex()
                                .items_center()
                                .child(div().min_w_0().truncate().child(placeholder)),
                        )
                    })
                    .when(search.is_none(), |this| this.h(rpx(22.)))
                    .children(field)
                })),
        )
        .map(|this| {
            if empty {
                this.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(rpx(24.))
                        .child(theme::icon("search", 16., theme::muted())),
                )
            } else {
                this.child(
                    theme::icon_button("search-clear", "x", 16., theme::muted())
                        .flex_none()
                        .size(rpx(24.))
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            clear
                                .update(cx, |app, cx| app.clear_search(window, cx))
                                .ok();
                        }),
                )
            }
        })
        .into_any_element()
}

fn box_chip(chip: &Chip) -> [Div; 2] {
    let pill = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(rpx(22.))
            .bg(rgb(theme::surface_highest()))
            .text_color(rgb(theme::strong()))
    };
    [
        pill()
            .px(rpx(4.))
            .rounded_l(rpx(4.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(format!("{}:", chip.key)),
        pill()
            .max_w(rpx(120.))
            .ml(rpx(-2.))
            .mr(rpx(2.))
            .pr(rpx(4.))
            .rounded_r(rpx(4.))
            .child(div().min_w_0().truncate().child(chip.label.clone())),
    ]
}

pub(super) fn history_chip(filter: &SavedFilter) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(24.))
        .max_w(rpx(160.))
        .mr(rpx(4.))
        .px(rpx(6.))
        .gap(rpx(4.))
        .rounded(rpx(4.))
        .bg(rgba(theme::control_selected()))
        .text_color(rgb(theme::strong()))
        .text_size(rpx(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(div().flex_none().child(format!("{}:", filter.key)))
        .child(div().min_w_0().truncate().child(filter.label.clone()))
}

pub(super) fn row_chip(app: &DiscordApp, chip: &Chip, cx: &App) -> Div {
    let user = match chip.value {
        ChipValue::User(id) => Some(id),
        _ => None,
    };
    let pill = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(rpx(24.))
            .bg(rgba(theme::control_selected()))
            .text_color(rgb(theme::strong()))
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .font_weight(FontWeight::SEMIBOLD)
        .child(
            pill()
                .pl(rpx(6.))
                .pr(rpx(2.))
                .rounded_l(rpx(4.))
                .child(format!("{}:", chip.key)),
        )
        .child(
            pill()
                .gap(rpx(4.))
                .max_w(rpx(160.))
                .pl(rpx(2.))
                .pr(rpx(6.))
                .mr(rpx(4.))
                .rounded_r(rpx(4.))
                .children(
                    user.map(|id| theme::user_avatar(app.store.user(id), id, 16., None, false, cx)),
                )
                .child(div().min_w_0().truncate().child(chip.label.clone())),
        )
}
