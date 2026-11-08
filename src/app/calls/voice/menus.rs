use super::*;

const MENU_ENTER: Duration = Duration::from_millis(150);
const MENU_RISE: f32 = 6.;
const SWITCH_SLIDE: Duration = Duration::from_millis(180);

/// Noise suppression popover above the voice panel's waveform button. Rendered from the
/// overlay view because deferred popups can't live inside the cached channel list view.
/// The voice panel's popups. Rendered from the overlay view because deferred popups can't
/// live inside the cached channel list view.
pub(crate) fn menu(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let menu = app.voice.menu.as_ref()?;
    let call = app.voice.call.as_ref();
    let card = match menu.kind {
        MenuKind::NoiseSuppression => noise_card(app, menu, cx),
        MenuKind::Details => details::card(app, call?, app.voice.details_tab, menu.closing, cx),
        MenuKind::Stream => share_panel::live_menu(app, menu),
        MenuKind::InputOptions => device_menu::card(app, menu, device_menu::DeviceKind::Input, cx),
        MenuKind::OutputOptions => {
            device_menu::card(app, menu, device_menu::DeviceKind::Output, cx)
        }
        MenuKind::CallAudio => device_menu::call_card(app, menu, cx),
    };
    let close = app.entity.clone();
    let card = card.when(!menu.closing, |this| {
        this.occlude().on_mouse_down_out(move |_, _, cx| {
            close
                .update(cx, |app, cx| {
                    let in_flyout = app
                        .voice
                        .menu
                        .as_ref()
                        .is_some_and(|menu| menu.pointer_in_flyout);
                    if !in_flyout {
                        app.close_menu(cx);
                    }
                })
                .ok();
        })
    });
    Some(popup(card, menu, cx))
}

fn noise_card(app: &DiscordApp, menu: &Menu, cx: &App) -> Stateful<Div> {
    let closing = menu.closing;
    let reduce_motion = cx.reduce_motion();
    let enabled = app.voice.noise_suppression;
    let toggle = app.entity.clone();
    let slide = (menu.switch_moved && !reduce_motion).then_some(menu.animation);
    let title = div()
        .id("noise-suppression-toggle")
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .cursor_pointer()
        .when(!closing, |this| {
            this.on_click(move |_, _, cx| {
                toggle
                    .update(cx, |app, cx| app.toggle_noise_suppression(cx))
                    .ok();
            })
        })
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Noise Suppression"),
        )
        .child(switch("noise-switch", enabled, slide));
    menu_frame("noise-menu")
        .gap_2()
        .w(rpx(296.))
        .p(rpx(16.))
        .child(title)
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::text()))
                .child(
                    "Filters steady background noise like fans, hum and typing out of your \
                     microphone before it's sent. Sudden sounds, such as clapping, may still \
                     get through.",
                ),
        )
        .child(
            div()
                .pt(rpx(4.))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child("Powered by RNNoise"),
        )
}

pub(super) fn menu_frame(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
}

fn popup(card: Stateful<Div>, menu: &Menu, cx: &App) -> AnyElement {
    let closing = menu.closing;
    let card = if cx.reduce_motion() {
        card.into_any_element()
    } else {
        let key = if closing {
            "voice-menu-out"
        } else {
            "voice-menu-in"
        };
        let animation = Animation::new(if closing { MENU_EXIT } else { MENU_ENTER })
            .with_easing(ease_out_quint());
        card.with_animation((key, menu.animation), animation, move |this, delta| {
            let progress = if closing { 1. - delta } else { delta };
            this.relative()
                .top(rpx((1. - progress) * MENU_RISE))
                .opacity(progress)
        })
        .into_any_element()
    };
    deferred(
        anchored()
            .position(menu.position - point(px(0.), px(16.)))
            .anchor(Anchor::BottomCenter)
            .snap_to_window_with_margin(px(8.))
            .child(card),
    )
    .with_priority(2)
    .into_any_element()
}

/// Discord's settings toggle. `slide` carries the menu's animation id once the user has
/// flipped it, so the knob and track ease into the new state instead of jumping.
pub(crate) fn switch(id: &'static str, on: bool, slide: Option<u64>) -> AnyElement {
    const OFF_LEFT: f32 = 2.;
    const ON_LEFT: f32 = 18.;
    let (icon, icon_color) = if on {
        ("check", theme::BLURPLE)
    } else {
        ("x", theme::faint())
    };
    let at = move |t: f32| if on { t } else { 1. - t };
    let knob = div()
        .absolute()
        .top(rpx(2.))
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(20.))
        .rounded_full()
        .bg(rgb(theme::control_thumb()))
        .when(crate::settings::switch_indicators(), |this| {
            this.child(theme::icon(icon, 12., icon_color))
        });
    let track = div()
        .relative()
        .flex_none()
        .w(rpx(40.))
        .h(rpx(24.))
        .rounded_full();
    let Some(slide) = slide else {
        return track
            .bg(blend(theme::faint(), theme::BLURPLE, at(1.)))
            .child(knob.left(rpx(OFF_LEFT + (ON_LEFT - OFF_LEFT) * at(1.))))
            .into_any_element();
    };
    let animation = Animation::new(SWITCH_SLIDE).with_easing(ease_out_quint());
    let key = slide << 1 | u64::from(on);
    track
        .child(knob.with_animation(
            ElementId::NamedInteger(format!("{id}-knob").into(), key),
            animation.clone(),
            move |this, delta| this.left(rpx(OFF_LEFT + (ON_LEFT - OFF_LEFT) * at(delta))),
        ))
        .with_animation(
            ElementId::NamedInteger(format!("{id}-track").into(), key),
            animation,
            move |this, delta| this.bg(blend(theme::faint(), theme::BLURPLE, at(delta))),
        )
        .into_any_element()
}

fn blend(from: u32, to: u32, t: f32) -> Rgba {
    let (from, to) = (rgb(from), rgb(to));
    Rgba {
        r: from.r + (to.r - from.r) * t,
        g: from.g + (to.g - from.g) * t,
        b: from.b + (to.b - from.b) * t,
        a: 1.,
    }
}
