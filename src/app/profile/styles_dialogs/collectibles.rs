use super::name_style::{effect_grid_art, frame_grid_art};
use super::*;

const GRID_GAP: f32 = 12.;
const DECORATION_ITEM: [f32; 2] = [75.3, 80.];
const PROFILE_ITEM: [f32; 2] = [83.3, 80.];
const NAMEPLATE_ITEM: [f32; 2] = [224., 42.];

fn grid_item(id: ElementId, [width, height]: [f32; 2], selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .w(rpx(width))
        .h(rpx(height))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rpx(4.))
        .overflow_hidden()
        .rounded(rpx(4.))
        .border_2()
        .border_color(if selected {
            rgb(theme::BLURPLE)
        } else {
            rgba(0)
        })
        .bg(rgb(theme::base_lowest()))
        .cursor_pointer()
        .hover(move |style| {
            if selected {
                style
            } else {
                style.border_color(rgba(theme::border_strong()))
            }
        })
}

fn select_collectible(
    app: &DiscordApp,
    kind: CollectibleKind,
    sku_id: Option<u64>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    on_app(app, move |app, cx| {
        app.update_styles_ui(
            |ui| {
                ui.dialog = Some(StyleDialog::Collectible {
                    kind,
                    selected: sku_id,
                })
            },
            cx,
        )
    })
}

fn inventory_controls(
    app: &DiscordApp,
    kind: CollectibleKind,
    selected: Option<u64>,
    size: [f32; 2],
) -> [AnyElement; 2] {
    let control = |id: &'static str, icon: &'static str, label: &'static str, selected: bool| {
        grid_item(id.into(), size, selected)
            .child(theme::icon(icon, 24., theme::strong()))
            .child(
                div()
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(theme::strong()))
                    .child(label),
            )
    };
    [
        control("collectible-none", "ban", "None", selected.is_none())
            .on_click(select_collectible(app, kind, None))
            .into_any_element(),
        control("collectible-shop", "store", "Shop", false)
            .on_click(|_, _, cx| {
                cx.stop_propagation();
                cx.open_url("https://discord.com/shop");
            })
            .into_any_element(),
    ]
}

fn nameplate_item(
    app: &DiscordApp,
    editor: &StylesEditor,
    sku_id: u64,
    selected: bool,
    cx: &App,
) -> AnyElement {
    let user_id = app.store.current_user_id;
    let hovered = editor.ui.hovered_collectible == Some(sku_id);
    div()
        .id(("collectible", sku_id))
        .relative()
        .flex_none()
        .w(rpx(NAMEPLATE_ITEM[0]))
        .h(rpx(NAMEPLATE_ITEM[1]))
        .overflow_hidden()
        .rounded(rpx(8.))
        .bg(rgb(theme::base_lowest()))
        .cursor_pointer()
        .on_click(select_collectible(
            app,
            CollectibleKind::Nameplate,
            Some(sku_id),
        ))
        .on_hover(hover_collectible(app, sku_id))
        .children(nameplate_of(app, editor, sku_id).and_then(|plate| {
            rounded_nameplate_layer(&plate, user_id, NAMEPLATE_ITEM[1], hovered, 8., cx)
        }))
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .px(rpx(8.))
                .child(theme::placeholder_avatar(32., None, cx))
                .child(placeholder_bar(121., 0.5)),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(rpx(8.))
                .border_2()
                .border_color(if selected {
                    rgb(theme::BLURPLE)
                } else {
                    rgba(0)
                })
                .hover(move |style| {
                    if selected {
                        style
                    } else {
                        style.border_color(rgba(theme::border_strong()))
                    }
                }),
        )
        .into_any_element()
}

fn placeholder_bar(width: f32, opacity: f32) -> Div {
    div()
        .w(rpx(width))
        .h(rpx(14.))
        .rounded(rpx(8.))
        .bg(rgb(0x706F74))
        .opacity(opacity)
}

#[derive(Clone, Copy)]
enum ListCell {
    None,
    Shop,
    Item(u64),
}

enum Line {
    Heading(&'static str, f32),
    Loading,
    Cells(Vec<ListCell>),
}

pub(super) fn item_size(kind: CollectibleKind) -> [f32; 2] {
    match kind {
        CollectibleKind::Decoration => DECORATION_ITEM,
        CollectibleKind::Nameplate => NAMEPLATE_ITEM,
        CollectibleKind::Effect | CollectibleKind::Frame => PROFILE_ITEM,
    }
}

fn list_lines(
    editor: &StylesEditor,
    kind: CollectibleKind,
    owned_title: &'static str,
) -> Vec<Line> {
    let mut lines = vec![Line::Heading(owned_title, 0.)];
    let Some(_) = editor.owned else {
        lines.push(Line::Loading);
        return lines;
    };
    let owned = editor
        .owned_of(kind)
        .into_iter()
        .map(|item| ListCell::Item(item.sku_id));
    let shop: Vec<ListCell> = editor
        .shop_of(kind)
        .into_iter()
        .map(|item| ListCell::Item(item.sku_id))
        .collect();
    if kind == CollectibleKind::Nameplate {
        lines.push(Line::Cells(vec![ListCell::None, ListCell::Shop]));
        lines.extend(owned.map(|cell| Line::Cells(vec![cell])));
    } else {
        let cells: Vec<ListCell> = [ListCell::None, ListCell::Shop]
            .into_iter()
            .chain(owned)
            .collect();
        lines.extend(cells.chunks(3).map(|row| Line::Cells(row.to_vec())));
    }
    if !shop.is_empty() {
        lines.push(Line::Heading("See What's in Shop", 16.));
        let columns = if kind == CollectibleKind::Nameplate {
            1
        } else {
            3
        };
        lines.extend(shop.chunks(columns).map(|row| Line::Cells(row.to_vec())));
    }
    lines
}

fn hover_collectible(
    app: &DiscordApp,
    sku_id: u64,
) -> impl Fn(&bool, &mut Window, &mut App) + use<> {
    let entity = app.entity.clone();
    move |entered, _, cx| {
        let entered = *entered;
        entity
            .update(cx, |app, cx| {
                app.update_styles_ui(
                    |ui| {
                        if entered {
                            ui.hovered_collectible = Some(sku_id);
                        } else if ui.hovered_collectible == Some(sku_id) {
                            ui.hovered_collectible = None;
                        }
                    },
                    cx,
                )
            })
            .ok();
    }
}

fn item_art(
    app: &DiscordApp,
    editor: &StylesEditor,
    kind: CollectibleKind,
    sku_id: u64,
    cx: &App,
) -> AnyElement {
    let animate = editor.ui.hovered_collectible == Some(sku_id);
    match kind {
        CollectibleKind::Decoration => decoration_of(app, editor, sku_id)
            .and_then(|decoration| {
                let pixels = theme::image_px(64.);
                let frames = (animate && decoration.asset.starts_with("a_"))
                    .then(|| animated::resolve(&decoration.image_spec(true), pixels, cx))
                    .flatten();
                match frames {
                    Some(frames) => Some(
                        animated::image(frames)
                            .id(("decoration-item", sku_id))
                            .size(rpx(64.))
                            .into_any_element(),
                    ),
                    None => images::resolve(&decoration.image_spec(false), pixels, cx)
                        .map(|path| img(path).size(rpx(64.)).into_any_element()),
                }
            })
            .unwrap_or_else(|| div().size(rpx(64.)).into_any_element()),
        CollectibleKind::Effect => effect_grid_art(editor, sku_id, animate, cx),
        CollectibleKind::Frame => frame_grid_art(editor, sku_id, cx),
        CollectibleKind::Nameplate => unreachable!("nameplates are listed as rows"),
    }
}

fn collectible_entry(
    app: &DiscordApp,
    editor: &StylesEditor,
    kind: CollectibleKind,
    sku_id: u64,
    selected: Option<u64>,
    cx: &App,
) -> AnyElement {
    let chosen = selected == Some(sku_id);
    let element = if kind == CollectibleKind::Nameplate {
        nameplate_item(app, editor, sku_id, chosen, cx)
    } else {
        grid_item(("collectible", sku_id).into(), item_size(kind), chosen)
            .on_click(select_collectible(app, kind, Some(sku_id)))
            .on_hover(hover_collectible(app, sku_id))
            .child(item_art(app, editor, kind, sku_id, cx))
            .into_any_element()
    };
    if editor.is_owned(sku_id) {
        return element;
    }
    div()
        .relative()
        .flex_none()
        .child(element)
        .child(
            div()
                .absolute()
                .top(rpx(-4.))
                .right(rpx(-4.))
                .size(rpx(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgb(theme::base_lowest()))
                .child(theme::icon("lock", 12., 0xFFFFFF)),
        )
        .into_any_element()
}

fn render_line(
    app: &DiscordApp,
    editor: &StylesEditor,
    kind: CollectibleKind,
    selected: Option<u64>,
    line: &Line,
    cx: &App,
) -> AnyElement {
    match line {
        Line::Heading(title, top) => div()
            .pt(rpx(*top))
            .pb(rpx(4.))
            .text_size(rpx(16.))
            .line_height(rpx(20.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::strong()))
            .child(*title)
            .into_any_element(),
        Line::Loading => div()
            .flex()
            .justify_center()
            .py(rpx(32.))
            .child(theme::spinner(24., theme::muted()))
            .into_any_element(),
        Line::Cells(cells) => {
            let controls = match kind {
                CollectibleKind::Nameplate => [106., 80.],
                kind => item_size(kind),
            };
            let [none, shop] = inventory_controls(app, kind, selected, controls);
            let mut controls = [Some(none), Some(shop)];
            div()
                .flex()
                .gap(rpx(GRID_GAP))
                .pb(rpx(GRID_GAP))
                .when(
                    kind == CollectibleKind::Nameplate && matches!(cells[..], [ListCell::None, ..]),
                    |this| this.pb(rpx(16.)),
                )
                .children(cells.iter().filter_map(|cell| match cell {
                    ListCell::None => controls[0].take(),
                    ListCell::Shop => controls[1].take(),
                    ListCell::Item(sku_id) => {
                        Some(collectible_entry(app, editor, kind, *sku_id, selected, cx))
                    }
                }))
                .into_any_element()
        }
    }
}

pub(super) fn collectible_dialog(
    app: &DiscordApp,
    editor: &StylesEditor,
    kind: CollectibleKind,
    selected: Option<u64>,
    cx: &App,
) -> AnyElement {
    let (title, owned_title, width) = match kind {
        CollectibleKind::Decoration => ("Change Avatar Decoration", "Your Decorations", 604.),
        CollectibleKind::Nameplate => ("Change Nameplate", "Your Nameplates", 604.),
        CollectibleKind::Effect => ("Change Profile Effect", "Your Effects", 695.),
        CollectibleKind::Frame => ("Change Profile Frame", "Your Frames", 695.),
    };
    let current = shown(app, editor);
    let saved = match kind {
        CollectibleKind::Decoration => current.decoration,
        CollectibleKind::Nameplate => current.nameplate,
        CollectibleKind::Effect => current.effect,
        CollectibleKind::Frame => current.frame,
    };
    let lines = Rc::new(list_lines(editor, kind, owned_title));
    let list_width = match kind {
        CollectibleKind::Nameplate => 106. * 2. + GRID_GAP,
        kind => item_size(kind)[0] * 3. + GRID_GAP * 2.,
    };
    let list_view = editor.ui.collectible_list.clone().map(|state| {
        if state.item_count() != lines.len() {
            state.reset(lines.len());
        }
        let entity = app.entity.clone();
        list(state, move |index, _, cx| {
            let Some(entity) = entity.upgrade() else {
                return div().into_any_element();
            };
            let app = entity.read(cx);
            let Some(editor) = app.profile_styles.as_ref() else {
                return div().into_any_element();
            };
            let selected = match &editor.ui.dialog {
                Some(StyleDialog::Collectible { selected, .. }) => selected.to_owned(),
                _ => None,
            };
            render_line(app, editor, kind, selected, &lines[index], cx)
        })
        .size_full()
    });
    let profile_preview = matches!(kind, CollectibleKind::Effect | CollectibleKind::Frame);
    let preview = collectible_preview(app, editor, kind, selected, &current, cx);
    let details = selected
        .and_then(|sku_id| editor.find_collectible(sku_id))
        .map(|item| {
            div()
                .p(rpx(2.))
                .rounded(rpx(4.))
                .bg(rgba(theme::border_subtle()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(8.))
                        .p(rpx(12.))
                        .rounded(rpx(2.))
                        .bg(rgb(theme::base_lowest()))
                        .child(
                            div()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(item.product.name.clone()),
                        )
                        .children(
                            item.purchased_at
                                .as_deref()
                                .and_then(acquired)
                                .or_else(|| {
                                    (!editor.is_owned(item.sku_id))
                                        .then(|| "Available in Shop".to_owned())
                                })
                                .map(|text| {
                                    div()
                                        .text_size(rpx(10.))
                                        .line_height(rpx(12.))
                                        .text_color(rgb(theme::muted()))
                                        .child(text)
                                }),
                        ),
                )
        });
    let body = div()
        .flex()
        .justify_between()
        .gap(rpx(16.))
        .min_h_0()
        .pt(rpx(8.))
        .px(rpx(24.))
        .child(
            div()
                .w(rpx(list_width + 8.))
                .h(rpx(if profile_preview { 640. } else { 408. }))
                .p(rpx(4.))
                .children(list_view),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_none()
                .w(rpx(if profile_preview { 342. } else { 280. }))
                .gap(rpx(12.))
                .pt(rpx(8.))
                .child(preview)
                .children(details),
        )
        .into_any_element();
    let changed = selected != saved && selected.is_none_or(|sku_id| editor.is_owned(sku_id));
    let footer = div()
        .child(
            button("collectible-apply", "Apply", true, 100.)
                .when(!changed, |this| this.opacity(0.5).cursor_default())
                .when(changed, |this| {
                    this.on_click(on_app(app, |app, cx| app.apply_style_dialog(cx)))
                }),
        )
        .child(
            button("collectible-cancel", "Cancel", false, 100.)
                .on_click(on_app(app, |app, cx| app.close_style_dialog(cx))),
        );
    shell(app, editor, Some(title), width, body, footer)
}

fn collectible_preview(
    app: &DiscordApp,
    editor: &StylesEditor,
    kind: CollectibleKind,
    selected: Option<u64>,
    current: &super::styles_panel::Shown,
    cx: &App,
) -> AnyElement {
    let user_id = app.store.current_user_id;
    let user = app.store.user(user_id);
    let decoration = match kind {
        CollectibleKind::Decoration => selected,
        _ => current.decoration,
    }
    .and_then(|sku| decoration_of(app, editor, sku));
    let avatar_with = |size: f32, status: bool| {
        // Each copy keeps its own animation clock; sharing one, the copies' different-sized
        // images would keep resetting it to the first frame.
        div()
            .id(SharedString::from(format!(
                "preview-avatar-{size}-{status}"
            )))
            .flex_none()
            .child(theme::preview_avatar(
                user,
                user_id,
                size,
                user.filter(|_| status)
                    .map(|user| (user.status, theme::base_lowest())),
                current.avatar.clone(),
                current.avatar_file.clone(),
                decoration.as_ref(),
                cx,
            ))
    };
    let avatar = |size: f32| avatar_with(size, false);
    let stage = || {
        div()
            .flex()
            .items_center()
            .justify_center()
            .rounded(rpx(4.))
            .bg(rgb(theme::base_lowest()))
    };
    match kind {
        CollectibleKind::Decoration => div()
            .flex()
            .gap(rpx(12.))
            .child(stage().size(rpx(208.)).child(avatar(120.)))
            .child(
                stage()
                    .flex_col()
                    .justify_between()
                    .w(rpx(56.))
                    .h(rpx(208.))
                    .p(rpx(8.))
                    .child(avatar(40.))
                    .child(avatar(32.))
                    .child(avatar_with(40., true))
                    .child(avatar_with(32., true)),
            )
            .into_any_element(),
        CollectibleKind::Nameplate => {
            let name = own_name(app);
            let style = current.name_style.as_ref().map(shown_name_style);
            let skeleton = || {
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(rpx(12.))
                    .h(rpx(NAMEPLATE_ITEM[1]))
                    .px(rpx(8.))
                    .child(div().size(rpx(32.)).rounded_full().bg(rgb(0x2E2F34)))
                    .child(
                        div()
                            .w(rpx(121.))
                            .h(rpx(14.))
                            .rounded(rpx(8.))
                            .bg(rgb(0x2E2F34)),
                    )
            };
            let member = div()
                .relative()
                .flex_none()
                .w(rpx(NAMEPLATE_ITEM[0]))
                .h(rpx(NAMEPLATE_ITEM[1]))
                .overflow_hidden()
                .rounded(rpx(8.))
                .children(
                    selected
                        .and_then(|sku| nameplate_of(app, editor, sku))
                        .and_then(|plate| {
                            member_nameplate_layer(&plate, user_id, NAMEPLATE_ITEM[1], true, 8., cx)
                        }),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .gap(rpx(12.))
                        .px(rpx(8.))
                        .child(avatar(32.))
                        .child(
                            div()
                                .min_w_0()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(display_name::render(
                                    "nameplate-preview-name",
                                    &name,
                                    style.as_ref(),
                                    Surface::Account { hovered: true },
                                    cx,
                                )),
                        )
                        .children(
                            user.and_then(|user| user.tag.as_ref())
                                .map(|tag| theme::guild_tag(tag, cx)),
                        ),
                );
            div()
                .relative()
                .w(rpx(276.))
                .h(rpx(190.))
                .overflow_hidden()
                .rounded(rpx(8.))
                .bg(rgb(theme::base_lowest()))
                .child(
                    div()
                        .absolute()
                        .top(rpx(-22.))
                        .left(rpx(26.))
                        .flex()
                        .flex_col()
                        .gap(rpx(6.))
                        .child(skeleton())
                        .child(skeleton())
                        .child(member)
                        .child(skeleton())
                        .child(skeleton()),
                )
                .into_any_element()
        }
        CollectibleKind::Effect | CollectibleKind::Frame => {
            let (effect_sku, frame_sku) = match kind {
                CollectibleKind::Effect => (selected, current.frame),
                _ => (current.effect, selected),
            };
            let product = |sku: Option<u64>| {
                sku.and_then(|sku| editor.find_collectible(sku))
                    .map(|owned| owned.product.clone())
            };
            let (effect, frame) = (product(effect_sku), product(frame_sku));
            let effect_item = effect
                .as_deref()
                .and_then(|product| product.items.iter().find(|item| item.kind == 1));
            let frame = frame.as_deref().and_then(super::frame::frame_item);
            let (playing, clock) = &editor.ui.preview_effect;
            if playing.replace(effect_sku) != effect_sku {
                clock.replace(None);
            }
            let overflow = frame
                .map(|(_, item)| super::frame::item_overflow(item, super::POPOUT_WIDTH))
                .unwrap_or_default();
            div()
                .flex()
                .justify_center()
                .pt(rpx(overflow.top.max(16.)))
                .pb(rpx(overflow.bottom))
                .children(app.profile.as_ref().map(|card| {
                    super::popout::style_preview(app, card, effect_item, frame, clock, false, cx)
                }))
                .into_any_element()
        }
    }
}
