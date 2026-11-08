use super::menus::{more_menu, panel};
use super::*;

const BAR_INSET: f32 = 8.;
const BAR_ICON: f32 = 20.;
const BAR_BUTTON: f32 = 34.;
const BAR_GAP: f32 = 12.;

fn bar_button(
    app: &DiscordApp,
    id: &'static str,
    tip: &'static str,
    icon: &str,
    active: bool,
) -> Stateful<Div> {
    let hover = app.entity.clone();
    div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(BAR_BUTTON))
        .rounded(rpx(8.))
        .cursor_pointer()
        .when(active, |this| this.bg(rgba(theme::selected())))
        .hover(|style| style.bg(rgba(theme::selected())))
        .on_hover(move |hovered, window, cx| {
            let key = (id, window.window_handle().window_id());
            hover
                .update(cx, |app, cx| {
                    if *hovered {
                        app.stream.hovered_control = Some(key);
                    } else if app.stream.hovered_control == Some(key) {
                        app.stream.hovered_control = None;
                    }
                    app.views.overlays.update(cx, |_, cx| cx.notify());
                })
                .ok();
        })
        .child(theme::icon(icon, BAR_ICON, theme::strong()))
        .when(
            app.stream
                .hovered_control
                .is_some_and(|(hovered, _)| hovered == id),
            |this| this.child(theme::tooltip_below(tip)),
        )
}

pub(super) fn gallery_button(app: &DiscordApp, forward: bool) -> Div {
    let entity = app.entity.clone();
    let icon = theme::icon("chevron-right", 20., theme::strong());
    let icon = if forward {
        icon
    } else {
        icon.with_transformation(Transformation::rotate(radians(std::f32::consts::PI)))
    };
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .flex()
        .items_center()
        .when(forward, |this| this.right(rpx(16.)))
        .when(!forward, |this| this.left(rpx(16.)))
        .child(
            div()
                .id(if forward {
                    "lightbox-next"
                } else {
                    "lightbox-previous"
                })
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(40.))
                .rounded_full()
                .cursor_pointer()
                .bg(rgb(SURFACE))
                .border_1()
                .border_color(rgba(theme::border()))
                .hover(|style| style.bg(rgba(theme::selected())))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    entity
                        .update(cx, |app, cx| app.step_lightbox(forward, cx))
                        .ok();
                })
                .child(icon),
        )
}

pub(super) fn toolbar(
    app: &DiscordApp,
    lightbox: &Lightbox,
    found: Option<Origin<'_>>,
) -> Stateful<Div> {
    let attachment = found.and_then(|origin| origin.attachment);
    let forward_target = found
        .filter(|origin| app.live && origin.message.forwardable)
        .map(|origin| (origin.message.id, origin.channel_id));
    let forward = app.entity.clone();
    let zoom = app.entity.clone();
    let save = app.entity.clone();
    let more = app.entity.clone();
    let close = app.entity.clone();
    let filename = file_name(lightbox, attachment);
    let url = lightbox.url.clone();
    let has_url = !url.is_empty();
    let menu_open = lightbox.menu != Menu::Closed;
    div()
        .id("lightbox-toolbar")
        .absolute()
        .top(rpx(titlebar::HEIGHT + BAR_INSET))
        .right(rpx(BAR_INSET))
        .flex()
        .items_center()
        .gap(rpx(BAR_GAP))
        .occlude()
        .without_text_selection()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            panel()
                .relative()
                .flex()
                .items_center()
                .gap(rpx(2.))
                .h(rpx(BAR_HEIGHT))
                .px(rpx(5.))
                .child(
                    bar_button(
                        app,
                        "lightbox-zoom",
                        if lightbox.zoomed {
                            "Zoom Out"
                        } else {
                            "Zoom In"
                        },
                        if lightbox.zoomed {
                            "zoom-out"
                        } else {
                            "zoom-in"
                        },
                        false,
                    )
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        zoom.update(cx, |app, cx| app.toggle_lightbox_zoom(None, window, cx))
                            .ok();
                    }),
                )
                .children(forward_target.map(|(message_id, channel_id)| {
                    bar_button(app, "lightbox-forward", "Forward", "forward", false).on_click(
                        move |_, window, cx| {
                            cx.stop_propagation();
                            forward
                                .update(cx, |app, cx| {
                                    app.open_forward_dialog(message_id, channel_id, window, cx)
                                })
                                .ok();
                        },
                    )
                }))
                .child(
                    bar_button(app, "lightbox-download", "Download", "download", false).on_click(
                        move |_, _, cx| {
                            cx.stop_propagation();
                            save.update(cx, |app, cx| {
                                app.save_lightbox_image(filename.clone(), cx)
                            })
                            .ok();
                        },
                    ),
                )
                .when(has_url, |this| {
                    this.child(
                        bar_button(
                            app,
                            "lightbox-open-browser",
                            "Open in Browser",
                            "external-link",
                            false,
                        )
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            cx.open_url(&url);
                        }),
                    )
                })
                .child(
                    bar_button(app, "lightbox-more", "More", "ellipsis", menu_open).on_click(
                        move |_, _, cx| {
                            cx.stop_propagation();
                            let menu = if menu_open { Menu::Closed } else { Menu::Open };
                            more.update(cx, |app, cx| app.set_lightbox_menu(menu, cx))
                                .ok();
                        },
                    ),
                )
                .when(menu_open, |this| {
                    this.child(more_menu(app, lightbox, attachment))
                }),
        )
        .child(
            panel()
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(BAR_HEIGHT))
                .child(
                    bar_button(app, "lightbox-close", "Close", "x", false).on_click(
                        move |_, _, cx| {
                            cx.stop_propagation();
                            close.update(cx, |app, cx| app.close_lightbox(cx)).ok();
                        },
                    ),
                ),
        )
}

pub(super) fn author_chip(app: &DiscordApp, message: &Message, cx: &App) -> Div {
    let guild_id = app.selected_guild_id;
    let tag = app
        .store
        .user(message.author_id)
        .and_then(|user| user.tag.as_ref());
    let style = app.store.member_name_style(guild_id, message.author_id);
    let name = app
        .store
        .member_name(guild_id, message.author_id)
        .map(str::to_owned)
        .or_else(|| {
            app.store
                .user(message.author_id)
                .map(|user| user.name.clone())
        })
        .unwrap_or_default();
    div()
        .absolute()
        .top(rpx(titlebar::HEIGHT + BAR_INSET))
        .left(rpx(16.))
        .flex()
        .items_center()
        .gap(rpx(12.))
        .max_w(relative(0.4))
        .without_text_selection()
        .child(theme::undecorated_member_avatar(
            &app.store,
            guild_id,
            message.author_id,
            40.,
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(4.))
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::strong()))
                                .child(display_name::render(
                                    ("lightbox-author", message.id),
                                    &name,
                                    style,
                                    Surface::FontOnly,
                                    cx,
                                )),
                        )
                        .children(tag.map(|tag| theme::guild_tag(tag, cx))),
                )
                .child(
                    div()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::muted()))
                        .child(message.timestamp.short()),
                ),
        )
}
