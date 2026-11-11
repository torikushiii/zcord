mod collectibles;
mod color_picker;
mod image;
mod name_style;

use collectibles::collectible_dialog;
use color_picker::color_picker;
use image::{image_dialog, image_edit_dialog};
use name_style::name_style_dialog;

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
        expressions::gif_picker,
        members::member_list::{member_nameplate_layer, rounded_nameplate_layer},
    },
    store::NameStyle as ShownNameStyle,
};

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

fn effect_colors(effect_id: u32) -> usize {
    match effect_id {
        SOLID_EFFECT | 3 | 4 | 5 => 1,
        7 => 5,
        8 => 4,
        _ => 2,
    }
}

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
        .child(theme::selection_guard())
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
        let solid = with_effect(&style, SOLID_EFFECT);
        assert_eq!(with_effect(&solid, 2).colors, GRADIENT_PRESETS[0]);
        assert_eq!(with_effect(&solid, 7).colors, PRISM_PRESETS[0]);
    }
}
