use super::*;
use crate::theme::WithoutTextSelection;

pub(crate) const MEDIUM: f32 = 480.;
pub(crate) const LARGE: f32 = 680.;

pub(crate) struct Dialog {
    pub(crate) id: &'static str,
    pub(crate) width: f32,
    pub(crate) title: &'static str,
    pub(crate) subtitle: Option<&'static str>,
    pub(crate) body: AnyElement,
    pub(crate) footer: AnyElement,
}

pub(crate) fn dialog(
    dialog: Dialog,
    close: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let dismiss = close.clone();
    div()
        .id((dialog.id, 0u64))
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .py(rpx(56.))
        .px(rpx(24.))
        .occlude()
        .bg(rgba(theme::SCRIM))
        .without_text_selection()
        .on_click(move |_, window, cx| dismiss(window, cx))
        .child(
            div()
                .id(dialog.id)
                .relative()
                .flex()
                .flex_col()
                .w(rpx(dialog.width))
                .max_w_full()
                .max_h_full()
                .pt(rpx(8.))
                .pb(rpx(16.))
                .rounded(rpx(12.))
                .border_1()
                .border_color(rgba(theme::border()))
                .bg(rgb(theme::surface_high()))
                .shadow(vec![
                    BoxShadow::new(px(0.), px(12.), hsla(0., 0., 0., 0.24)).blur_radius(px(24.)),
                ])
                .text_color(rgb(theme::text()))
                .occlude()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(8.))
                        .pt(rpx(16.))
                        .px(rpx(24.))
                        .pb(rpx(8.))
                        .child(
                            div()
                                .mr(rpx(36.))
                                .text_size(rpx(20.))
                                .line_height(rpx(24.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::strong()))
                                .child(dialog.title),
                        )
                        .children(dialog.subtitle.map(|subtitle| {
                            div()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .text_color(rgb(theme::muted()))
                                .child(subtitle)
                        })),
                )
                .child(
                    div().absolute().top(rpx(8.)).right(rpx(8.)).child(
                        theme::icon_button((dialog.id, 1u64), "x", 20., theme::muted())
                            .role(Role::Button)
                            .aria_label("Close")
                            .size(rpx(40.))
                            .rounded(rpx(8.))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                close(window, cx);
                            }),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .mt(rpx(16.))
                        .mb(rpx(8.))
                        .px(rpx(24.))
                        .child(dialog.body),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .pt(rpx(16.))
                        .px(rpx(24.))
                        .pb(rpx(8.))
                        .child(dialog.footer),
                )
                .with_animation(
                    (dialog.id, 2u64),
                    Animation::new(Duration::from_millis(150)).with_easing(ease_out_quint()),
                    |this, delta| this.opacity(delta),
                ),
        )
        .into_any_element()
}

pub(crate) fn button(
    id: impl Into<ElementId>,
    label: &'static str,
    primary: bool,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(rpx(40.))
        .px(rpx(16.))
        .rounded(rpx(8.))
        .border_1()
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .font_weight(FontWeight::MEDIUM)
        .without_text_selection()
        .map(|this| {
            if primary {
                this.bg(rgb(theme::BLURPLE))
                    .border_color(rgba(0xFFFFFF14))
                    .text_color(gpui::white())
                    .when(enabled, |this| {
                        this.hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
                    })
            } else {
                this.bg(rgba(theme::control_bg()))
                    .border_color(rgba(theme::border_soft()))
                    .text_color(rgb(theme::strong()))
                    .when(enabled, |this| {
                        this.hover(|style| style.bg(rgba(theme::control_selected())))
                    })
            }
        })
        .when(enabled, |this| this.cursor_pointer())
        .when(!enabled, |this| this.opacity(0.5))
        .child(label)
}

pub(crate) fn text_button(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .cursor_pointer()
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .hover(|style| style.underline())
        .without_text_selection()
        .child(label)
}
