use super::*;
use crate::media::images::{self, ImageSpec};

pub(super) const MOD_SUBTLE: u32 = 0x97979F1F;
pub(super) const BORDER_SUBTLE: u32 = 0x97979F29;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tone {
    White,
    Subtle,
    Brand,
}

pub(super) fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tone: Tone,
) -> Stateful<Div> {
    let (bg, hover, text) = match tone {
        Tone::White => (0xFFFFFFFF, 0xE8E8EAFF, 0x000000),
        Tone::Subtle => (MOD_SUBTLE, 0x97979F33, theme::strong()),
        Tone::Brand => (
            (theme::BLURPLE << 8) | 0xFF,
            (theme::BLURPLE_ACTIVE << 8) | 0xFF,
            0xFFFFFF,
        ),
    };
    let label = label.into();
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(rpx(8.))
        .h(rpx(40.))
        .px(rpx(16.))
        .rounded(rpx(8.))
        .bg(rgba(bg))
        .when(tone == Tone::Subtle, |this| {
            this.border_1().border_color(rgba(BORDER_SUBTLE))
        })
        .cursor_pointer()
        .hover(move |style| style.bg(rgba(hover)))
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(text))
        .when(!label.is_empty(), |this| this.child(label))
}

pub(super) fn art(url: &str, size: f32, cx: &App) -> Option<Img> {
    images::resolve(
        &ImageSpec::Collectible {
            url: url.to_owned(),
        },
        theme::image_px(size),
        cx,
    )
    .map(img)
}

pub(super) fn on_app(
    app: &DiscordApp,
    handler: impl Fn(&mut DiscordApp, &mut Window, &mut Context<DiscordApp>) + 'static,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = app.entity.clone();
    move |_, window, cx| {
        cx.stop_propagation();
        entity.update(cx, |app, cx| handler(app, window, cx)).ok();
    }
}

pub(super) struct Choice {
    pub label: &'static str,
    pub selected: bool,
    pub pick: Rc<dyn Fn(&mut DiscordApp, &mut Context<DiscordApp>)>,
}

pub(super) fn select(
    app: &DiscordApp,
    id: &'static str,
    menu: Menu,
    width: f32,
    label: &'static str,
    choices: Vec<Choice>,
) -> AnyElement {
    let open = app.home.shop.menu == Some(menu);
    let popup = open.then(|| {
        let close = app.entity.clone();
        let options = choices.into_iter().enumerate().map(|(index, choice)| {
            let entity = app.entity.clone();
            let pick = choice.pick.clone();
            div()
                .id((id, index))
                .flex()
                .items_center()
                .justify_between()
                .h(rpx(36.))
                .px(rpx(8.))
                .rounded(rpx(4.))
                .cursor_pointer()
                .hover(|style| style.bg(rgba(theme::hover())))
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .on_click(move |_, _, cx| {
                    let pick = pick.clone();
                    entity
                        .update(cx, |app, cx| {
                            app.home.shop.menu = None;
                            pick(app, cx);
                            cx.notify();
                        })
                        .ok();
                })
                .child(choice.label)
                .when(choice.selected, |this| {
                    this.child(theme::icon("check", 16., theme::BLURPLE))
                })
        });
        div().absolute().top(rpx(44.)).left_0().size_0().child(
            deferred(
                anchored().anchor(Anchor::TopLeft).child(
                    div()
                        .id((id, usize::MAX))
                        .flex()
                        .flex_col()
                        .w(rpx(width))
                        .p(rpx(6.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::surface_high()))
                        .border_1()
                        .border_color(rgba(theme::outline()))
                        .shadow_lg()
                        .occlude()
                        .on_mouse_down_out(move |_, _, cx| {
                            close
                                .update(cx, |app, cx| {
                                    app.close_shop_menu(cx);
                                })
                                .ok();
                        })
                        .children(options),
                ),
            )
            .with_priority(2),
        )
    });
    div()
        .relative()
        .flex_none()
        .children(popup)
        .child(
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_between()
                .w(rpx(width))
                .h(rpx(40.))
                .px(rpx(12.))
                .rounded(rpx(8.))
                .bg(rgb(theme::input_bg()))
                .border_1()
                .border_color(rgb(if open {
                    theme::BLURPLE
                } else {
                    theme::input_border()
                }))
                .cursor_pointer()
                .text_size(rpx(16.))
                .text_color(rgb(theme::strong()))
                .on_click(on_app(app, move |app, _, cx| {
                    app.toggle_shop_menu(menu, cx)
                }))
                .child(label)
                .child(theme::icon(
                    if open { "chevron-up" } else { "chevron-down" },
                    18.,
                    theme::strong(),
                )),
        )
        .into_any_element()
}

pub(super) fn pulse(id: impl Into<ElementId>, block: Div, cx: &App) -> AnyElement {
    let fill = |amount: f32| {
        let (from, to) = if theme::is_light() {
            (0x29, 0x1F)
        } else {
            (0x1F, 0x14)
        };
        let alpha = from as f32 + (to as f32 - from as f32) * amount;
        rgba(0x97979F00 | alpha.round() as u32)
    };
    if cx.reduce_motion() {
        return block.bg(fill(0.)).into_any_element();
    }
    block
        .with_animation(
            id,
            Animation::new(Duration::from_millis(1600)).repeat(),
            move |block, delta| {
                let half = if delta < 0.5 {
                    delta * 2.
                } else {
                    (1. - delta) * 2.
                };
                block.bg(fill(ease_in_out(half)))
            },
        )
        .into_any_element()
}
