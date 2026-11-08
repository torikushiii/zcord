use gpui::{prelude::*, *};
use std::cell::Cell;

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

thread_local! {
    static CLOSING_PRESS: Cell<Option<Point<Pixels>>> = const { Cell::new(None) };
}

/// For buttons that open a popup. Popups close on any press outside them, their own button's
/// included, so the release would open them again. A click whose press began while `open` held
/// was that close, and `handler` doesn't see it.
pub(in crate::app) trait PopupButton:
    InteractiveElement + StatefulInteractiveElement
{
    fn on_popup_click(
        self,
        open: impl Fn(&App) -> bool + 'static,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.capture_any_mouse_down(move |event, _, cx| {
            CLOSING_PRESS
                .set((event.button == MouseButton::Left && open(cx)).then_some(event.position));
        })
        .on_click(move |event, window, cx| {
            let closing = CLOSING_PRESS.take();
            if let ClickEvent::Mouse(click) = event
                && closing == Some(click.down.position)
            {
                return;
            }
            handler(event, window, cx);
        })
    }
}

impl<E: StatefulInteractiveElement> PopupButton for E {}

pub(in crate::app) fn shown(
    app: &super::DiscordApp,
    open: impl Fn(&super::DiscordApp) -> bool + 'static,
) -> impl Fn(&App) -> bool + 'static {
    let entity = app.entity.clone();
    move |cx| entity.read_with(cx, |app, _| open(app)).unwrap_or(false)
}
