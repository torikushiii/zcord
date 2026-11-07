use super::*;

const PEEK: f32 = 8.;
const LIFT: f32 = 2.;
const MOTION: Duration = Duration::from_millis(200);

pub(in crate::app::profile) fn hover_card(
    app: &DiscordApp,
    id: impl Into<ElementId>,
    tab: Tab,
    palette: ProfilePalette,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    div()
        .id(id)
        .cursor_pointer()
        .hover(move |style| style.bg(palette.panel.opacity(1.25)))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            entity
                .update(cx, |app, cx| app.open_full_profile_on(tab, cx))
                .ok();
        })
}

pub(in crate::app::profile) fn card_stack(
    app: &DiscordApp,
    card: &ProfileCard,
    stack: Stack,
    tab: Tab,
    front: AnyElement,
    cx: &App,
) -> AnyElement {
    let palette = card.palette();
    let StackHover {
        hovered,
        generation,
    } = card.stack_hover[stack as usize];
    let panel = card.panel;
    let key = card.user_id ^ stack as u64;
    // Never hovered yet: nothing to animate, so a fresh card doesn't play a settle on open.
    let animate = generation > 0 && !cx.reduce_motion();
    let peek = hover_card(app, ("profile-stack-peek", key), tab, palette)
        .absolute()
        .left(rpx(8.))
        .right(rpx(8.))
        .rounded_b(rpx(8.))
        .bg(palette.panel)
        .border_1()
        .border_t_0()
        .border_color(palette.border);
    let entity = app.entity.clone();
    div()
        .id(("profile-stack", key))
        .relative()
        .pb(rpx(PEEK))
        .on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, cx| {
                    app.hover_profile_stack(panel, stack, *hovered, cx)
                })
                .ok();
        })
        .child(motion(
            peek,
            ("profile-stack-peek-motion", key ^ generation),
            hovered,
            animate,
            |this, lift| this.bottom(rpx(-lift)).h(rpx(PEEK + lift * 2.)),
        ))
        .child(motion(
            div().relative().child(front),
            ("profile-stack-front-motion", key ^ generation),
            hovered,
            animate,
            |this, lift| this.top(rpx(-lift)),
        ))
        .into_any_element()
}

fn motion<E: IntoElement + 'static>(
    element: E,
    id: (&'static str, u64),
    hovered: bool,
    animate: bool,
    style: impl Fn(E, f32) -> E + 'static,
) -> AnyElement {
    let lift = move |delta: f32| LIFT * if hovered { delta } else { 1. - delta };
    if !animate {
        return style(element, lift(1.)).into_any_element();
    }
    element
        .with_animation(
            id,
            Animation::new(MOTION).with_easing(ease_in_out),
            move |this, delta| style(this, lift(delta)),
        )
        .into_any_element()
}
