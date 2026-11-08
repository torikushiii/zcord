use super::*;

const BAR_RADIUS: f32 = 12.;
const MENU_WIDTH: f32 = 208.;
const MENU_ROW: f32 = 40.;
const MENU_GAP: f32 = 8.;
const DETAILS_WIDTH: f32 = 212.;

pub(super) fn panel() -> Div {
    div()
        .bg(rgb(SURFACE))
        .border_1()
        .border_color(rgba(theme::border()))
        .rounded(rpx(BAR_RADIUS))
        .shadow_lg()
}

fn row(id: &'static str, label: &'static str, icon: Option<&str>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(MENU_ROW))
        .px(rpx(8.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .hover(|style| style.bg(rgba(theme::selected())))
        .children(icon.map(|icon| theme::icon(icon, 20., theme::strong())))
        .child(div().flex_1().min_w_0().truncate().child(label))
}

fn menu_row(
    app: &DiscordApp,
    id: &'static str,
    label: &'static str,
    icon: Option<&str>,
    menu: Menu,
) -> Stateful<Div> {
    let hover = app.entity.clone();
    row(id, label, icon).on_hover(move |hovered, _, cx| {
        if *hovered {
            hover
                .update(cx, |app, cx| app.set_lightbox_menu(menu, cx))
                .ok();
        }
    })
}

fn context_row(
    app: &DiscordApp,
    id: &'static str,
    label: &'static str,
    icon: &str,
    run: impl Fn(&mut DiscordApp, &mut Context<DiscordApp>) + 'static,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    row(id, label, Some(icon)).on_click(move |_, _, cx| {
        cx.stop_propagation();
        entity
            .update(cx, |app, cx| {
                app.close_lightbox_context_menu(cx);
                run(app, cx);
            })
            .ok();
    })
}

/// The scrim swallows the click that dismisses the menu, so it doesn't also close the lightbox.
pub(super) fn context_menu(
    app: &DiscordApp,
    lightbox: &Lightbox,
    position: Point<Pixels>,
    attachment: Option<&Attachment>,
) -> AnyElement {
    let dismiss = app.entity.clone();
    let dismiss_right = app.entity.clone();
    let filename = file_name(lightbox, attachment);
    let url = lightbox.url.clone();
    let has_url = !url.is_empty();
    let link = url.clone();
    deferred(
        div()
            .id("lightbox-context-scrim")
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                dismiss
                    .update(cx, |app, cx| app.close_lightbox_context_menu(cx))
                    .ok();
            })
            .on_mouse_down(MouseButton::Right, move |_, _, cx| {
                dismiss_right
                    .update(cx, |app, cx| app.close_lightbox_context_menu(cx))
                    .ok();
            })
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                anchored()
                    .position(position)
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        panel()
                            .id("lightbox-context-menu")
                            .flex()
                            .flex_col()
                            .w(rpx(MENU_WIDTH))
                            .p(rpx(8.))
                            .occlude()
                            .without_text_selection()
                            .child(context_row(
                                app,
                                "lightbox-context-copy-image",
                                "Copy Image",
                                "image",
                                |app, cx| app.copy_lightbox_image(cx),
                            ))
                            .child(context_row(
                                app,
                                "lightbox-context-save-image",
                                "Save Image",
                                "image",
                                move |app, cx| app.save_lightbox_image(filename.clone(), cx),
                            ))
                            .when(has_url, |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .h(rpx(1.))
                                        .my(rpx(4.))
                                        .mx(rpx(8.))
                                        .bg(rgba(theme::border())),
                                )
                                .child(context_row(
                                    app,
                                    "lightbox-context-copy-link",
                                    "Copy Image Link",
                                    "link",
                                    move |_, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            url.clone(),
                                        ))
                                    },
                                ))
                                .child(
                                    context_row(
                                        app,
                                        "lightbox-context-open-link",
                                        "Open Image Link",
                                        "app-window",
                                        move |_, cx| cx.open_url(&link),
                                    )
                                    .child(theme::icon(
                                        "arrow-up-right",
                                        18.,
                                        theme::muted(),
                                    )),
                                )
                            }),
                    ),
            ),
    )
    .with_priority(2)
    .into_any_element()
}

fn details_field(title: &'static str, value: String) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .child(
            div()
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .child(title),
        )
        .child(
            div()
                .truncate()
                .text_size(rpx(13.))
                .text_color(rgb(theme::muted()))
                .child(value),
        )
}

pub(super) fn more_menu(
    app: &DiscordApp,
    lightbox: &Lightbox,
    attachment: Option<&Attachment>,
) -> Div {
    let copy = app.entity.clone();
    let close_link = app.entity.clone();
    let close_id = app.entity.clone();
    let url = lightbox.url.clone();
    let has_url = !url.is_empty();
    let attachment_id = attachment.map(|attachment| attachment.id);
    let size = format!("{}x{}", lightbox.width, lightbox.height);
    let bytes = attachment
        .map(|attachment| attachment.size)
        .or(lightbox.size);
    let size = match bytes {
        Some(bytes) => format!("{size} ({})", chat::file_size(bytes)),
        None => size,
    };
    div()
        .absolute()
        .top(rpx(BAR_HEIGHT + MENU_GAP))
        .right_0()
        .child(
            panel()
                .relative()
                .flex()
                .flex_col()
                .w(rpx(MENU_WIDTH))
                .p(rpx(8.))
                .child(
                    menu_row(
                        app,
                        "lightbox-copy-image",
                        "Copy Image",
                        Some("image"),
                        Menu::Open,
                    )
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        copy.update(cx, |app, cx| {
                            app.copy_lightbox_image(cx);
                            app.set_lightbox_menu(Menu::Closed, cx);
                        })
                        .ok();
                    }),
                )
                .when(has_url, |this| {
                    this.child(
                        menu_row(
                            app,
                            "lightbox-copy-link",
                            "Copy Media Link",
                            Some("link"),
                            Menu::Open,
                        )
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                            close_link
                                .update(cx, |app, cx| app.set_lightbox_menu(Menu::Closed, cx))
                                .ok();
                        }),
                    )
                })
                .child(
                    menu_row(app, "lightbox-details", "View Details", None, Menu::Details)
                        .when(lightbox.menu == Menu::Details, |this| {
                            this.bg(rgba(theme::selected()))
                        })
                        .child(theme::icon("chevron-right", 16., theme::strong())),
                )
                .children(attachment_id.map(|id| {
                    menu_row(
                        app,
                        "lightbox-copy-id",
                        "Copy Attachment ID",
                        Some("id-badge"),
                        Menu::Open,
                    )
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()));
                        close_id
                            .update(cx, |app, cx| app.set_lightbox_menu(Menu::Closed, cx))
                            .ok();
                    })
                }))
                .when(lightbox.menu == Menu::Details, |this| {
                    this.child(
                        panel()
                            .absolute()
                            .top(rpx(MENU_ROW))
                            .right(rpx(MENU_WIDTH + MENU_GAP))
                            .flex()
                            .flex_col()
                            .gap(rpx(16.))
                            .w(rpx(DETAILS_WIDTH))
                            .p(rpx(16.))
                            .child(details_field("Filename", file_name(lightbox, attachment)))
                            .child(details_field("Size", size)),
                    )
                }),
        )
}
