use super::*;

const PICKER_SQUARE: [f32; 2] = [220., 150.];
const HUE_STOPS: [u32; 7] = [
    0xFF0000, 0xFFFF00, 0x00FF00, 0x00FFFF, 0x0000FF, 0xFF00FF, 0xFF0000,
];

pub(super) fn color_picker(
    app: &DiscordApp,
    editor: &StylesEditor,
    picker: ColorPicker,
) -> AnyElement {
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
