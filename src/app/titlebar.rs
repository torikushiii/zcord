use gpui::{prelude::*, *};

use crate::{
    app::DiscordApp,
    theme::{self, rpx},
};

pub const HEIGHT: f32 = 32.;
const CAPTION_BUTTON_WIDTH: f32 = 46.;

pub fn render(app: &DiscordApp, window: &Window) -> impl IntoElement + use<> {
    let guild = app.guild();
    div()
        .id("titlebar")
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(HEIGHT))
        .w_full()
        .bg(rgb(theme::BG))
        .window_control_area(WindowControlArea::Drag)
        .when(!cfg!(target_os = "windows"), |this| {
            this.on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .pl_3()
                .child(nav_button("back", "arrow-left"))
                .child(nav_button("forward", "arrow-right")),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .gap_2()
                .text_size(rpx(13.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::STRONG))
                .children(guild.map(|guild| {
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(rpx(16.))
                        .rounded(rpx(5.))
                        .bg(rgb(guild.icon_color))
                        .text_size(rpx(7.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(white())
                        .child(guild.initials.clone())
                }))
                .child(guild.map_or("zcord", |guild| &guild.name).to_owned()),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .pr_2()
                .child(nav_button("inbox", "inbox"))
                .child(nav_button("help", "circle-question-mark")),
        )
        .child(caption_buttons(window))
}

fn nav_button(id: &'static str, icon: &str) -> impl IntoElement {
    theme::icon_button(id, icon, 16., theme::MUTED)
        .size(rpx(24.))
        .occlude()
}

fn caption_buttons(window: &Window) -> impl IntoElement {
    let (max_id, max_icon) = if window.is_maximized() {
        ("window-restore", "icons/window-restore.svg")
    } else {
        ("window-maximize", "icons/window-maximize.svg")
    };
    div()
        .flex()
        .h_full()
        .child(caption_button(
            "window-minimize",
            "icons/window-minimize.svg",
            WindowControlArea::Min,
            false,
            |window| window.minimize_window(),
        ))
        .child(caption_button(
            max_id,
            max_icon,
            WindowControlArea::Max,
            false,
            |window| window.zoom_window(),
        ))
        .child(caption_button(
            "window-close",
            "icons/window-close.svg",
            WindowControlArea::Close,
            true,
            |window| window.remove_window(),
        ))
}

fn caption_button(
    id: &'static str,
    icon_path: &'static str,
    area: WindowControlArea,
    close: bool,
    action: fn(&mut Window),
) -> impl IntoElement {
    let hover_bg: Hsla = if close {
        rgb(0xE81123).into()
    } else {
        rgb(theme::SELECTED).into()
    };
    div()
        .id(id)
        .group(id)
        .flex()
        .items_center()
        .justify_center()
        .w(rpx(CAPTION_BUTTON_WIDTH))
        .h_full()
        .hover(move |style| style.bg(hover_bg))
        .occlude()
        .window_control_area(area)
        .when(!cfg!(target_os = "windows"), |this| {
            this.on_click(move |_, window, _| action(window))
        })
        .child(
            svg()
                .path(icon_path)
                .size(rpx(14.))
                .text_color(rgb(theme::MUTED))
                .group_hover(id, |style| style.text_color(white())),
        )
}
