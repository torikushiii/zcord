//! Discord's popup menu pieces, shared by the stream settings, the live stream menu and the
//! composer's right-click menu.

use gpui::{prelude::*, *};

use crate::theme::{self, WithoutTextSelection, rpx};

pub(in crate::app) fn row(key: impl Into<ElementId>, enabled: bool) -> Stateful<Div> {
    div()
        .id(key)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .min_h(rpx(32.))
        .p(rpx(8.))
        .rounded(rpx(4.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::text()))
        .without_text_selection()
        .map(|this| {
            if enabled {
                this.cursor_pointer()
                    .hover(|style| style.bg(rgba(theme::selected())))
            } else {
                this.opacity(0.5)
            }
        })
}

/// Drawn from SVGs rather than nested boxes: at fractional display scales a box's dot rounds to
/// whole pixels apart from its ring and sits off-centre.
pub(in crate::app) fn radio(selected: bool) -> Div {
    let size = 20.;
    let layer = |name, color| {
        div()
            .absolute()
            .inset_0()
            .child(theme::icon(name, size, color))
    };
    div().relative().flex_none().size(rpx(size)).map(|this| {
        if selected {
            this.child(layer("radio-fill", theme::BLURPLE))
                .child(layer("radio-dot", 0xFFFFFF))
        } else {
            this.child(layer("radio-ring", theme::muted()))
        }
    })
}

pub(in crate::app) fn checkbox(checked: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(20.))
        .rounded(rpx(4.))
        .map(|this| {
            if checked {
                this.bg(rgb(theme::BLURPLE))
                    .child(theme::icon("check", 14., 0xFFFFFF))
            } else {
                this.border_1().border_color(rgb(theme::muted()))
            }
        })
}

pub(in crate::app) fn divider() -> Div {
    div()
        .flex_none()
        .h(rpx(1.))
        .m(rpx(8.))
        .bg(rgba(theme::border()))
}

pub(in crate::app) fn panel(id: &'static str, width: f32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .w(rpx(width))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
        .occlude()
}
