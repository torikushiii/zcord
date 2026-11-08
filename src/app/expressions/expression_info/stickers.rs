use std::collections::HashSet;

use super::*;

pub(super) struct Source<'a> {
    pub id: u64,
    pub joined: bool,
    pub catalog_complete: bool,
    name: &'a str,
    icon: Option<ImageSpec>,
    initials: String,
    color: u32,
    online: Option<u32>,
    pub(super) discoverable: Option<bool>,
    stickers: Option<&'a [model::StickerDetails]>,
}

fn complete_catalog(guild: &model::StickerGuild) -> bool {
    guild.stickers.as_ref().is_some_and(|stickers| {
        guild.sticker_count.map_or(stickers.len() < 30, |count| {
            count as usize <= stickers.len()
        })
    })
}

pub(super) fn source<'a>(app: &'a DiscordApp, popout: &Popout) -> Option<Source<'a>> {
    let remote = popout
        .source_sticker
        .and_then(|id| app.expression_info.sticker_guild(id));
    let joined = match popout.source_sticker {
        Some(id) => app.store.sticker_origin(id).or_else(|| {
            let guild_id = app.expression_info.sticker_details(id)?.guild_id?;
            app.store.guild_by_id(guild_id)
        }),
        None => origin(app, &popout.expression),
    }
    .or_else(|| remote.and_then(|guild| app.store.guild_by_id(guild.id)));
    let id = joined
        .map(|guild| guild.id)
        .or_else(|| remote.map(|guild| guild.id))?;
    let remote = remote.filter(|guild| guild.id == id);
    let preview = match app.expression_info.catalogs.get(&id) {
        Some(Load::Ready(guild)) => Some(guild),
        _ => None,
    };
    let joined_catalog = app.store.guild_stickers(id);
    let preview_catalog = preview.and_then(|guild| guild.stickers.as_deref());
    let stickers = joined_catalog
        .or(preview_catalog)
        .or_else(|| remote.and_then(|guild| guild.stickers.as_deref()));
    let catalog_complete = joined_catalog.is_some()
        || preview_catalog.is_some()
        || remote.is_some_and(complete_catalog);
    let online = preview
        .and_then(|guild| guild.approximate_presence_count)
        .or_else(|| remote.and_then(|guild| guild.approximate_presence_count));
    if let Some(guild) = joined {
        return Some(Source {
            id,
            joined: true,
            catalog_complete,
            name: &guild.name,
            icon: guild.icon_spec(),
            initials: guild.initials.clone(),
            color: guild.icon_color,
            online,
            discoverable: app.store.guild_discoverable(id),
            stickers,
        });
    }
    let guild = remote?;
    Some(Source {
        id,
        joined: false,
        catalog_complete,
        name: &guild.name,
        icon: guild.icon.as_ref().map(|hash| ImageSpec::GuildIcon {
            guild_id: id,
            hash: hash.clone(),
        }),
        initials: guild
            .name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .collect(),
        color: theme::surface_highest(),
        online,
        discoverable: guild.discoverable(),
        stickers,
    })
}

pub(super) fn source_card(
    app: &DiscordApp,
    popout: &Popout,
    source: &Source<'_>,
    cx: &App,
) -> AnyElement {
    let sticker = popout.source_sticker.is_some();
    let corner = rpx(if sticker { 8. } else { 18. });
    let pixels = theme::image_px(36.);
    let icon = source
        .icon
        .as_ref()
        .and_then(|spec| {
            let animated = spec
                .animated()
                .and_then(|spec| animated::resolve(&spec, pixels, cx));
            Some(match animated {
                Some(image) => animated::image(image)
                    .size(rpx(36.))
                    .rounded(corner)
                    .object_fit(ObjectFit::Cover)
                    .id(("expression-guild-icon-animated", source.id))
                    .into_any_element(),
                None => img(images::resolve(spec, pixels, cx)?)
                    .size(rpx(36.))
                    .rounded(corner)
                    .object_fit(ObjectFit::Cover)
                    .into_any_element(),
            })
        })
        .unwrap_or_else(|| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .bg(rgb(source.color))
                .child(source.initials.clone())
                .into_any_element()
        });
    let metadata = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(rpx(8.))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .children(
            source
                .online
                .map(|online| div().child(format!("{} Online", grouped(online)))),
        )
        .when(
            source.online.is_some() && source.discoverable.is_some(),
            |this| this.child(div().size(rpx(3.)).rounded_full().bg(rgb(theme::muted()))),
        )
        .children(source.discoverable.map(|discoverable| {
            div().child(match (sticker, discoverable) {
                (true, true) => "Discoverable Server",
                (true, false) => "Invite-only Server",
                (false, true) => "Discoverable server",
                (false, false) => "Invite-only server",
            })
        }));
    let mut body = div()
        .flex()
        .flex_col()
        .gap(rpx(12.))
        .child(div().text_color(rgb(theme::muted())).child(if sticker {
            "Get this sticker from"
        } else {
            "This emoji is from"
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .child(
                    div()
                        .size(rpx(36.))
                        .flex_none()
                        .rounded(corner)
                        .overflow_hidden()
                        .child(icon),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(rpx(if sticker { 16. } else { 14. }))
                                .line_height(rpx(20.))
                                .line_clamp(2)
                                .child(
                                    if source.name.is_empty() {
                                        "Unknown server"
                                    } else {
                                        source.name
                                    }
                                    .to_owned(),
                                ),
                        )
                        .child(metadata),
                ),
        );
    if popout.source_sticker.is_some() {
        let click = app.entity.clone();
        let keyboard = app.entity.clone();
        body = body.child(
            div()
                .id("expression-more-stickers")
                .role(Role::Button)
                .aria_expanded(popout.expanded)
                .focusable()
                .tab_index(0)
                .flex()
                .items_center()
                .gap(rpx(4.))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
                .cursor_pointer()
                .hover(|style| style.text_color(rgb(theme::text())))
                .focus_visible(|style| style.bg(rgba(theme::selected())))
                .on_click(move |_, _, cx| {
                    click
                        .update(cx, |app, cx| app.toggle_expression_stickers(cx))
                        .ok();
                    cx.stop_propagation();
                })
                .on_key_down(move |event, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        keyboard
                            .update(cx, |app, cx| app.toggle_expression_stickers(cx))
                            .ok();
                        cx.stop_propagation();
                    }
                })
                .child(if popout.expanded {
                    "Show Fewer Stickers"
                } else {
                    "Show More Stickers"
                })
                .child(theme::icon(
                    if popout.expanded {
                        "chevron-up"
                    } else {
                        "chevron-down"
                    },
                    14.,
                    theme::muted(),
                )),
        );
        if popout.expanded {
            body = body.child(sticker_grid(app, popout, source));
        }
    }
    body.into_any_element()
}

fn catalog_entries(details: &[model::StickerDetails]) -> Vec<Sticker> {
    let mut seen = HashSet::new();
    details
        .iter()
        .filter(|sticker| sticker.id != 0 && seen.insert(sticker.id))
        .map(|sticker| Sticker {
            id: sticker.id,
            name: sticker.name.clone(),
            format: sticker.format_type,
        })
        .collect()
}

fn sticker_grid(app: &DiscordApp, popout: &Popout, source: &Source<'_>) -> AnyElement {
    let entries = source.stickers.map(catalog_entries).unwrap_or_default();
    let rows = entries.len().div_ceil(STICKER_COLUMNS);
    let hovered = popout.hovered_sticker;
    let selected = popout.expression.key();
    let entity = app.entity.clone();
    let mut body = div().flex().flex_col().gap(rpx(8.));
    if !entries.is_empty() {
        body = body.child(
            uniform_list("expression-sticker-grid", rows, move |range, _, cx| {
                range
                    .map(|row| {
                        div()
                            .flex()
                            .h(rpx(STICKER_CELL))
                            .children(
                                entries
                                    .iter()
                                    .skip(row * STICKER_COLUMNS)
                                    .take(STICKER_COLUMNS)
                                    .map(|sticker| {
                                        let expression = Expression::Sticker(sticker.clone());
                                        let label = expression.label();
                                        let id = sticker.id;
                                        let click = entity.clone();
                                        let keyboard = entity.clone();
                                        let hover = entity.clone();
                                        let clicked = sticker.clone();
                                        let keyed = sticker.clone();
                                        div()
                                            .id(("expression-sticker-cell", id))
                                            .role(Role::Button)
                                            .aria_label(format!("Preview {label}"))
                                            .aria_selected(selected == expression.key())
                                            .focusable()
                                            .tab_index(0)
                                            .relative()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .size(rpx(STICKER_CELL))
                                            .flex_none()
                                            .rounded(rpx(4.))
                                            .cursor_pointer()
                                            .hover(|style| style.bg(rgba(theme::selected())))
                                            .focus_visible(|style| {
                                                style.bg(rgba(theme::selected()))
                                            })
                                            .on_hover(move |hovered, _, cx| {
                                                hover
                                                    .update(cx, |app, cx| {
                                                        app.hover_catalog_sticker(id, *hovered, cx)
                                                    })
                                                    .ok();
                                            })
                                            .on_click(move |_, _, cx| {
                                                click
                                                    .update(cx, |app, cx| {
                                                        app.inspect_catalog_sticker(
                                                            clicked.clone(),
                                                            cx,
                                                        )
                                                    })
                                                    .ok();
                                                cx.stop_propagation();
                                            })
                                            .on_key_down(move |event, _, cx| {
                                                if matches!(
                                                    event.keystroke.key.as_str(),
                                                    "enter" | "space"
                                                ) {
                                                    keyboard
                                                        .update(cx, |app, cx| {
                                                            app.inspect_catalog_sticker(
                                                                keyed.clone(),
                                                                cx,
                                                            )
                                                        })
                                                        .ok();
                                                    cx.stop_propagation();
                                                }
                                            })
                                            .child(preview(&expression, 36., cx))
                                            .when(hovered == Some(id), |this| {
                                                this.child(theme::tooltip_above(label))
                                            })
                                    }),
                            )
                            .into_any_element()
                    })
                    .collect()
            })
            .track_scroll(&popout.scroll)
            .h(rpx(rows.min(2) as f32 * STICKER_CELL))
            .w_full(),
        );
    }
    let pending = matches!(
        app.expression_info.catalogs.get(&source.id),
        Some(Load::Pending)
    );
    let failed = !source.catalog_complete && !pending;
    if rows == 0 || pending || failed {
        let text = if pending {
            "Loading stickers..."
        } else if failed {
            "Some stickers couldn't be loaded."
        } else {
            "No stickers available"
        };
        body = body.child(
            div()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child(text),
        );
        if failed {
            let entity = app.entity.clone();
            let keyboard = entity.clone();
            body = body.child(
                div()
                    .id("expression-stickers-retry")
                    .role(Role::Button)
                    .focusable()
                    .tab_index(0)
                    .cursor_pointer()
                    .text_size(rpx(12.))
                    .text_color(rgb(theme::link()))
                    .hover(|style| style.underline())
                    .focus_visible(|style| style.underline())
                    .on_click(move |_, _, cx| {
                        entity
                            .update(cx, |app, cx| app.load_open_sticker_catalog(cx))
                            .ok();
                    })
                    .on_key_down(move |event, _, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            keyboard
                                .update(cx, |app, cx| app.load_open_sticker_catalog(cx))
                                .ok();
                            cx.stop_propagation();
                        }
                    })
                    .child("Retry"),
            );
        }
    }
    body.into_any_element()
}

fn grouped(value: u32) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::{catalog_entries, complete_catalog, grouped};
    use crate::discord::model::StickerGuild;
    use serde_json::json;

    #[test]
    fn catalogs_distinguish_complete_partial_and_unknown() {
        for (payload, complete) in [
            (json!({"id": "1"}), false),
            (json!({"id": "1", "stickers": null}), false),
            (json!({"id": "1", "stickers": []}), true),
            (
                json!({"id": "1", "stickers": [], "sticker_count": 1}),
                false,
            ),
            (
                json!({"id": "1", "stickers": [{"id": "2"}], "sticker_count": 1}),
                true,
            ),
        ] {
            let guild: StickerGuild = serde_json::from_value(payload).unwrap();
            assert_eq!(complete_catalog(&guild), complete);
        }
        let guild: StickerGuild = serde_json::from_value(json!({
            "id": "1", "stickers": (1..=30).map(|id| json!({"id": id})).collect::<Vec<_>>()
        }))
        .unwrap();
        assert!(!complete_catalog(&guild));
    }

    #[test]
    fn thumbnails_deduplicate_ids_without_losing_formats() {
        let guild: StickerGuild = serde_json::from_value(json!({
            "id": "1", "stickers": [
                {"id": "2", "name": "first", "format_type": 2},
                {"id": "2", "name": "duplicate", "format_type": 1},
                {"id": "3", "name": "second", "format_type": 4},
                {"id": "0", "name": "invalid"}
            ]
        }))
        .unwrap();
        let entries = catalog_entries(guild.stickers.as_deref().unwrap());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "first");
        assert_eq!(entries[0].format, 2);
        assert_eq!(entries[1].id, 3);
        assert_eq!(entries[1].format, 4);
    }

    #[test]
    fn online_counts_keep_zero_and_group_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(42_115), "42,115");
        assert_eq!(grouped(u32::MAX), "4,294,967,295");
    }
}
