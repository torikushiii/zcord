use gpui::{prelude::*, *};

use crate::app::menu::PopupButton as _;
use crate::{
    app::{DiscordApp, guild_rail, message_body::label},
    media::images,
    store::Guild,
    theme::{self, rpx},
};

pub const HEIGHT: f32 = 32.;
const CAPTION_BUTTON_WIDTH: f32 = 46.;
const GUILD_ICON: f32 = 16.;

pub fn render(app: &DiscordApp, window: &Window, cx: &App) -> impl IntoElement + use<> {
    let guild = app.guild();
    div()
        .id("titlebar")
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(HEIGHT))
        .w_full()
        .bg(theme::surface(
            theme::Layer::Higher,
            Some(theme::Layer::Higher),
            theme::base_lowest(),
        ))
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
                .text_color(rgb(theme::strong()))
                .children(guild.map(|guild| guild_icon(guild, 5., cx)))
                .child(label(
                    ("titlebar-guild", guild.map_or(0, |guild| guild.id)),
                    guild.map_or("zcord", |guild| &guild.name),
                )),
        )
        .child(div().flex_1())
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .pr_2()
                .children(super::saved::overdue_pill(app))
                .child(inbox_button(app))
                .child(nav_button("help", "circle-question-mark")),
        )
        .child(caption_buttons(window, CAPTION_BUTTON_WIDTH))
}

pub(in crate::app) fn guild_icon(guild: &Guild, radius: f32, cx: &App) -> AnyElement {
    let image = guild
        .icon_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(guild_rail::ICON), cx));
    match image {
        Some(path) => img(path)
            .size(rpx(GUILD_ICON))
            .flex_none()
            .rounded(rpx(radius))
            .into_any_element(),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(GUILD_ICON))
            .rounded(rpx(radius))
            .bg(rgb(guild.icon_color))
            .text_size(rpx(7.))
            .font_weight(FontWeight::BOLD)
            .text_color(white())
            .child(guild.initials.clone())
            .into_any_element(),
    }
}

fn inbox_button(app: &DiscordApp) -> impl IntoElement + use<> {
    let entity = app.entity.clone();
    theme::icon_button("inbox", "inbox", 16., theme::muted())
        .relative()
        .size(rpx(24.))
        .occlude()
        .when(app.inbox.is_some(), |this| this.bg(rgba(theme::selected())))
        // The open inbox sits right under the button, where the tooltip would cover its header.
        .when(app.inbox.is_none(), |this| {
            this.child(theme::hover_tooltip_below("inbox-tooltip", "Inbox"))
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_popup_click(
            super::menu::shown(app, |app| app.inbox.is_some()),
            move |_, _, cx| {
                entity.update(cx, |app, cx| app.toggle_inbox(None, cx)).ok();
            },
        )
}

fn nav_button(id: &'static str, icon: &str) -> impl IntoElement {
    theme::icon_button(id, icon, 16., theme::muted())
        .size(rpx(24.))
        .occlude()
}

pub(super) fn caption_buttons(window: &Window, width: f32) -> impl IntoElement {
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
            width,
            |window| window.minimize_window(),
        ))
        .child(caption_button(
            max_id,
            max_icon,
            WindowControlArea::Max,
            false,
            width,
            |window| window.zoom_window(),
        ))
        .child(caption_button(
            "window-close",
            "icons/window-close.svg",
            WindowControlArea::Close,
            true,
            width,
            |window| window.remove_window(),
        ))
}

fn caption_button(
    id: &'static str,
    icon_path: &'static str,
    area: WindowControlArea,
    close: bool,
    width: f32,
    action: fn(&mut Window),
) -> impl IntoElement {
    let hover_bg: Hsla = if close {
        rgb(0xE81123).into()
    } else {
        rgba(theme::selected()).into()
    };
    div()
        .id(id)
        .group(id)
        .flex()
        .items_center()
        .justify_center()
        .w(rpx(width))
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
                .text_color(rgb(theme::muted()))
                .group_hover(id, move |style| {
                    style.text_color(if close {
                        white()
                    } else {
                        rgb(theme::bright()).into()
                    })
                }),
        )
}
