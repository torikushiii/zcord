use super::*;

const TOOLTIP_ENTER: Duration = Duration::from_millis(120);

pub fn tooltip_bubble() -> Div {
    div()
        .max_w(rpx(196.))
        .px(rpx(11.))
        .py(rpx(7.))
        .rounded(rpx(8.))
        .bg(rgb(tooltip_bg()))
        .border_1()
        .border_color(rgb(tooltip_ring()))
        .shadow(vec![BoxShadow {
            color: rgba(tooltip_shadow()).into(),
            offset: point(px(0.), px(12.)),
            blur_radius: px(24.),
            spread_radius: px(0.),
            inset: false,
        }])
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(strong()))
}

fn tooltip_arrow(fill: &str, edge: &str, width: f32, height: f32) -> Div {
    div()
        .relative()
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .child(icon(fill, width, tooltip_bg()).h(rpx(height)))
        .child(
            icon(edge, width, tooltip_ring())
                .h(rpx(height))
                .absolute()
                .top_0()
                .left_0(),
        )
}

pub fn tooltip_arrow_left() -> Div {
    tooltip_arrow("tooltip-arrow", "tooltip-arrow-edge", 9., 16.).mr(rpx(-1.))
}

fn tooltip_arrow_down() -> Div {
    tooltip_arrow("tooltip-arrow-down", "tooltip-arrow-down-edge", 16., 9.).mt(rpx(-1.))
}

fn tooltip_arrow_up() -> Div {
    tooltip_arrow("tooltip-arrow-up", "tooltip-arrow-up-edge", 16., 9.).mb(rpx(-1.))
}

/// Tooltip shown centred below an element, arrow pointing up, for elements with no room above.
/// Add it as a child of a `relative()` element while that element is hovered.
pub fn tooltip_below(label: impl Into<SharedString>) -> impl IntoElement {
    TooltipBelow {
        label: label.into(),
    }
}

/// Shows a `tooltip_above` while the pointer is over its `relative()` parent, holding its own
/// hover so that the parent's view needs no state for it.
pub fn hover_tooltip(
    key: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> impl IntoElement {
    HoverTooltip {
        key: key.into(),
        label: label.into(),
        detail: None,
        body: None,
        below: false,
    }
}

pub fn hover_tooltip_below(
    key: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> impl IntoElement {
    HoverTooltip {
        key: key.into(),
        label: label.into(),
        detail: None,
        body: None,
        below: true,
    }
}

pub fn hover_tooltip_body(key: impl Into<ElementId>, body: impl IntoElement) -> impl IntoElement {
    HoverTooltip {
        key: key.into(),
        label: SharedString::default(),
        detail: None,
        body: Some(body.into_any_element()),
        below: false,
    }
}

/// A hover tooltip with a second, regular-weight line under a bold label.
pub fn hover_tooltip_detail(
    key: impl Into<ElementId>,
    label: impl Into<SharedString>,
    detail: impl Into<SharedString>,
) -> impl IntoElement {
    HoverTooltip {
        key: key.into(),
        label: label.into(),
        detail: Some(detail.into()),
        body: None,
        below: false,
    }
}

#[derive(IntoElement)]
struct HoverTooltip {
    key: ElementId,
    label: SharedString,
    detail: Option<SharedString>,
    body: Option<AnyElement>,
    below: bool,
}

impl RenderOnce for HoverTooltip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let hovered = window.use_keyed_state(self.key.clone(), cx, |_, _| false);
        let shown = *hovered.read(cx);
        div()
            .id(self.key)
            .absolute()
            .inset_0()
            .on_hover(move |over, _, cx| {
                hovered.update(cx, |hovered, cx| {
                    *hovered = *over;
                    cx.notify();
                });
            })
            .when(shown, |this| {
                if self.below {
                    this.child(TooltipBelow { label: self.label })
                } else {
                    this.child(TooltipAbove {
                        label: self.label,
                        detail: self.detail,
                        body: self.body,
                    })
                }
            })
    }
}

/// Tooltip shown centred above an element, arrow pointing down. Add it as a
/// child of a `relative()` element while that element is hovered.
pub fn tooltip_above(label: impl Into<SharedString>) -> impl IntoElement {
    TooltipAbove {
        label: label.into(),
        detail: None,
        body: None,
    }
}

/// `relative` keeps the rise out of the anchor's layout.
pub fn tooltip_entrance(body: Div, cx: &App) -> AnyElement {
    if cx.reduce_motion() {
        return body.into_any_element();
    }
    let animation = Animation::new(TOOLTIP_ENTER).with_easing(ease_out_quint());
    body.relative()
        .with_animation("tooltip-enter", animation, |this, delta| {
            this.opacity(delta).top(rpx(4. * (1. - delta)))
        })
        .into_any_element()
}

#[derive(IntoElement)]
struct TooltipAbove {
    label: SharedString,
    detail: Option<SharedString>,
    body: Option<AnyElement>,
}

impl RenderOnce for TooltipAbove {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let bubble = div().child(
            tooltip_bubble()
                .flex()
                .flex_col()
                .items_center()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .map(|this| match (self.body, self.detail) {
                    (Some(body), _) => this.items_start().child(body),
                    (None, Some(detail)) => this
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(strong()))
                                .child(self.label),
                        )
                        .child(div().child(detail)),
                    (None, None) => this.child(self.label),
                }),
        );
        // The arrow stays on the target while only the bubble slides to fit the window. A single
        // anchored body would flip its anchor at the window edge and point at a neighbour instead.
        div()
            .absolute()
            .top(rpx(-4.))
            .left(relative(0.5))
            .size_0()
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::BottomCenter)
                        .child(tooltip_entrance(tooltip_arrow_down(), cx)),
                )
                .with_priority(2),
            )
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::BottomCenter)
                        // The arrow is 9px tall and overlaps the bubble's border by 1px.
                        .offset(point(px(0.), -rpx(8.).to_pixels(window.rem_size())))
                        .snap_to_window_with_margin(px(8.))
                        .child(tooltip_entrance(bubble, cx)),
                )
                .with_priority(2),
            )
    }
}

/// `body` centred above its `relative()` parent, over everything else.
fn float_above(body: Div, cx: &App) -> Div {
    div()
        .absolute()
        .top(rpx(-4.))
        .left(relative(0.5))
        .size_0()
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::BottomCenter)
                    .child(tooltip_entrance(body, cx)),
            )
            // Tooltips also appear inside deferred popouts, so they must outrank them.
            .with_priority(2),
        )
}

/// A tooltip bubble with no arrow holding `content`, centred above its `relative()` parent.
pub fn tooltip_card_above(content: impl IntoElement, cx: &App) -> Div {
    float_above(div().child(content), cx)
}

#[derive(IntoElement)]
struct TooltipBelow {
    label: SharedString,
}

impl RenderOnce for TooltipBelow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let body = div()
            .flex()
            .flex_col()
            .items_center()
            .child(tooltip_arrow_up())
            .child(
                tooltip_bubble()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .child(self.label),
            );
        div()
            .absolute()
            .bottom(rpx(-4.))
            .left(relative(0.5))
            .size_0()
            .child(
                deferred(
                    anchored()
                        .anchor(Anchor::TopCenter)
                        .child(tooltip_entrance(body, cx)),
                )
                .with_priority(2),
            )
    }
}
