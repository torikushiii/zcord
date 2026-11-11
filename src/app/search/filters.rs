use gpui_component::calendar::Date;
use gpui_component::date_picker::{DatePicker, DatePickerState};

use super::*;

const WIDTH: f32 = 480.;
const BODY_HEIGHT: f32 = 480.;
const OPTIONS_HEIGHT: f32 = 220.;
const OPTION_LIMIT: usize = 25;
const CHECK_SIZE: f32 = 20.;
const DATE_KINDS: [(&str, &str); 3] = [
    ("before", "Before"),
    ("after", "After"),
    ("during", "During"),
];

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Field {
    From,
    In,
    Has,
    Mentions,
    Author,
    Pinned,
    DateKind(usize),
}

impl Field {
    fn searchable(self) -> bool {
        matches!(self, Field::From | Field::In | Field::Mentions)
    }
}

struct DateRow {
    key: &'static str,
    picker: Entity<DatePickerState>,
}

pub(super) struct FiltersDialog {
    from: Vec<(u64, String)>,
    channels: Vec<(u64, String)>,
    has: Vec<&'static str>,
    mentions: Vec<(u64, String)>,
    dates: Vec<DateRow>,
    authors: Vec<&'static str>,
    pinned: Option<bool>,
    open: Option<Field>,
    query: Entity<InputState>,
    initial: Vec<Chip>,
    _query_events: Subscription,
}

fn has_name(api: &str) -> &'static str {
    HAS_OPTIONS
        .iter()
        .find(|(_, known)| *known == api)
        .map_or("file", |(name, _)| *name)
}

impl FiltersDialog {
    fn chips(&self, cx: &App) -> Vec<Chip> {
        let users = |key: &'static str, users: &[(u64, String)]| {
            users
                .iter()
                .map(|(id, label)| Chip {
                    key,
                    label: label.clone(),
                    value: ChipValue::User(*id),
                })
                .collect::<Vec<_>>()
        };
        let mut chips = users("from", &self.from);
        chips.extend(self.channels.iter().map(|(id, name)| Chip {
            key: "in",
            label: name.clone(),
            value: ChipValue::Channel(*id),
        }));
        chips.extend(self.has.iter().map(|api| Chip {
            key: "has",
            label: has_name(api).to_owned(),
            value: ChipValue::Has(api),
        }));
        chips.extend(users("mentions", &self.mentions));
        chips.extend(self.dates.iter().filter_map(|row| {
            let Date::Single(Some(date)) = row.picker.read(cx).date() else {
                return None;
            };
            Some(Chip {
                key: row.key,
                label: date.format("%Y-%m-%d").to_string(),
                value: ChipValue::Date(date),
            })
        }));
        chips.extend(self.authors.iter().map(|kind| Chip {
            key: "author",
            label: (*kind).to_owned(),
            value: ChipValue::Author(kind),
        }));
        chips.extend(self.pinned.map(|pinned| Chip {
            key: "pinned",
            label: pinned.to_string(),
            value: ChipValue::Pinned(pinned),
        }));
        chips
    }

    fn date_row(
        key: &'static str,
        date: Option<NaiveDate>,
        window: &mut Window,
        cx: &mut App,
    ) -> DateRow {
        let picker = cx.new(|cx| {
            let mut picker = DatePickerState::new(window, cx).date_format("%b %-d, %Y");
            if let Some(date) = date {
                picker.set_date(date, window, cx);
            }
            picker
        });
        DateRow { key, picker }
    }
}

fn users_of(dialog: &mut FiltersDialog, field: Field) -> &mut Vec<(u64, String)> {
    if field == Field::From {
        &mut dialog.from
    } else {
        &mut dialog.mentions
    }
}

fn toggle<T: PartialEq>(list: &mut Vec<T>, value: T) {
    match list.iter().position(|item| *item == value) {
        Some(index) => {
            list.remove(index);
        }
        None => list.push(value),
    }
}

impl DiscordApp {
    fn search_filters_mut(&mut self) -> Option<&mut FiltersDialog> {
        self.active_search_mut()?.filters.as_mut()
    }

    pub(crate) fn open_search_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(search) = self.active_search() else {
            return;
        };
        let initial = search.chips.clone();
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let events = cx.subscribe_in(&query, window, |app, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                app.search_filter_query_changed(cx);
            }
        });
        let mut dialog = FiltersDialog {
            from: Vec::new(),
            channels: Vec::new(),
            has: Vec::new(),
            mentions: Vec::new(),
            dates: Vec::new(),
            authors: Vec::new(),
            pinned: None,
            open: None,
            query,
            initial: initial.clone(),
            _query_events: events,
        };
        for chip in initial {
            match chip.value {
                ChipValue::User(id) if chip.key == "from" => dialog.from.push((id, chip.label)),
                ChipValue::User(id) => dialog.mentions.push((id, chip.label)),
                ChipValue::Channel(id) => dialog.channels.push((id, chip.label)),
                ChipValue::Has(api) => dialog.has.push(api),
                ChipValue::Date(date) => {
                    dialog
                        .dates
                        .push(FiltersDialog::date_row(chip.key, Some(date), window, cx))
                }
                ChipValue::Author(kind) => dialog.authors.push(kind),
                ChipValue::Pinned(pinned) => dialog.pinned = Some(pinned),
            }
        }
        if let Some(search) = self.active_search_mut() {
            search.popover = false;
            search.sort_menu = false;
            search.filters = Some(dialog);
        }
        cx.notify();
    }

    fn close_search_filters(&mut self, cx: &mut Context<Self>) {
        if let Some(search) = self.active_search_mut() {
            search.filters = None;
            cx.notify();
        }
    }

    fn apply_search_filters(&mut self, cx: &mut Context<Self>) {
        let Some(search) = self.active_search_mut() else {
            return;
        };
        let Some(dialog) = search.filters.take() else {
            return;
        };
        search.chips = dialog.chips(cx);
        if search.chips.is_empty() && search.text(cx).trim().is_empty() {
            search.results = None;
            cx.notify();
            return;
        }
        self.run_search(cx);
    }

    fn search_filter_query_changed(&mut self, cx: &mut Context<Self>) {
        let guild_id = match self.active_search().map(|search| search.scope) {
            Some(SearchScope::Guild(guild_id)) => Some(guild_id),
            _ => None,
        };
        let session = self.session.clone().filter(|_| self.live);
        let Some(search) = self.active_search_mut() else {
            return;
        };
        let Some(dialog) = search.filters.as_ref() else {
            return;
        };
        let query = dialog.query.read(cx).value().trim().to_owned();
        let users = matches!(dialog.open, Some(Field::From | Field::Mentions));
        search.member_query = None;
        if let (true, Some(guild_id), Some(session), false) =
            (users, guild_id, session, query.is_empty())
        {
            search.member_query = Some(cx.spawn(async move |_, cx| {
                cx.background_executor().timer(MEMBER_QUERY_DELAY).await;
                session.query_members(guild_id, &query, MEMBER_LIMIT);
            }));
        }
        cx.notify();
    }

    fn toggle_filter_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.search_filters_mut() else {
            return;
        };
        dialog.open = (dialog.open != Some(field)).then_some(field);
        if dialog.open.is_some_and(Field::searchable) {
            let query = dialog.query.clone();
            query.update(cx, |query, cx| {
                query.set_value("", window, cx);
                query.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn edit_search_filters(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut FiltersDialog),
    ) {
        if let Some(dialog) = self.search_filters_mut() {
            edit(dialog);
            cx.notify();
        }
    }

    fn add_search_date(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = FiltersDialog::date_row("before", None, window, cx);
        self.edit_search_filters(cx, |dialog| {
            dialog.dates.push(row);
            dialog.open = None;
        });
    }
}

fn checkbox(checked: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(CHECK_SIZE))
        .rounded(rpx(4.))
        .border_1()
        .border_color(rgba(theme::border_strong()))
        .when(checked, |this| {
            this.bg(rgb(theme::BLURPLE))
                .border_color(rgb(theme::BLURPLE))
                .child(theme::icon("check", 14., 0xFFFFFF))
        })
}

fn option_row(
    id: impl Into<ElementId>,
    content: impl IntoElement,
    checked: bool,
    on_click: impl Fn(&mut DiscordApp, &mut Window, &mut Context<DiscordApp>) + 'static,
    entity: WeakEntity<DiscordApp>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .min_h(rpx(36.))
        .px(rpx(8.))
        .rounded(rpx(6.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::hover())))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity.update(cx, |app, cx| on_click(app, window, cx)).ok();
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(8.))
                .flex_1()
                .min_w_0()
                .child(content),
        )
        .child(checkbox(checked))
}

fn pill(
    id: impl Into<ElementId>,
    text: String,
    on_remove: impl Fn(&mut DiscordApp, &mut Context<DiscordApp>) + 'static,
    entity: WeakEntity<DiscordApp>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(4.))
        .h(rpx(24.))
        .max_w(rpx(180.))
        .pl(rpx(8.))
        .pr(rpx(4.))
        .rounded(rpx(4.))
        .bg(rgba(theme::control_selected()))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .child(div().min_w_0().truncate().child(text))
        .child(
            theme::icon_button(id, "x", 12., theme::muted()).on_click(move |_, _, cx| {
                cx.stop_propagation();
                entity.update(cx, |app, cx| on_remove(app, cx)).ok();
            }),
        )
}

fn field_box(
    id: &'static str,
    field: Field,
    open: bool,
    placeholder: &'static str,
    pills: Vec<AnyElement>,
    entity: WeakEntity<DiscordApp>,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(rpx(8.))
        .min_h(rpx(40.))
        .px(rpx(12.))
        .py(rpx(4.))
        .rounded(rpx(8.))
        .bg(rgba(theme::recessed()))
        .border_1()
        .border_color(if open {
            rgb(theme::BLURPLE)
        } else {
            rgba(theme::border_emphasis())
        })
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity
                .update(cx, |app, cx| app.toggle_filter_field(field, window, cx))
                .ok();
        })
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(rpx(4.))
                .flex_1()
                .min_w_0()
                .map(|this| {
                    if pills.is_empty() {
                        this.text_size(rpx(14.))
                            .text_color(rgb(theme::faint()))
                            .child(placeholder)
                    } else {
                        this.children(pills)
                    }
                }),
        )
        .child(theme::icon(
            if open { "chevron-up" } else { "chevron-down" },
            16.,
            theme::muted(),
        ))
}

fn options_list(
    dialog: &FiltersDialog,
    searchable: bool,
    rows: Vec<AnyElement>,
) -> impl IntoElement {
    let empty = rows.is_empty();
    div()
        .id("search-filter-dropdown")
        .flex()
        .flex_col()
        .mt(rpx(4.))
        .on_click(|_, _, cx| cx.stop_propagation())
        .p(rpx(4.))
        .rounded(rpx(8.))
        .bg(rgb(theme::surface_high()))
        .border_1()
        .border_color(rgba(theme::outline()))
        .when(searchable, |this| {
            this.child(div().p(rpx(4.)).child(Input::new(&dialog.query)))
        })
        .child(
            div()
                .id("search-filter-options")
                .flex()
                .flex_col()
                .max_h(rpx(OPTIONS_HEIGHT))
                .overflow_y_scroll()
                .children(rows)
                .when(empty, |this| {
                    this.child(
                        div()
                            .px(rpx(8.))
                            .py(rpx(8.))
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::muted()))
                            .child("No matches"),
                    )
                }),
        )
}

fn section(title: &'static str, subtitle: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(title),
        )
        .child(
            div()
                .mb(rpx(8.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::muted()))
                .child(subtitle),
        )
}

fn user_section(
    app: &DiscordApp,
    dialog: &FiltersDialog,
    field: Field,
    guild_id: Option<u64>,
    cx: &App,
) -> impl IntoElement {
    let (id, title, subtitle) = match field {
        Field::From => ("filter-from", "From", "Sent by any of the selected users"),
        _ => (
            "filter-mentions",
            "Mentions",
            "Mentions any of the selected users",
        ),
    };
    let chosen = if field == Field::From {
        &dialog.from
    } else {
        &dialog.mentions
    };
    let open = dialog.open == Some(field);
    let pills = chosen
        .iter()
        .map(|(user_id, label)| {
            let user_id = *user_id;
            pill(
                (id, user_id),
                label.clone(),
                move |app, cx| {
                    app.edit_search_filters(cx, |dialog| {
                        users_of(dialog, field).retain(|(chosen, _)| *chosen != user_id)
                    })
                },
                app.entity.clone(),
            )
            .into_any_element()
        })
        .collect();
    let options = open.then(|| {
        let query = dialog.query.read(cx).value().trim().to_lowercase();
        let rows = app
            .store
            .search_users(guild_id, app.selected_channel_id, &query, OPTION_LIMIT)
            .into_iter()
            .filter_map(|user_id| {
                let user = app.store.user(user_id)?;
                let name = guild_id
                    .and_then(|guild_id| app.store.member_name(guild_id, user_id))
                    .unwrap_or(&user.name)
                    .to_owned();
                let username = user.username.clone();
                let checked = chosen.iter().any(|(chosen, _)| *chosen == user_id);
                let label = username.clone();
                Some(
                    option_row(
                        (id, user_id),
                        div()
                            .flex()
                            .items_center()
                            .gap(rpx(8.))
                            .min_w_0()
                            .child(theme::user_avatar(
                                Some(user),
                                user_id,
                                20.,
                                None,
                                false,
                                cx,
                            ))
                            .child(div().flex_none().max_w(rpx(180.)).truncate().child(name))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(rgb(theme::muted()))
                                    .child(username),
                            ),
                        checked,
                        move |app, _, cx| {
                            let label = label.clone();
                            app.edit_search_filters(cx, |dialog| {
                                let list = users_of(dialog, field);
                                match list.iter().position(|(chosen, _)| *chosen == user_id) {
                                    Some(index) => {
                                        list.remove(index);
                                    }
                                    None => list.push((user_id, label)),
                                }
                            })
                        },
                        app.entity.clone(),
                    )
                    .into_any_element(),
                )
            })
            .collect();
        options_list(dialog, true, rows)
    });
    section(title, subtitle)
        .child(field_box(
            id,
            field,
            open,
            "ex. username",
            pills,
            app.entity.clone(),
        ))
        .children(options)
}

fn channel_section(app: &DiscordApp, dialog: &FiltersDialog, cx: &App) -> impl IntoElement {
    let open = dialog.open == Some(Field::In);
    let pills = dialog
        .channels
        .iter()
        .map(|(channel_id, name)| {
            let channel_id = *channel_id;
            pill(
                ("filter-in", channel_id),
                format!("#{name}"),
                move |app, cx| {
                    app.edit_search_filters(cx, |dialog| {
                        dialog.channels.retain(|(chosen, _)| *chosen != channel_id)
                    })
                },
                app.entity.clone(),
            )
            .into_any_element()
        })
        .collect();
    let options = open.then(|| {
        let query = dialog.query.read(cx).value().trim().to_lowercase();
        let rows = app
            .guild()
            .into_iter()
            .flat_map(|guild| guild.channels())
            .filter(|channel| channel.kind.is_selectable())
            .filter(|channel| channel.name.to_lowercase().contains(&query))
            .take(OPTION_LIMIT)
            .map(|channel| {
                let channel_id = channel.id;
                let name = channel.name.clone();
                let checked = dialog
                    .channels
                    .iter()
                    .any(|(chosen, _)| *chosen == channel_id);
                option_row(
                    ("filter-in", channel_id),
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(6.))
                        .min_w_0()
                        .child(theme::icon("hash", 16., theme::muted()))
                        .child(div().min_w_0().truncate().child(name.clone())),
                    checked,
                    move |app, _, cx| {
                        let name = name.clone();
                        app.edit_search_filters(cx, |dialog| {
                            match dialog
                                .channels
                                .iter()
                                .position(|(chosen, _)| *chosen == channel_id)
                            {
                                Some(index) => {
                                    dialog.channels.remove(index);
                                }
                                None => dialog.channels.push((channel_id, name)),
                            }
                        })
                    },
                    app.entity.clone(),
                )
                .into_any_element()
            })
            .collect();
        options_list(dialog, true, rows)
    });
    section("In", "Sent in any of the selected channels")
        .child(field_box(
            "filter-in",
            Field::In,
            open,
            "ex. general",
            pills,
            app.entity.clone(),
        ))
        .children(options)
}

fn choice_section(
    app: &DiscordApp,
    dialog: &FiltersDialog,
    field: Field,
    choices: Vec<(&'static str, &'static str)>,
    chosen: Vec<&'static str>,
) -> impl IntoElement {
    let (id, title, subtitle, placeholder) = match field {
        Field::Has => (
            "filter-has",
            "Has",
            "Includes any of the selected types of data",
            "Any content",
        ),
        Field::Author => (
            "filter-author",
            "Author Type",
            "Sent by any of the selected types of author",
            "ex. bot",
        ),
        _ => (
            "filter-pinned",
            "Pinned",
            "If the message is pinned or not",
            "ex. true",
        ),
    };
    let open = dialog.open == Some(field);
    let apply = move |dialog: &mut FiltersDialog, value: &'static str| match field {
        Field::Has => toggle(&mut dialog.has, value),
        Field::Author => toggle(&mut dialog.authors, value),
        _ => {
            let pinned = value == "true";
            dialog.pinned = (dialog.pinned != Some(pinned)).then_some(pinned);
        }
    };
    let pills = choices
        .iter()
        .enumerate()
        .filter(|(_, (value, _))| chosen.contains(value))
        .map(|(index, (value, label))| {
            let value = *value;
            pill(
                (id, index as u64),
                (*label).to_owned(),
                move |app, cx| app.edit_search_filters(cx, |dialog| apply(dialog, value)),
                app.entity.clone(),
            )
            .into_any_element()
        })
        .collect();
    let options = open.then(|| {
        let rows = choices
            .iter()
            .enumerate()
            .map(|(index, (value, label))| {
                let value = *value;
                option_row(
                    (id, index as u64),
                    div().child(*label),
                    chosen.contains(&value),
                    move |app, _, cx| app.edit_search_filters(cx, |dialog| apply(dialog, value)),
                    app.entity.clone(),
                )
                .into_any_element()
            })
            .collect();
        options_list(dialog, false, rows)
    });
    section(title, subtitle)
        .child(field_box(
            id,
            field,
            open,
            placeholder,
            pills,
            app.entity.clone(),
        ))
        .children(options)
}

fn date_section(app: &DiscordApp, dialog: &FiltersDialog) -> impl IntoElement {
    let rows = dialog.dates.iter().enumerate().map(|(index, row)| {
        let open = dialog.open == Some(Field::DateKind(index));
        let toggle_kind = app.entity.clone();
        let remove = app.entity.clone();
        let label = DATE_KINDS
            .iter()
            .find(|(key, _)| *key == row.key)
            .map_or("Before", |(_, label)| *label);
        let kinds = open.then(|| {
            div()
                .w(rpx(120.))
                .p(rpx(4.))
                .rounded(rpx(8.))
                .bg(rgb(theme::surface_high()))
                .border_1()
                .border_color(rgba(theme::outline()))
                .shadow_lg()
                .occlude()
                .children(DATE_KINDS.iter().map(|(key, label)| {
                    let key = *key;
                    let entity = app.entity.clone();
                    div()
                        .id((
                            "filter-date-kind",
                            index * 3 + DATE_KINDS.iter().position(|(k, _)| *k == key).unwrap_or(0),
                        ))
                        .flex()
                        .items_center()
                        .h(rpx(32.))
                        .px(rpx(8.))
                        .rounded(rpx(6.))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgba(theme::hover())))
                        .when(key == row.key, |this| this.bg(rgba(theme::selected())))
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            entity
                                .update(cx, |app, cx| {
                                    app.edit_search_filters(cx, |dialog| {
                                        if let Some(row) = dialog.dates.get_mut(index) {
                                            row.key = key;
                                        }
                                        dialog.open = None;
                                    })
                                })
                                .ok();
                        })
                        .child(*label)
                }))
        });
        div()
            .relative()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .mb(rpx(8.))
            .child(
                div()
                    .id(("filter-date-type", index))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .gap(rpx(8.))
                    .w(rpx(120.))
                    .h(rpx(40.))
                    .px(rpx(12.))
                    .rounded(rpx(8.))
                    .bg(rgba(theme::recessed()))
                    .border_1()
                    .border_color(if open {
                        rgb(theme::BLURPLE)
                    } else {
                        rgba(theme::border_emphasis())
                    })
                    .cursor_pointer()
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::strong()))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        toggle_kind
                            .update(cx, |app, cx| {
                                app.toggle_filter_field(Field::DateKind(index), window, cx)
                            })
                            .ok();
                    })
                    .child(label)
                    .child(theme::icon("chevron-down", 16., theme::muted())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(DatePicker::new(&row.picker).placeholder("Pick a date")),
            )
            .child(
                theme::icon_button(("filter-date-remove", index), "x", 16., theme::muted())
                    .on_click(move |_, _, cx| {
                        remove
                            .update(cx, |app, cx| {
                                app.edit_search_filters(cx, |dialog| {
                                    if index < dialog.dates.len() {
                                        dialog.dates.remove(index);
                                    }
                                    dialog.open = None;
                                })
                            })
                            .ok();
                    }),
            )
            // Deferred so the sections below don't paint over the open list.
            .children(kinds.map(|kinds| {
                div().absolute().top(rpx(44.)).left_0().size_0().child(
                    deferred(anchored().anchor(Anchor::TopLeft).child(kinds)).with_priority(1),
                )
            }))
    });
    let add = app.entity.clone();
    section("Date", "When the message was sent")
        .children(rows)
        .child(
            div()
                .id("filter-date-add")
                .flex()
                .items_center()
                .justify_center()
                .gap(rpx(6.))
                .h(rpx(36.))
                .rounded(rpx(8.))
                .bg(rgba(theme::control_bg()))
                .border_1()
                .border_color(rgba(theme::border_soft()))
                .cursor_pointer()
                .hover(|style| style.bg(rgba(theme::hover())))
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .on_click(move |_, window, cx| {
                    add.update(cx, |app, cx| app.add_search_date(window, cx))
                        .ok();
                })
                .child(theme::icon("plus", 16., theme::strong()))
                .child("Add date"),
        )
}

fn footer_button(id: &'static str, text: String, primary: bool, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(rpx(38.))
        .px(rpx(16.))
        .rounded(rpx(8.))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if primary {
            white()
        } else {
            rgb(theme::bright()).into()
        })
        .bg(rgb(if primary {
            theme::BLURPLE
        } else {
            theme::surface_highest()
        }))
        .when(enabled, |this| {
            this.cursor_pointer().hover(|style| style.opacity(0.85))
        })
        .when(!enabled, |this| this.opacity(0.5))
        .child(text)
}

pub(crate) fn filters_dialog(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let search = app.active_search()?;
    let dialog = search.filters.as_ref()?;
    let guild_id = match search.scope {
        SearchScope::Guild(guild_id) => Some(guild_id),
        SearchScope::Channel(_) | SearchScope::Dms => None,
    };
    let chips = dialog.chips(cx);
    let changed = chips != dialog.initial;
    let count = chips.len();
    let backdrop = app.entity.clone();
    let close = app.entity.clone();
    let cancel = app.entity.clone();
    let apply = app.entity.clone();
    let clear = app.entity.clone();
    let dismiss = app.entity.clone();
    let pinned: Vec<&'static str> = dialog
        .pinned
        .map(|pinned| if pinned { "true" } else { "false" })
        .into_iter()
        .collect();
    Some(
        div()
            .id("search-filters-backdrop")
            .child(theme::selection_guard())
            .absolute()
            .inset_0()
            .top(rpx(titlebar::HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .bg(rgba(theme::SCRIM))
            .on_click(move |_, _, cx| {
                backdrop
                    .update(cx, |app, cx| app.close_search_filters(cx))
                    .ok();
            })
            .child(
                div()
                    .id("search-filters-dialog")
                    .flex()
                    .flex_col()
                    .w(rpx(WIDTH))
                    .max_w_full()
                    .rounded(rpx(12.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .occlude()
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        dismiss
                            .update(cx, |app, cx| {
                                app.edit_search_filters(cx, |dialog| dialog.open = None)
                            })
                            .ok();
                    })
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_between()
                            .px(rpx(16.))
                            .py(rpx(16.))
                            .border_b_1()
                            .border_color(rgba(theme::border()))
                            .child(
                                div()
                                    .text_size(rpx(20.))
                                    .line_height(rpx(24.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(theme::strong()))
                                    .child("Filters"),
                            )
                            .child(
                                theme::icon_button(
                                    "search-filters-close",
                                    "x",
                                    20.,
                                    theme::muted(),
                                )
                                .on_click(move |_, _, cx| {
                                    close
                                        .update(cx, |app, cx| app.close_search_filters(cx))
                                        .ok();
                                }),
                            ),
                    )
                    .child(
                        div()
                            .id("search-filters-body")
                            .flex()
                            .flex_col()
                            // gpui-component's scroll wrapper sizes itself as a percentage of
                            // its parent, so it needs a definite height here.
                            .h(rpx(BODY_HEIGHT))
                            .gap(rpx(16.))
                            .px(rpx(16.))
                            .py(rpx(16.))
                            .overflow_y_scrollbar()
                            .child(user_section(app, dialog, Field::From, guild_id, cx))
                            .when(guild_id.is_some(), |this| {
                                this.child(channel_section(app, dialog, cx))
                            })
                            .child(choice_section(
                                app,
                                dialog,
                                Field::Has,
                                HAS_OPTIONS
                                    .iter()
                                    .map(|(name, api)| (*api, *name))
                                    .collect(),
                                dialog.has.clone(),
                            ))
                            .child(user_section(app, dialog, Field::Mentions, guild_id, cx))
                            .child(date_section(app, dialog))
                            .child(choice_section(
                                app,
                                dialog,
                                Field::Author,
                                AUTHOR_TYPES.iter().map(|kind| (*kind, *kind)).collect(),
                                dialog.authors.clone(),
                            ))
                            .child(choice_section(
                                app,
                                dialog,
                                Field::Pinned,
                                vec![("true", "true"), ("false", "false")],
                                pinned,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(rpx(8.))
                            .px(rpx(16.))
                            .py(rpx(16.))
                            .border_t_1()
                            .border_color(rgba(theme::border()))
                            .child(
                                div()
                                    .id("search-filters-clear")
                                    .flex_1()
                                    .text_size(rpx(14.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(theme::link()))
                                    .when(count == 0, |this| this.opacity(0.5))
                                    .when(count > 0, |this| {
                                        this.cursor_pointer()
                                            .hover(|style| style.underline())
                                            .on_click(move |_, _, cx| {
                                                clear
                                                    .update(cx, |app, cx| {
                                                        app.edit_search_filters(cx, |dialog| {
                                                            dialog.from.clear();
                                                            dialog.channels.clear();
                                                            dialog.has.clear();
                                                            dialog.mentions.clear();
                                                            dialog.dates.clear();
                                                            dialog.authors.clear();
                                                            dialog.pinned = None;
                                                            dialog.open = None;
                                                        })
                                                    })
                                                    .ok();
                                            })
                                    })
                                    .child(if count > 0 {
                                        format!("Clear Filters ({count})")
                                    } else {
                                        "Clear Filters".to_owned()
                                    }),
                            )
                            .child(
                                footer_button(
                                    "search-filters-cancel",
                                    "Cancel".to_owned(),
                                    false,
                                    true,
                                )
                                .on_click(move |_, _, cx| {
                                    cancel
                                        .update(cx, |app, cx| app.close_search_filters(cx))
                                        .ok();
                                }),
                            )
                            .child(
                                footer_button(
                                    "search-filters-apply",
                                    "Apply Filters".to_owned(),
                                    true,
                                    changed,
                                )
                                .when(changed, |this| {
                                    this.on_click(move |_, _, cx| {
                                        apply
                                            .update(cx, |app, cx| app.apply_search_filters(cx))
                                            .ok();
                                    })
                                }),
                            ),
                    ),
            )
            .into_any_element(),
    )
}
