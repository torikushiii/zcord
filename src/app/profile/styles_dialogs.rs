use super::image_editor::{ImageEdit, MAX_ZOOM, STAGE};
use super::styles::{
    CollectibleKind, ImageChange, ImageTarget, NameStyle, StyleScope, StylesEditor,
};
use super::styles_panel::{
    ColorPicker, ColorSlot, SOLID_EFFECT, StyleDialog, decoration_of, hsv_to_rgb, measure_into,
    nameplate_of, on_app, own_name, shown, shown_name_style,
};
use super::*;
use crate::{
    app::{
        display_name::{self, Surface},
        gif_picker,
        member_list::{member_nameplate_layer, rounded_nameplate_layer},
    },
    store::NameStyle as ShownNameStyle,
};

const GRID_GAP: f32 = 12.;
const DECORATION_ITEM: [f32; 2] = [75.3, 80.];
const PROFILE_ITEM: [f32; 2] = [83.3, 80.];
const NAMEPLATE_ITEM: [f32; 2] = [224., 42.];
const PICKER_SQUARE: [f32; 2] = [220., 150.];

/// Display name fonts in the dialog's order, by Discord id.
const FONTS: [(&str, u32); 12] = [
    ("gg sans", 11),
    ("Tempo", 12),
    ("Sakura", 3),
    ("Jellybean", 4),
    ("Modern", 6),
    ("Medieval", 7),
    ("8Bit", 8),
    ("Vampyre", 10),
    ("Monkey Bars", 13),
    ("Mainframe", 14),
    ("Headbang", 15),
    ("Journal", 16),
];

const EFFECTS: [(&str, u32); 7] = [
    ("Solid", SOLID_EFFECT),
    ("Gradient", 2),
    ("Neon", 3),
    ("Toon", 4),
    ("Pop", 5),
    ("Gummy", 8),
    ("Prism", 7),
];

const GUMMY_PRESETS: [[u32; 4]; 7] = [
    [0xE9A9D5, 0xA9DDF0, 0xB9A7EE, 0xB9EACB],
    [0xFF6FC2, 0xF54FA8, 0xFF9BD6, 0xD96AB7],
    [0xFFB57A, 0xFF936F, 0xFF7C78, 0xF06B8A],
    [0xC4ED65, 0x98DE60, 0x6CCD63, 0x45BC66],
    [0x7CB8FF, 0x8EE0D0, 0x9D94F5, 0x66CFF0],
    [0xD77BFF, 0xB56BFF, 0xE45CE5, 0x8B6CFF],
    [0xD83A5F, 0xC94BCB, 0x8B5CFF, 0xB06CFF],
];

/// The picker's suggestions for a name color, and for theme colors without an avatar to sample.
pub(super) fn preset_suggestions() -> [u32; 5] {
    std::array::from_fn(|index| GUMMY_PRESETS[index][0])
}

const GRADIENT_PRESETS: [[u32; 2]; 5] = [
    [0x2AAEA6, 0xFFC490],
    [0xE45EFF, 0x267DE0],
    [0x7E4FCB, 0x67CB98],
    [0xEFB4AA, 0xE46CE2],
    [0xF5983C, 0xE55AE7],
];

const PRISM_PRESETS: [[u32; 5]; 3] = [
    [0xFE9242, 0x35ED7E, 0xFFE047, 0xFF4CD2, 0x5865F2],
    [0x7EE7D8, 0x7CCBFF, 0x7F8CFF, 0xB56DFF, 0xF38AD8],
    [0x7A0906, 0xC81E12, 0xF45A18, 0xFF9500, 0xFFD447],
];

/// Every one-color effect shares these swatches, after a default of its own.
const SINGLE_PRESETS: [[u32; 1]; 6] = [
    [0x24E36D],
    [0x1C98EB],
    [0xC32AFF],
    [0xDA0149],
    [0xF93722],
    [0xC6A615],
];

fn single_default(effect_id: u32) -> &'static [u32] {
    match effect_id {
        SOLID_EFFECT => &[0xD4D5D8],
        3 => &[0x691DED],
        4 => &[0xF42098],
        _ => &[0x0FCF86],
    }
}

const HUE_STOPS: [u32; 7] = [
    0xFF0000, 0xFFFF00, 0x00FF00, 0x00FFFF, 0x0000FF, 0xFF00FF, 0xFF0000,
];

/// How many colors Discord stores for each name effect.
fn effect_colors(effect_id: u32) -> usize {
    match effect_id {
        SOLID_EFFECT | 3 | 4 | 5 => 1,
        7 => 5,
        8 => 4,
        _ => 2,
    }
}

/// Keeps the palette when it has enough colors for `effect_id`, else starts from the effect's
/// first preset, so a single color doesn't become a flat gradient.
fn with_effect(style: &NameStyle, effect_id: u32) -> NameStyle {
    let count = effect_colors(effect_id);
    let palette = if style.colors.len() >= count {
        &style.colors[..]
    } else {
        presets(effect_id)[0]
    };
    NameStyle {
        font_id: style.font_id,
        effect_id,
        colors: palette.iter().copied().cycle().take(count).collect(),
    }
}

fn with_preset(style: &NameStyle, preset: &[u32]) -> NameStyle {
    NameStyle {
        colors: preset
            .iter()
            .copied()
            .cycle()
            .take(effect_colors(style.effect_id))
            .collect(),
        ..style.clone()
    }
}

/// Gradient and Prism blend their colors in the swatches and custom tile.
fn blends(effect_id: u32) -> bool {
    matches!(effect_id, 2 | 7)
}

fn presets(effect_id: u32) -> Vec<&'static [u32]> {
    match effect_id {
        2 => GRADIENT_PRESETS.iter().map(|preset| &preset[..]).collect(),
        SOLID_EFFECT | 3..=5 => std::iter::once(single_default(effect_id))
            .chain(SINGLE_PRESETS.iter().map(|preset| &preset[..]))
            .collect(),
        7 => PRISM_PRESETS.iter().map(|preset| &preset[..]).collect(),
        _ => GUMMY_PRESETS.iter().map(|preset| &preset[..]).collect(),
    }
}

/// `colors` across a swatch: blended left to right, or in hard stripes. gpui clips to a
/// rectangle, so the ends round themselves to `radius`.
fn color_fill(colors: &[u32], blend: bool, radius: f32) -> Div {
    let segments: Vec<(u32, u32)> = if blend && colors.len() > 1 {
        colors.windows(2).map(|pair| (pair[0], pair[1])).collect()
    } else {
        colors.iter().map(|color| (*color, *color)).collect()
    };
    let last = segments.len().saturating_sub(1);
    div()
        .absolute()
        .inset_0()
        .flex()
        .children(segments.into_iter().enumerate().map(|(index, (from, to))| {
            div()
                .flex_1()
                .h_full()
                .bg(linear_gradient(
                    90.,
                    linear_color_stop(rgb(from), 0.),
                    linear_color_stop(rgb(to), 1.),
                ))
                .when(index == 0, |this| this.rounded_l(rpx(radius)))
                .when(index == last, |this| this.rounded_r(rpx(radius)))
        }))
}

/// Discord's custom tile for blended effects: the current colors with a handle on each stop.
fn custom_gradient(app: &DiscordApp, style: &NameStyle, selected: bool) -> AnyElement {
    let count = style.colors.len();
    let handles = style.colors.iter().enumerate().map(|(index, &color)| {
        let bounds = Rc::new(Cell::new(None));
        let label: SharedString = match (style.effect_id, index) {
            (2, 0) => "Custom Gradient Start Color".into(),
            (2, _) => "Custom Gradient End Color".into(),
            _ => format!("Custom Gradient Color {}", index + 1).into(),
        };
        div()
            .id(("name-color-stop", index))
            .relative()
            .size(rpx(28.))
            .rounded(rpx(8.))
            .border_2()
            .border_color(gpui::white())
            .bg(rgb(color))
            .cursor_pointer()
            .child(measure_into(bounds.clone()))
            .on_click(on_app(app, move |app, cx| {
                app.open_color_picker(ColorSlot::Name(index), color, bounds.get(), cx)
            }))
            .child(theme::hover_tooltip(
                ("name-color-stop-tooltip", index),
                label,
            ))
    });
    div()
        .id("name-color-custom")
        .relative()
        .flex_none()
        .w(rpx(if count > 2 { 232. } else { 136. }))
        .h(rpx(40.))
        .px(rpx(10.))
        .flex()
        .items_center()
        .justify_between()
        .rounded(rpx(8.))
        .border_2()
        .border_color(if selected {
            rgb(theme::BLURPLE)
        } else {
            rgba(0)
        })
        .child(color_fill(&style.colors, true, 6.))
        .children(handles)
        .into_any_element()
}

/// "Acquired October 2026" from an ISO purchase time.
fn acquired(purchased_at: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let year = purchased_at.get(..4)?;
    let month: usize = purchased_at.get(5..7)?.parse().ok()?;
    Some(format!(
        "Acquired {} {year}",
        MONTHS.get(month.checked_sub(1)?)?
    ))
}

fn button(id: &'static str, label: &'static str, primary: bool, width: f32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .gap(rpx(8.))
        .w(rpx(width))
        .h(rpx(40.))
        .rounded(rpx(8.))
        .border_1()
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .map(|this| {
            if primary {
                this.bg(rgb(theme::BLURPLE))
                    .border_color(rgba(0xFFFFFF14))
                    .text_color(gpui::white())
                    .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
            } else {
                this.bg(rgba(theme::control_bg()))
                    .border_color(rgba(theme::border_soft()))
                    .text_color(rgb(theme::strong()))
                    .hover(|style| style.bg(rgba(theme::control_selected())))
            }
        })
        .child(label)
}

fn section_title(title: &'static str) -> Div {
    div()
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .child(title)
}

/// A dialog over a scrim; clicking the scrim or the close button cancels it. It pops in on
/// open and back out while `close_style_dialog` waits to drop it.
///
/// Without a `title` the body lays out its own heading, edge to edge, under a corner close
/// button.
fn shell(
    app: &DiscordApp,
    editor: &StylesEditor,
    title: Option<&'static str>,
    width: f32,
    body: AnyElement,
    footer: Div,
) -> AnyElement {
    let closing = editor.ui.dialog_closing.is_some();
    let (key, duration) = if closing {
        ("style-dialog-out", crate::app::DIALOG_EXIT)
    } else {
        ("style-dialog-in", ENTER)
    };
    let animation = Animation::new(duration).with_easing(ease_out_quint());
    let progress = move |delta: f32| if closing { 1. - delta } else { delta };
    let scrim_progress = {
        let swapped = editor.ui.dialog_swapped && !closing;
        move |delta: f32| if swapped { 1. } else { progress(delta) }
    };
    let opens = editor.ui.dialog_opens;
    div()
        .id("style-dialog-scrim")
        // Their clicks stop here, so a selection they began would never see its mouse-up.
        .without_text_selection()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_click(on_app(app, |app, cx| app.close_style_dialog(cx)))
        .with_animation((key, opens), animation.clone(), move |this, delta| {
            this.bg(rgba(theme::SCRIM).opacity(scrim_progress(delta)))
        })
        .child(
            div()
                .id("style-dialog")
                .relative()
                .flex()
                .flex_col()
                .w(rpx(width))
                .max_h(relative(0.9))
                .rounded(rpx(12.))
                .border_1()
                .border_color(rgba(theme::border()))
                .bg(rgb(theme::surface_high()))
                .shadow(vec![
                    BoxShadow::new(px(0.), px(4.), hsla(0., 0., 0., 0.16)).blur_radius(px(8.)),
                ])
                .text_color(rgb(theme::strong()))
                .on_click(|_, _, cx| cx.stop_propagation())
                .map(|this| match title {
                    Some(title) => this.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pt(rpx(16.))
                            .px(rpx(24.))
                            .child(
                                div()
                                    .text_size(rpx(20.))
                                    .line_height(rpx(24.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(close_button(app)),
                    ),
                    None => this.overflow_hidden(),
                })
                .child(body)
                .child(
                    footer
                        .flex()
                        .flex_row_reverse()
                        .gap(rpx(8.))
                        .py(rpx(16.))
                        .px(rpx(24.))
                        .rounded_b(rpx(12.)),
                )
                .when(title.is_none(), |this| {
                    this.child(
                        div()
                            .absolute()
                            .top(rpx(12.))
                            .right(rpx(12.))
                            .child(close_button(app)),
                    )
                })
                .with_animation((key, opens), animation, move |this, delta| {
                    let shown = progress(delta);
                    this.opacity(shown).top(rpx(16. * (1. - shown)))
                }),
        )
        .into_any_element()
}

fn close_button(app: &DiscordApp) -> Stateful<Div> {
    theme::icon_button("style-dialog-close", "x", 24., theme::strong())
        .on_click(on_app(app, |app, cx| app.close_style_dialog(cx)))
}

/// The open dialog and color picker, drawn over the whole window.
pub(super) fn style_overlays(app: &DiscordApp, editor: &StylesEditor, cx: &App) -> Vec<AnyElement> {
    let dialog = editor.ui.dialog.as_ref().map(|dialog| match dialog {
        StyleDialog::Collectible { kind, selected } => {
            collectible_dialog(app, editor, *kind, *selected, cx)
        }
        StyleDialog::NameStyle(style) => name_style_dialog(app, editor, style, cx),
        StyleDialog::Image(target) => image_dialog(app, editor, *target, cx),
        StyleDialog::ImageEdit(edit) => image_edit_dialog(app, editor, edit),
    });
    let picker = editor
        .ui
        .color_picker
        .map(|picker| color_picker(app, editor, picker));
    dialog.into_iter().chain(picker).collect()
}

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

/// The None and Shop buttons that lead every collectible list.
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

/// A nameplate as it sits in the list: a stand-in avatar and name over the art.
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

/// One line of a collectible list. Only the lines in view are drawn: off-screen rows of
/// rounded art otherwise leave streaks behind the scroll area, and the shop runs to hundreds.
enum Line {
    Heading(&'static str, f32),
    Loading,
    Cells(Vec<ListCell>),
}

fn item_size(kind: CollectibleKind) -> [f32; 2] {
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

/// Tracks which collectible is under the pointer, so only that one plays its animation.
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
                // The None/Shop pair sits a little apart from the nameplate rows under it.
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

fn collectible_dialog(
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
    // Effects and frames are tried on a whole profile card, which needs the room.
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
    // The ring cuts the status out of the stage behind the avatar.
    let avatar_with = |size: f32, status: bool| {
        theme::preview_avatar(
            user,
            user_id,
            size,
            user.filter(|_| status)
                .map(|user| (user.status, theme::base_lowest())),
            current.avatar.clone(),
            current.avatar_file.clone(),
            decoration.as_ref(),
            cx,
        )
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
            // Five member-list rows, centered on yours, the outer ones cut off by the box.
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
            // A newly picked effect plays from its start.
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

/// Discord's stand-in profile, drawn behind effect and frame grid items.
const SAMPLE_PROFILE: &str = "https://cdn.discordapp.com/assets/content/a859611882903f1102a796c08c68278917821af3e87eb9191df74bf78426971d.png";

fn sample_profile(width: f32, cx: &App) -> Option<Img> {
    let spec = ImageSpec::Collectible {
        url: SAMPLE_PROFILE.to_owned(),
    };
    images::resolve(&spec, theme::image_px(width), cx)
        .map(|path| img(path).object_fit(ObjectFit::Cover))
}

/// An effect over the sample profile, filling its grid item. Hovered, it plays the effect's
/// layers rather than the still thumbnail.
fn effect_grid_art(editor: &StylesEditor, sku_id: u64, animate: bool, cx: &App) -> AnyElement {
    let [width, height] = item_size(CollectibleKind::Effect);
    let pixels = theme::image_px(width.max(height));
    let fill = |this: Div| this.absolute().inset_0().rounded(rpx(4.)).overflow_hidden();
    let item = super::styles_panel::collectible_item(editor, sku_id);
    let layers = item.filter(|_| animate).and_then(|item| {
        item.effects
            .iter()
            .enumerate()
            .map(|(index, layer)| {
                let spec = ImageSpec::Collectible {
                    url: layer.src.clone(),
                };
                let frames = animated::resolve(&spec, pixels, cx)?;
                Some(
                    fill(div()).child(
                        animated::image(frames)
                            .object_fit(ObjectFit::Cover)
                            .size_full()
                            .rounded(rpx(4.))
                            .id(SharedString::from(format!("effect-item-{sku_id}-{index}"))),
                    ),
                )
            })
            .collect::<Option<Vec<_>>>()
            .filter(|layers| !layers.is_empty())
    });
    let still = layers.is_none().then(|| {
        item.and_then(|item| item.thumbnail_preview_src.clone())
            .and_then(|url| images::resolve(&ImageSpec::Collectible { url }, pixels, cx))
            .map(|path| {
                fill(div()).child(
                    img(path)
                        .size_full()
                        .rounded(rpx(4.))
                        .object_fit(ObjectFit::Cover),
                )
            })
    });
    div()
        .absolute()
        .inset_0()
        .children(
            sample_profile(width, cx)
                .map(|image| fill(div()).child(image.size_full().rounded(rpx(4.)))),
        )
        .children(still.flatten())
        .children(layers.into_iter().flatten())
        .into_any_element()
}

fn frame_grid_art(editor: &StylesEditor, sku_id: u64, cx: &App) -> AnyElement {
    const WIDTH: f32 = 50.;
    const HEIGHT: f32 = 68.;
    let frame = editor
        .find_collectible(sku_id)
        .and_then(|owned| super::frame::frame_item(&owned.product));
    let overflow = frame
        .map(|(_, item)| super::frame::item_overflow(item, WIDTH))
        .unwrap_or_default();
    let card = div()
        .w(rpx(WIDTH))
        .h(rpx((HEIGHT - overflow.top - overflow.bottom).max(32.)))
        .rounded(rpx(4.))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::elevated()))
        .overflow_hidden()
        .children(sample_profile(WIDTH, cx).map(|image| image.size_full().rounded(rpx(4.))))
        .into_any_element();
    div()
        .pt(rpx(overflow.top))
        .pb(rpx(overflow.bottom))
        .child(super::frame::framed_with(frame, WIDTH, card, false, cx))
        .into_any_element()
}

fn set_name_style(
    app: &DiscordApp,
    style: NameStyle,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    on_app(app, move |app, cx| {
        let style = style.clone();
        app.update_styles_ui(|ui| ui.dialog = Some(StyleDialog::NameStyle(style)), cx)
    })
}

/// A font or effect tile: Discord's 4-column grid in the 384px options column.
const NAME_TILE: [f32; 2] = [90., 68.];
const NEW_FONTS: [u32; 2] = [15, 16];

/// Discord's own colors on each effect's tile, whatever colors the user picked.
fn effect_sample(effect_id: u32) -> Vec<u32> {
    match effect_id {
        SOLID_EFFECT => vec![0xD4D5D8],
        2 => vec![0x2AAEA6, 0xFFC490],
        3 => vec![0x7935EF],
        4 => vec![0xF42098],
        5 => vec![0x0FCF86],
        8 => GUMMY_PRESETS[0].to_vec(),
        _ => PRISM_PRESETS[0].to_vec(),
    }
}

fn option_tile(id: ElementId, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(rpx(NAME_TILE[0]))
        .h(rpx(NAME_TILE[1]))
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .rounded(rpx(8.))
        .border_1()
        .border_color(if selected {
            rgb(theme::BLURPLE)
        } else {
            rgba(0x97979F1F)
        })
        .bg(rgba(0x97979F14))
        .shadow(vec![
            BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., 0.14)).blur_radius(px(4.)),
        ])
        .cursor_pointer()
        .hover(|style| style.bg(rgba(0x97979F1F)))
}

fn selector_heading(title: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(6.))
        .mb(rpx(8.))
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()))
        .child(title)
        .child(theme::icon("nitro", 16., theme::strong()))
}

fn name_style_dialog(
    app: &DiscordApp,
    editor: &StylesEditor,
    style: &NameStyle,
    cx: &App,
) -> AnyElement {
    let fonts = FONTS.iter().map(|&(font, font_id)| {
        let sample = ShownNameStyle {
            font_id: Some(font_id),
            effect_id: None,
            colors: Vec::new(),
        };
        let selected = style.font_id == font_id;
        option_tile(("name-font", font_id as u64).into(), selected)
            .on_click(set_name_style(
                app,
                NameStyle {
                    font_id,
                    ..style.clone()
                },
            ))
            .child(
                div()
                    .text_size(rpx(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(if selected {
                        theme::strong()
                    } else {
                        theme::text()
                    }))
                    .child(display_name::render(
                        ("name-font-sample", font_id as u64),
                        "Gg",
                        Some(&sample),
                        Surface::FontOnly,
                        cx,
                    )),
            )
            .when(NEW_FONTS.contains(&font_id), |this| {
                this.child(
                    div()
                        .absolute()
                        .top(rpx(4.))
                        .right(rpx(4.))
                        .size(rpx(4.))
                        .rounded_full()
                        .bg(rgb(theme::BLURPLE))
                        .shadow(vec![
                            BoxShadow::new(px(0.), px(0.), rgb(theme::BLURPLE).into())
                                .blur_radius(px(4.)),
                        ]),
                )
            })
            .child(theme::hover_tooltip(
                ("name-font-tooltip", font_id as usize),
                font,
            ))
            .into_any_element()
    });
    let effects = EFFECTS.iter().map(|&(label, effect_id)| {
        let sample = ShownNameStyle {
            font_id: None,
            effect_id: Some(effect_id),
            colors: effect_sample(effect_id),
        };
        option_tile(
            ("name-effect", effect_id as u64).into(),
            style.effect_id == effect_id,
        )
        .on_click(set_name_style(app, with_effect(style, effect_id)))
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .child(display_name::render(
                    ("name-effect-sample", effect_id as u64),
                    label,
                    Some(&sample),
                    Surface::Account { hovered: true },
                    cx,
                )),
        )
        .into_any_element()
    });
    let swatch_border = |selected: bool| {
        if selected {
            rgb(theme::BLURPLE)
        } else {
            rgba(0)
        }
    };
    let presets = presets(style.effect_id);
    let custom_selected = !style.colors.is_empty()
        && !presets
            .iter()
            .any(|preset| with_preset(style, preset).colors == style.colors);
    let swatches = presets.iter().enumerate().map(|(index, preset)| {
        let selected = with_preset(style, preset).colors == style.colors;
        div()
            .id(("name-color", index))
            .flex_none()
            .size(rpx(40.))
            .flex()
            .overflow_hidden()
            .rounded(rpx(8.))
            .border_2()
            .border_color(swatch_border(selected))
            .cursor_pointer()
            .on_click(set_name_style(app, with_preset(style, preset)))
            .child(color_fill(preset, blends(style.effect_id), 6.))
            .into_any_element()
    });
    let custom = if blends(style.effect_id) {
        custom_gradient(app, style, custom_selected)
    } else {
        let color = style.colors.first().copied().unwrap_or(GUMMY_PRESETS[2][0]);
        let bounds = Rc::new(Cell::new(None));
        div()
            .id("name-color-custom")
            .relative()
            .flex_none()
            .size(rpx(40.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(rpx(8.))
            .map(|this| {
                if custom_selected {
                    this.border_2()
                        .border_color(rgb(theme::BLURPLE))
                        .bg(rgb(color))
                } else {
                    this.border_1()
                        .border_color(rgba(0x97979F3D))
                        .bg(rgba(0x97979F1F))
                }
            })
            .cursor_pointer()
            .child(measure_into(bounds.clone()))
            .on_click(on_app(app, move |app, cx| {
                app.open_color_picker(ColorSlot::Name(0), color, bounds.get(), cx)
            }))
            .child(theme::icon("pipette", 14., 0xFFFFFF))
            .into_any_element()
    };
    let colors = div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .child(custom)
        .children(swatches);
    let grid = |tiles: Vec<AnyElement>| {
        div()
            .flex()
            .flex_wrap()
            .gap(rpx(8.))
            .w(rpx(384.))
            .children(tiles)
    };
    let section = |title: &'static str, content: Div| {
        div()
            .flex()
            .flex_col()
            .mt(rpx(24.))
            .child(selector_heading(title))
            .child(content)
    };
    let options = div()
        .id("name-style-options")
        .flex()
        .flex_col()
        .flex_none()
        .w(rpx(424.))
        .h_full()
        .overflow_y_scroll()
        .pt(rpx(24.))
        .pb(rpx(24.))
        .pl(rpx(24.))
        .pr(rpx(16.))
        .child(
            div()
                .mb(rpx(8.))
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Change Display Name Style"),
        )
        .child(section("Choose Font", grid(fonts.collect())))
        .child(section("Choose Effect", grid(effects.collect())))
        .child(section("Choose Color", colors));
    let body = div()
        .flex()
        .min_h_0()
        .h(rpx(631.))
        .child(options)
        .child(name_style_preview(app, editor, style, cx))
        .into_any_element();
    let footer = div()
        .border_t_1()
        .border_color(rgba(theme::border_subtle()))
        .child(
            button("name-style-apply", "Apply", true, 139.)
                .on_click(on_app(app, |app, cx| app.apply_style_dialog(cx))),
        )
        .child({
            let style = style.clone();
            button("name-style-surprise", "Surprise Me", false, 139.)
                .child(theme::icon("dices", 16., theme::strong()))
                .on_click(on_app(app, move |app, cx| {
                    let next = surprise(&style);
                    app.update_styles_ui(|ui| ui.dialog = Some(StyleDialog::NameStyle(next)), cx)
                }))
        });
    shell(app, editor, None, 836., body, footer)
}

/// A random font, effect and preset, never the current combination.
fn surprise(current: &NameStyle) -> NameStyle {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos() as usize);
    let font = FONTS[seed % FONTS.len()].1;
    let effect = EFFECTS[(seed / FONTS.len()) % EFFECTS.len()].1;
    let presets = presets(effect);
    let preset = presets[(seed / 97) % presets.len()];
    let next = with_preset(
        &NameStyle {
            font_id: font,
            effect_id: effect,
            colors: Vec::new(),
        },
        preset,
    );
    if next == *current {
        with_effect(
            &next,
            if effect == SOLID_EFFECT {
                2
            } else {
                SOLID_EFFECT
            },
        )
    } else {
        next
    }
}

const NAME_PREVIEW_WIDTH: f32 = 410.;
/// Discord's preview: the profile card, a chat message and a member row overlapping down the
/// right of the dialog, over the profile's theme and a faded banner, with a toggle that shows
/// the message and member row in light mode.
fn name_style_preview(
    app: &DiscordApp,
    editor: &StylesEditor,
    style: &NameStyle,
    cx: &App,
) -> AnyElement {
    let user_id = app.store.current_user_id;
    let user = app.store.user(user_id);
    let current = shown(app, editor);
    let name = own_name(app);
    let shown_style = shown_name_style(style);
    let light = editor.ui.name_preview_light;
    let decoration = current
        .decoration
        .and_then(|sku| decoration_of(app, editor, sku));
    let avatar = |size: f32, decorated: bool| {
        theme::preview_avatar(
            user,
            user_id,
            size,
            None,
            current.avatar.clone(),
            current.avatar_file.clone(),
            decoration.as_ref().filter(|_| decorated),
            cx,
        )
    };
    let card = app.profile.as_ref();
    let backdrop = card
        .and_then(|card| card.theme())
        .map_or_else(|| rgb(theme::bg()).into(), |theme| theme.modal_background());
    let banner = current
        .banner_file
        .clone()
        .map(|path| ImageSpec::LocalGif { path })
        .or_else(|| current.banner.clone())
        .and_then(|spec| super::banner_backdrop(&spec, NAME_PREVIEW_WIDTH, 0., cx));
    let frame = current
        .frame
        .and_then(|sku| editor.find_collectible(sku))
        .map(|owned| owned.product.clone());
    let profile = card.map(|card| {
        super::popout::style_preview(
            app,
            card,
            None,
            frame.as_deref().and_then(super::frame::frame_item),
            &editor.ui.preview_effect.1,
            true,
            cx,
        )
    });
    let (surface, border, text, muted) = if light {
        (0xFFFFFF, rgba(0x0000001F), 0x313338, 0x5C5E66)
    } else {
        (
            theme::surface_high(),
            rgba(theme::border_subtle()),
            theme::text(),
            theme::muted(),
        )
    };
    let now = chrono::Local::now().timestamp();
    let message = div()
        .relative()
        .ml(rpx(25.))
        .mt(rpx(-10.))
        .mb(rpx(-5.))
        .min_w(rpx(320.))
        .py(rpx(20.))
        .pl(rpx(70.))
        .pr(rpx(20.))
        .rounded(rpx(8.))
        .border_1()
        .border_color(border)
        .bg(rgb(surface))
        .child(
            div()
                .absolute()
                .left(rpx(16.))
                .top(rpx(22.))
                .child(avatar(40., true)),
        )
        .child(
            div()
                .flex()
                .items_baseline()
                .gap(rpx(4.))
                .child(
                    div()
                        .max_w(rpx(180.))
                        .text_size(rpx(16.))
                        .line_height(rpx(22.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if light { 0x060607 } else { theme::strong() }))
                        .child(display_name::render(
                            "name-style-preview-message",
                            &name,
                            Some(&shown_style),
                            Surface::Account { hovered: true },
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(rpx(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(muted))
                        .child(crate::text::timestamp::format_timestamp(now, 't')),
                ),
        )
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(22.))
                .text_color(rgb(text))
                .child("does anyone read this?"),
        );
    let plate = current
        .nameplate
        .and_then(|sku| nameplate_of(app, editor, sku))
        .and_then(|plate| member_nameplate_layer(&plate, user_id, 42., true, 8., cx));
    let member = div()
        .relative()
        .w(rpx(280.))
        .h(rpx(44.))
        .overflow_hidden()
        .rounded(rpx(8.))
        .border_1()
        .border_color(border)
        .bg(rgb(if light {
            0xEBEBED
        } else {
            theme::surface_highest()
        }))
        .children(plate)
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .pl(rpx(8.))
                .pr(rpx(32.))
                .child(avatar(32., true))
                .child(
                    div()
                        .min_w_0()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if light { 0x060607 } else { theme::strong() }))
                        .child(display_name::render(
                            "name-style-preview-row",
                            &name,
                            Some(&shown_style),
                            Surface::Account { hovered: true },
                            cx,
                        )),
                )
                .children(
                    user.and_then(|user| user.tag.as_ref())
                        .map(|tag| theme::guild_tag(tag, cx)),
                ),
        );
    let toggle_option = |id: &'static str, icon: &'static str, selected: bool| {
        div()
            .id(id)
            .size(rpx(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(rpx(8.))
            .cursor_pointer()
            .map(|this| {
                if selected {
                    this.border_1()
                        .border_color(rgba(0x97979F29))
                        .bg(rgb(theme::surface_high()))
                } else {
                    this.hover(|style| style.bg(rgba(0x97979F1F)))
                }
            })
            .on_click(on_app(app, move |app, cx| {
                let light = icon == "theme-light";
                app.update_styles_ui(|ui| ui.name_preview_light = light, cx)
            }))
            .child(theme::icon(
                icon,
                16.,
                if selected { 0xDCDCDF } else { 0x96979E },
            ))
    };
    // `min_w_0` and the clip hold it to its column; otherwise the note's unwrapped
    // width and the frame art stretch it past the dialog's edge.
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_hidden()
        .p(rpx(12.))
        .bg(gpui::black())
        .child(div().absolute().inset_0().bg(backdrop))
        .children(banner)
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .justify_center()
                .mx_auto()
                .mt(rpx(32.))
                .pl(rpx(16.))
                .children(profile.map(|profile| div().w(rpx(300.)).child(profile)))
                .child(message)
                .child(member),
        )
        .child(
            div()
                .relative()
                .flex()
                .items_center()
                .justify_end()
                .px(rpx(16.))
                .pb(rpx(16.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(theme::muted()))
                        .child(
                            "Note: Display name colors and effects will not appear in servers. \
                             Styles may also look different across Discord and light/dark mode.",
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(rpx(4.))
                        .ml(rpx(16.))
                        .p(rpx(4.))
                        .rounded(rpx(12.))
                        .bg(gpui::black())
                        .child(toggle_option("name-preview-dark", "theme-dark", !light))
                        .child(toggle_option("name-preview-light", "theme-light", light)),
                ),
        )
        .into_any_element()
}

const IMAGE_OPTION: f32 = 211.;
const RECENT_AVATAR: f32 = 56.;
const RECENT_AVATAR_SLOTS: usize = 6;

fn image_dialog(
    app: &DiscordApp,
    editor: &StylesEditor,
    target: ImageTarget,
    cx: &App,
) -> AnyElement {
    let user_id = app.store.current_user_id;
    // Banners and animated avatars both need Nitro, so it badges every option it gates.
    let upload_premium = target == ImageTarget::Banner;
    let option = |id: &'static str, enabled: bool| {
        div()
            .id(id)
            .group(id)
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(rpx(IMAGE_OPTION))
            .rounded(rpx(8.))
            .bg(rgba(0x97979F33))
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(rgba(0x97979F4D)))
            })
            .when(!enabled, |this| this.opacity(0.5))
    };
    let label = |icon: AnyElement, text: &'static str, premium: bool| {
        div()
            .relative()
            .flex()
            .flex_col()
            .items_center()
            .gap(rpx(8.))
            .child(icon)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .when(premium, |this| {
                        this.child(theme::icon("nitro", 16., theme::strong()))
                    })
                    .child(text),
            )
    };
    let upload = option("image-upload", editor.nitro || !upload_premium)
        .when(editor.nitro || !upload_premium, |this| {
            // The dialog stays up behind the file picker; a picked image replaces it with
            // the editor, and cancelling leaves it open.
            this.on_click(on_app(app, move |app, cx| app.pick_style_image(target, cx)))
        })
        .child(label(
            theme::icon("image-plus", 24., theme::strong()).into_any_element(),
            "Upload Image",
            upload_premium,
        ));
    let half = IMAGE_OPTION / 2.;
    let mosaic = gif_picker::category_previews(app, 4, cx)
        .into_iter()
        .enumerate()
        .map(|(index, src)| {
            let corners = match index {
                0 => [8., 0., 0., 0.],
                1 => [0., 8., 0., 0.],
                2 => [0., 0., 0., 8.],
                _ => [0., 0., 8., 0.],
            };
            div()
                .absolute()
                .left(rpx(if index % 2 == 0 { 0. } else { half }))
                .top(rpx(if index < 2 { 0. } else { half }))
                .size(rpx(half))
                .child(gif_picker::gif_cell(&src, half, corners, cx))
        });
    let gif_badge = div()
        .px(rpx(4.))
        .h(rpx(18.))
        .flex()
        .items_center()
        .rounded(rpx(4.))
        .bg(rgb(0xFFFFFF))
        .text_size(rpx(11.))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(0x000000))
        .child("GIF");
    let entity = app.entity.clone();
    let gif = option("image-gif", editor.nitro)
        .when(editor.nitro, |this| {
            this.on_click(move |_, window, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| {
                        app.close_style_dialog(cx);
                        app.open_style_gif_picker(target, window, cx);
                    })
                    .ok();
            })
        })
        .children(mosaic)
        // Dims the stills so the label reads over bright frames; lifts a little on hover.
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(rpx(8.))
                .bg(rgba(0x00000080))
                .group_hover("image-gif", |style| style.bg(rgba(0x00000059))),
        )
        .child(label(gif_badge.into_any_element(), "Choose GIF", true));
    let recent = (target == ImageTarget::Avatar)
        .then(|| editor.recent_avatars.clone())
        .flatten()
        .map(|avatars| {
            let slots = avatars.iter().take(RECENT_AVATAR_SLOTS).map(|avatar| {
                let (id, storage_hash) = (avatar.id, avatar.storage_hash.clone());
                let spec = ImageSpec::ArchivedAvatar {
                    user_id,
                    avatar_id: id,
                    hash: storage_hash.to_string(),
                };
                let pixels = theme::image_px(RECENT_AVATAR);
                // Animated uploads play here, as on Discord; the still shows until they load.
                let image = spec
                    .animated()
                    .and_then(|animated| animated::resolve(&animated, pixels, cx))
                    .map(|frames| {
                        animated::image(frames)
                            .object_fit(ObjectFit::Cover)
                            .id(("recent-avatar-frames", id))
                            .size_full()
                            .rounded_full()
                            .into_any_element()
                    })
                    .or_else(|| {
                        images::resolve(&spec, pixels, cx).map(|path| {
                            img(path)
                                .size_full()
                                .rounded_full()
                                .object_fit(ObjectFit::Cover)
                                .into_any_element()
                        })
                    });
                div()
                    .id(("recent-avatar", id))
                    .size(rpx(RECENT_AVATAR))
                    .flex_none()
                    .rounded_full()
                    .bg(rgb(theme::base_lowest()))
                    .cursor_pointer()
                    .hover(|style| style.opacity(0.8))
                    // Like an upload, it opens in the editor; left untouched it is reused.
                    .on_click(on_app(app, move |app, cx| {
                        app.edit_recent_avatar(id, storage_hash.clone(), cx)
                    }))
                    // gpui clips children square; each picture rounds itself.
                    .children(image)
            });
            let empty = RECENT_AVATAR_SLOTS.saturating_sub(avatars.len());
            let placeholders = (0..empty).map(|_| {
                div()
                    .size(rpx(RECENT_AVATAR))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(rgba(0x97979F33))
                    .child(theme::icon("user", 28., theme::muted()))
            });
            div()
                .flex()
                .flex_col()
                .gap(rpx(12.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(4.))
                        .child(section_title("Recent Avatars"))
                        .child(
                            div()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .text_color(rgb(theme::muted()))
                                .child("Access your 6 most recent avatar uploads."),
                        ),
                )
                .child(
                    div()
                        .mt(rpx(4.))
                        .flex()
                        .gap(rpx(8.))
                        .children(slots)
                        .children(placeholders),
                )
        });
    let body = div()
        .flex()
        .flex_col()
        .gap(rpx(24.))
        .pt(rpx(16.))
        .px(rpx(24.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(8.))
                .child(div().flex().gap(rpx(8.)).child(upload).child(gif))
                .when(target == ImageTarget::Banner, |this| {
                    this.child(
                        div()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .text_color(rgb(theme::muted()))
                            .child(
                                "Upload a PNG, JPG, or GIF under 10MB. Images should be at \
                                 least 680x240.",
                            ),
                    )
                }),
        )
        .children(recent)
        .into_any_element();
    // Discord's main profile can't drop an image here, but a server profile can fall back.
    let saved = app.saved_style_values();
    let has_own = match target {
        ImageTarget::Avatar => saved.avatar.is_some() || editor.draft.avatar.is_some(),
        ImageTarget::Banner => saved.banner.is_some() || editor.draft.banner.is_some(),
    };
    let footer = div().when(
        has_own && matches!(editor.scope, StyleScope::Guild(_)),
        |this| {
            this.child(
                button("image-remove", "Reset to main profile", false, 180.).on_click(on_app(
                    app,
                    move |app, cx| {
                        app.close_style_dialog(cx);
                        app.update_style_draft(
                            |draft| match target {
                                ImageTarget::Avatar => draft.avatar = Some(ImageChange::Remove),
                                ImageTarget::Banner => draft.banner = Some(ImageChange::Remove),
                            },
                            cx,
                        );
                    },
                )),
            )
        },
    );
    shell(app, editor, Some("Select an Image"), 480., body, footer)
}

/// Discord dims everything outside the frame with a 9999px box-shadow. gpui would paint that
/// under the frame as well, so the dimming is a border this thick whose inner edge is the frame.
const IMAGE_EDIT_DIM: f32 = 480.;
const ZOOM_SLIDER: f32 = 128.;
/// The slider's handle stops this far from either end of its bar.
const ZOOM_TRACK_INSET: f32 = 5.;

fn image_edit_dialog(app: &DiscordApp, editor: &StylesEditor, edit: &ImageEdit) -> AnyElement {
    let [image_width, image_height] = edit.image_size();
    let [frame_width, frame_height] = edit.frame_size();
    let frame_left = (STAGE[0] - frame_width) / 2.;
    let frame_top = (STAGE[1] - frame_height) / 2.;
    let round = edit.target == ImageTarget::Avatar;
    let (press, pan, release, release_out) = (
        app.entity.clone(),
        app.entity.clone(),
        app.entity.clone(),
        app.entity.clone(),
    );
    let stage = div()
        .id("image-edit-stage")
        .relative()
        .flex_none()
        .w(rpx(STAGE[0]))
        .h(rpx(STAGE[1]))
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(gpui::black())
        .cursor(CursorStyle::OpenHand)
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            let position = event.position;
            press
                .update(cx, |app, cx| app.start_image_drag(position, cx))
                .ok();
        })
        .on_mouse_move(move |event, window, cx| {
            if event.dragging() {
                let (position, scale) = (
                    event.position,
                    f32::from(rpx(1.).to_pixels(window.rem_size())),
                );
                pan.update(cx, |app, cx| app.drag_image(position, scale, cx))
                    .ok();
            }
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
            release.update(cx, |app, cx| app.end_image_drag(cx)).ok();
        })
        .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
            release_out
                .update(cx, |app, cx| app.end_image_drag(cx))
                .ok();
        })
        .child(
            // Plays animated picks frame by frame; a still is one frame.
            animated::image(edit.preview.clone())
                .object_fit(ObjectFit::Fill)
                .id("image-edit-preview")
                .absolute()
                .left(rpx((STAGE[0] - image_width) / 2. + edit.offset[0]))
                .top(rpx((STAGE[1] - image_height) / 2. + edit.offset[1]))
                .w(rpx(image_width))
                .h(rpx(image_height)),
        )
        .child(
            div()
                .absolute()
                .left(rpx(frame_left - IMAGE_EDIT_DIM))
                .top(rpx(frame_top - IMAGE_EDIT_DIM))
                .w(rpx(frame_width + IMAGE_EDIT_DIM * 2.))
                .h(rpx(frame_height + IMAGE_EDIT_DIM * 2.))
                .border(rpx(IMAGE_EDIT_DIM))
                .border_color(rgba(0x2F313699))
                .when(round, |this| {
                    this.rounded(rpx(frame_width / 2. + IMAGE_EDIT_DIM))
                }),
        )
        .child(
            div()
                .absolute()
                .left(rpx(frame_left))
                .top(rpx(frame_top))
                .w(rpx(frame_width))
                .h(rpx(frame_height))
                .border(rpx(5.))
                .border_color(gpui::white())
                .when(round, |this| this.rounded_full()),
        );
    let slider_bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
    let entity = app.entity.clone();
    let measured = slider_bounds.clone();
    let zoom_at = move |position: Point<Pixels>, cx: &mut App| {
        let Some(bar) = measured.get() else {
            return;
        };
        let inset = bar.size.width * (ZOOM_TRACK_INSET / ZOOM_SLIDER);
        let travel = bar.size.width - inset * 2.;
        let t = ((position.x - bar.origin.x - inset) / travel).clamp(0., 1.);
        entity
            .update(cx, |app, cx| {
                app.set_image_zoom(1. + t * (MAX_ZOOM - 1.), cx)
            })
            .ok();
    };
    let progress = (edit.zoom - 1.) / (MAX_ZOOM - 1.);
    let zoom_slider = {
        let (down, moved) = (zoom_at.clone(), zoom_at);
        div()
            .id("image-edit-zoom")
            .relative()
            .flex_none()
            .w(rpx(ZOOM_SLIDER))
            .h(rpx(24.))
            .cursor(CursorStyle::ResizeLeftRight)
            .child(measure_into(slider_bounds))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(rpx(10.))
                    .w_full()
                    .h(rpx(4.))
                    .rounded(rpx(4.))
                    .overflow_hidden()
                    .bg(rgb(0x383940))
                    .child(
                        div()
                            .h_full()
                            .w(rpx(ZOOM_SLIDER * progress))
                            .border_1()
                            .border_color(rgba(0xFFFFFF1A))
                            .bg(rgb(theme::BLURPLE)),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(rpx(4.))
                    .left(rpx(ZOOM_TRACK_INSET
                        + (ZOOM_SLIDER - ZOOM_TRACK_INSET * 2.) * progress
                        - 8.))
                    .size(rpx(16.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgba(0x97979F3D))
                    .bg(gpui::white())
                    .shadow(vec![
                        BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., 0.14)).blur_radius(px(4.)),
                    ]),
            )
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                down(event.position, cx)
            })
            .on_mouse_move(move |event, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    moved(event.position, cx);
                }
            })
    };
    let zoom_icon = |size: f32| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(rpx(24.))
            .child(theme::icon("image-scale", size, 0x96979E))
    };
    let rotate = div()
        .id("image-edit-rotate")
        .group("image-edit-rotate")
        .relative()
        .flex_none()
        .p(rpx(6.))
        .cursor_pointer()
        .on_click(on_app(app, |app, cx| app.rotate_image_edit(cx)))
        .child(
            svg()
                .path("app/rotate-square.svg")
                .size(rpx(24.))
                .text_color(rgb(0x96979E))
                .group_hover("image-edit-rotate", |style| style.text_color(rgb(0xDCDCDF))),
        )
        .child(theme::hover_tooltip("image-edit-rotate-tooltip", "Rotate"));
    let tools = div()
        .flex()
        .items_center()
        .mx(rpx(-6.))
        .py(rpx(16.))
        .pl(rpx(36.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap(rpx(8.))
                .mx_auto()
                .child(zoom_icon(12.))
                .child(zoom_slider)
                .child(zoom_icon(24.)),
        )
        .child(rotate);
    let body = div()
        .flex()
        .flex_col()
        .pt(rpx(16.))
        .px(rpx(24.))
        .child(stage)
        .child(tools)
        .children(edit.error.clone().map(|error| {
            div()
                .pb(rpx(8.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::RED))
                .child(error)
        }))
        .into_any_element();
    let edited = edit.edited() && !edit.applying;
    let footer = div()
        .child(
            button("image-edit-apply", "Apply", true, 100.)
                .when(edit.applying, |this| this.opacity(0.5).cursor_default())
                .when(!edit.applying, |this| {
                    this.on_click(on_app(app, |app, cx| app.apply_image_edit(cx)))
                }),
        )
        .child(
            button("image-edit-cancel", "Cancel", false, 100.)
                .on_click(on_app(app, |app, cx| app.close_style_dialog(cx))),
        )
        // The shell's footer runs right to left; this pushes Reset to the far left. An auto
        // margin would be simpler, but Taffy misplaces it in a reversed row.
        .child(div().flex_1())
        .child(
            div()
                .id("image-edit-reset")
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(0x6374F4))
                .when(!edited, |this| this.opacity(0.5))
                .when(edited, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.underline())
                        .on_click(on_app(app, |app, cx| app.reset_image_edit(cx)))
                })
                .child("Reset"),
        )
        .items_center();
    shell(app, editor, Some("Edit Image"), 480., body, footer)
}

fn color_picker(app: &DiscordApp, editor: &StylesEditor, picker: ColorPicker) -> AnyElement {
    let [hue, saturation, value] = picker.hsv;
    let color = hsv_to_rgb(picker.hsv);
    let bounds = editor.ui.picker_bounds.clone();
    let square_bounds = bounds.square.clone();
    let hue_bounds = bounds.hue.clone();
    let entity = app.entity.clone();
    let pick_square = move |position: Point<Pixels>, cx: &mut App| {
        let Some(area) = square_bounds.get() else {
            return;
        };
        let x = ((position.x - area.origin.x) / area.size.width).clamp(0., 1.);
        let y = ((position.y - area.origin.y) / area.size.height).clamp(0., 1.);
        entity
            .update(cx, |app, cx| app.pick_style_color([hue, x, 1. - y], cx))
            .ok();
    };
    let entity = app.entity.clone();
    let pick_hue = move |position: Point<Pixels>, cx: &mut App| {
        let Some(bar) = hue_bounds.get() else {
            return;
        };
        let x = ((position.x - bar.origin.x) / bar.size.width).clamp(0., 1.);
        entity
            .update(cx, |app, cx| {
                app.pick_style_color([x * 359.9, saturation, value], cx)
            })
            .ok();
    };
    let measure = |cell: Rc<Cell<Option<Bounds<Pixels>>>>| {
        canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
            .absolute()
            .size_full()
    };
    let square = {
        let (down, moved) = (pick_square.clone(), pick_square);
        div()
            .id("color-square")
            .relative()
            .w(rpx(PICKER_SQUARE[0]))
            .h(rpx(PICKER_SQUARE[1]))
            .rounded(rpx(3.))
            .overflow_hidden()
            .bg(rgb(hsv_to_rgb([hue, 1., 1.])))
            .cursor_crosshair()
            .child(measure(bounds.square.clone()))
            .child(div().absolute().inset_0().bg(linear_gradient(
                90.,
                linear_color_stop(gpui::white(), 0.),
                linear_color_stop(hsla(0., 0., 1., 0.), 1.),
            )))
            .child(div().absolute().inset_0().bg(linear_gradient(
                0.,
                linear_color_stop(gpui::black(), 0.),
                linear_color_stop(hsla(0., 0., 0., 0.), 1.),
            )))
            .child(
                div()
                    .absolute()
                    .left(rpx(PICKER_SQUARE[0] * saturation - 4.))
                    .top(rpx(PICKER_SQUARE[1] * (1. - value) - 4.))
                    .size(rpx(8.))
                    .rounded_full()
                    .border_2()
                    .border_color(gpui::white()),
            )
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                down(event.position, cx)
            })
            .on_mouse_move(move |event, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    moved(event.position, cx);
                }
            })
    };
    let hue_bar = {
        let (down, moved) = (pick_hue.clone(), pick_hue);
        div()
            .id("color-hue")
            .relative()
            .w(rpx(PICKER_SQUARE[0]))
            .h(rpx(8.))
            .flex()
            .rounded(rpx(3.))
            .cursor_pointer()
            .child(measure(bounds.hue.clone()))
            .children(HUE_STOPS.windows(2).map(|pair| {
                div().flex_1().h_full().bg(linear_gradient(
                    90.,
                    linear_color_stop(rgb(pair[0]), 0.),
                    linear_color_stop(rgb(pair[1]), 1.),
                ))
            }))
            .child(
                div()
                    .absolute()
                    .top(rpx(-4.))
                    .left(rpx(PICKER_SQUARE[0] * hue / 360. - 4.))
                    .w(rpx(8.))
                    .h(rpx(16.))
                    .rounded(rpx(3.))
                    .bg(gpui::white())
                    .shadow(vec![
                        BoxShadow::new(px(0.), px(0.), hsla(0., 0., 0., 0.6)).blur_radius(px(2.)),
                    ]),
            )
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                down(event.position, cx)
            })
            .on_mouse_move(move |event, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    moved(event.position, cx);
                }
            })
    };
    // Theme colors suggest from the avatar, as Discord's do; a blank row holds the space while
    // it's sampled.
    let suggestions = match picker.slot {
        ColorSlot::Name(_) => Some(preset_suggestions()),
        _ => editor
            .ui
            .theme_suggestions
            .as_ref()
            .and_then(|(_, colors)| *colors),
    };
    let suggestions = suggestions
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, suggestion)| {
            div()
                .id(("color-suggestion", index))
                .size(rpx(34.))
                .rounded(rpx(4.))
                .border_1()
                .border_color(rgb(0x84858D))
                .bg(rgb(suggestion))
                .cursor_pointer()
                .on_click(on_app(app, move |app, cx| {
                    app.pick_style_color(super::styles_panel::rgb_to_hsv(suggestion), cx)
                }))
        });
    let panel = div()
        .id("color-picker")
        .flex()
        .flex_col()
        .gap(rpx(16.))
        .p(rpx(16.))
        .rounded(rpx(4.))
        .border_1()
        .border_color(rgba(theme::border()))
        .bg(rgb(theme::base_lowest()))
        .shadow(vec![
            BoxShadow::new(px(0.), px(8.), hsla(0., 0., 0., 0.24)).blur_radius(px(16.)),
        ])
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(16.))
                .child(square)
                .child(hue_bar),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .h(rpx(40.))
                .px(rpx(8.))
                .rounded(rpx(8.))
                .bg(rgba(0x0000001F))
                .border_1()
                .border_color(rgba(theme::border_subtle()))
                .child(
                    div()
                        .text_size(rpx(16.))
                        .text_color(rgb(theme::muted()))
                        .child("#"),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(rpx(16.))
                        .text_color(rgb(theme::strong()))
                        .child(format!("{color:06x}")),
                )
                .child(div().size(rpx(20.)).rounded(rpx(4.)).bg(rgb(color))),
        )
        .child(
            div()
                .flex()
                .justify_center()
                .gap(rpx(12.))
                .h(rpx(34.))
                .children(suggestions),
        );
    let scrim = div()
        .id("color-picker-scrim")
        .without_text_selection()
        .absolute()
        .inset_0()
        .occlude()
        .on_click(on_app(app, |app, cx| app.close_color_picker(cx)));
    // Beside the swatch that opened it, like Discord's popout, moved inward where it won't fit.
    match picker.anchor {
        Some(anchor) => scrim
            .child(
                anchored()
                    .position(point(anchor.right() + px(8.), anchor.top()))
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .into_any_element(),
        None => scrim
            .flex()
            .items_center()
            .justify_center()
            .child(panel)
            .into_any_element(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EFFECTS, GRADIENT_PRESETS, NameStyle, PRISM_PRESETS, SOLID_EFFECT, acquired, effect_colors,
        with_effect,
    };

    #[test]
    fn acquired_reads_the_purchase_month() {
        assert_eq!(
            acquired("2026-10-30T14:05:14.465983+00:00").as_deref(),
            Some("Acquired October 2026")
        );
        assert_eq!(acquired("bad"), None);
    }

    #[test]
    fn switching_effects_keeps_the_palette_and_fits_its_size() {
        let palette = vec![0x111111, 0x222222, 0x333333, 0x444444, 0x555555];
        let style = NameStyle {
            font_id: 14,
            effect_id: 7,
            colors: palette.clone(),
        };
        for (_, effect_id) in EFFECTS {
            let next = with_effect(&style, effect_id);
            assert_eq!(next.colors.len(), effect_colors(effect_id));
            assert!(palette.starts_with(&next.colors));
        }
        // Too few colors start the effect from its own first swatch instead of repeating one.
        let solid = with_effect(&style, SOLID_EFFECT);
        assert_eq!(with_effect(&solid, 2).colors, GRADIENT_PRESETS[0]);
        assert_eq!(with_effect(&solid, 7).colors, PRISM_PRESETS[0]);
    }
}
