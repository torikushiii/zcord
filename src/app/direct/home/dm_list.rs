use super::*;

const NAV_HEIGHT: f32 = 42.;
const DM_HEIGHT: f32 = 42.;

pub(crate) fn sidebar(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let entity = app.entity.clone();
    let find = div()
        .id("find-conversation")
        .flex()
        .items_center()
        .justify_center()
        .w_full()
        .h(rpx(32.))
        .px_2()
        .rounded(rpx(8.))
        .bg(rgba(theme::hover()))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::muted()))
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(rgba(theme::active()))
                .text_color(rgb(theme::text()))
        })
        .child("Find or start a conversation")
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.toggle_quick_switcher(window, cx))
                .ok();
        });
    let friends_selected = app.showing_home_page() && app.home.page == HomePage::Friends;
    let requests_selected = app.showing_home_page() && app.home.page == HomePage::Requests;
    let quests_selected = app.showing_home_page() && app.home.page == HomePage::Quests;
    let shop_selected = app.showing_home_page() && app.home.page == HomePage::Shop;
    let entity = app.entity.clone();
    let friends = nav_row(
        "home-friends",
        "users",
        "Friends",
        friends_selected,
        app.store.pending_count(),
    )
    .cursor_pointer()
    .when(!friends_selected, |this| {
        this.hover(|style| {
            style
                .bg(rgba(theme::hover()))
                .text_color(rgb(theme::text()))
        })
    })
    .on_click(move |_, window, cx| {
        entity
            .update(cx, |app, cx| app.open_friends(window, cx))
            .ok();
    });
    let selected = app.selected_channel_id;
    let rows: Vec<AnyElement> = app
        .store
        .dm_list()
        .into_iter()
        .map(|dm| dm_row(app, dm, dm.channel.id == selected, cx).into_any_element())
        .collect();
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .w_full()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .h(rpx(chat::HEADER_HEIGHT))
                .px_2()
                .border_b_1()
                .border_color(rgba(theme::border()))
                .child(find),
        )
        .child(
            div()
                .id("dm-list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .px_2()
                .py_2()
                .child(friends)
                .child(nav_row("home-library", "library", "Library", false, 0).opacity(0.5))
                .child(
                    nav_row("home-shop", "store", "Shop", shop_selected, 0)
                        .cursor_pointer()
                        .when(!shop_selected, |this| {
                            this.hover(|style| {
                                style
                                    .bg(rgba(theme::hover()))
                                    .text_color(rgb(theme::text()))
                            })
                        })
                        .on_click({
                            let entity = app.entity.clone();
                            move |_, window, cx| {
                                entity.update(cx, |app, cx| app.open_shop(window, cx)).ok();
                            }
                        }),
                )
                .child(
                    nav_row("home-quests", "laurel-wreath", "Quests", quests_selected, 0)
                        .cursor_pointer()
                        .when(!quests_selected, |this| {
                            this.hover(|style| {
                                style
                                    .bg(rgba(theme::hover()))
                                    .text_color(rgb(theme::text()))
                            })
                        })
                        .on_click({
                            let entity = app.entity.clone();
                            move |_, window, cx| {
                                entity
                                    .update(cx, |app, cx| app.open_quests(window, cx))
                                    .ok();
                            }
                        }),
                )
                .child(
                    nav_row(
                        "home-requests",
                        "mail",
                        "Message Requests",
                        requests_selected,
                        app.store.message_request_count(),
                    )
                    .cursor_pointer()
                    .when(!requests_selected, |this| {
                        this.hover(|style| {
                            style
                                .bg(rgba(theme::hover()))
                                .text_color(rgb(theme::text()))
                        })
                    })
                    .on_click({
                        let entity = app.entity.clone();
                        move |_, window, cx| {
                            entity
                                .update(cx, |app, cx| app.open_requests(window, cx))
                                .ok();
                        }
                    }),
                )
                .child(
                    div()
                        .flex_none()
                        .h(rpx(1.))
                        .mx_2()
                        .my_2()
                        .bg(rgba(theme::border())),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_between()
                        .h(rpx(32.))
                        .px_2()
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::muted()))
                        .child("Direct Messages")
                        .child(
                            theme::icon_button("dm-create", "plus", 16., theme::muted()).on_click(
                                {
                                    let entity = app.entity.clone();
                                    move |_, _, cx| {
                                        entity
                                            .update(cx, |app, cx| {
                                                app.open_dm_dialog(
                                                    dm_menu::DmDialog::Recipients {
                                                        channel_id: None,
                                                        selected: Vec::new(),
                                                    },
                                                    cx,
                                                )
                                            })
                                            .ok();
                                    }
                                },
                            ),
                        ),
                )
                .children(rows),
        )
}

fn nav_row(
    id: &'static str,
    icon: &'static str,
    text: &'static str,
    selected: bool,
    badge: u32,
) -> Stateful<Div> {
    let color = if selected {
        theme::strong()
    } else {
        theme::muted()
    };
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .h(rpx(NAV_HEIGHT))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .when(selected, |this| this.bg(rgba(theme::selected())))
        .child(theme::icon(icon, 20., color))
        .child(div().flex_1().child(text))
        .when(badge > 0, |this| this.child(count_pill(badge)))
}

fn dm_avatar(
    app: &DiscordApp,
    dm: &PrivateChannel,
    ring: u32,
    animate: bool,
    cx: &App,
) -> AnyElement {
    if !dm.group {
        let user_id = dm.recipient_ids.first().copied().unwrap_or(0);
        let user = app.store.user(user_id);
        let status = user.map_or(Status::Offline, |user| user.status);
        return theme::user_avatar(user, user_id, DM_AVATAR, Some((status, ring)), animate, cx)
            .into_any_element();
    }
    let image = dm
        .icon
        .as_ref()
        .map(|hash| ImageSpec::ChannelIcon {
            channel_id: dm.channel.id,
            hash: hash.clone(),
        })
        .and_then(|spec| images::resolve(&spec, theme::image_px(DM_AVATAR), cx));
    match image {
        Some(path) => img(path)
            .size(rpx(DM_AVATAR))
            .flex_none()
            .rounded_full()
            .into_any_element(),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(DM_AVATAR))
            .rounded_full()
            .bg(theme::user_color(dm.channel.id))
            .child(theme::icon("users", 18., theme::strong()))
            .into_any_element(),
    }
}

fn subtitle(app: &DiscordApp, dm: &PrivateChannel) -> Option<(String, Option<&'static str>)> {
    if dm.group {
        let members = dm.recipient_ids.len() + 1;
        return Some((format!("{members} Members"), None));
    }
    let user = app.store.user(*dm.recipient_ids.first()?)?;
    let text = user.activity.clone().filter(|_| user.status.is_online())?;
    Some((text, theme::activity_icon(user)))
}

fn dm_row(
    app: &DiscordApp,
    dm: &PrivateChannel,
    selected: bool,
    cx: &App,
) -> impl IntoElement + use<> {
    let id = dm.channel.id;
    let unread = (dm.channel.unread || dm.channel.mentions > 0) && !selected;
    let name = app.store.private_channel_name(id);
    let subtitle = subtitle(app, dm);
    let text = if selected || unread {
        theme::strong()
    } else {
        theme::muted()
    };
    let ring = if selected {
        theme::composite(theme::selected(), theme::base_lower())
    } else {
        theme::base_lower()
    };
    let recipient = dm.recipient_ids.first().copied().filter(|_| !dm.group);
    let user = recipient.and_then(|user_id| app.store.user(user_id));
    let hovered = recipient.is_some() && app.hover.member == recipient;
    let tag = user
        .and_then(|user| user.tag.as_ref())
        .map(|tag| theme::guild_tag(tag, cx));
    let dm_name_line = div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .text_size(rpx(16.))
        .line_height(rpx(20.))
        .font_weight(if unread {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .child(div().min_w_0().truncate().child(display_name::render(
            ("dm-name", id),
            &name,
            user.and_then(|user| user.name_style.as_ref()),
            Surface::Account { hovered },
            cx,
        )))
        .children(tag);
    let dm_subtitle = subtitle.map(|(text, icon)| {
        div()
            .flex()
            .items_center()
            .gap(rpx(4.))
            .min_w_0()
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .text_color(rgb(theme::muted()))
            .children(icon.map(|icon| theme::icon(icon, 14., theme::GREEN)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(label(("dm-subtitle", id), &text)),
            )
    });
    let nameplate = user.zip(recipient).and_then(|(user, user_id)| {
        member_list::member_nameplate_layer(
            user.nameplate.as_ref()?,
            user_id,
            DM_HEIGHT,
            hovered,
            member_list::ROW_RADIUS,
            cx,
        )
    });
    let plated = nameplate.is_some();
    let blocked = recipient.is_some_and(|user_id| {
        app.store
            .relationship(user_id)
            .is_some_and(|relationship| relationship.kind == RelationshipKind::Blocked)
    });
    let dim = rgba(
        if selected {
            ring
        } else if hovered {
            theme::composite(theme::hover(), theme::base_lower())
        } else {
            theme::base_lower()
        } << 8
            | member_list::NAMEPLATE_DIM,
    );
    let entity = app.entity.clone();
    let row = div()
        .id(("dm", id))
        .relative()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .w_full()
        .h(rpx(DM_HEIGHT))
        .px_2()
        .rounded(rpx(8.))
        .overflow_hidden()
        .cursor_pointer()
        .text_color(rgb(text))
        .when(selected, |this| this.bg(rgba(theme::selected())))
        .when(blocked && !selected, |this| this.opacity(0.35))
        .when(!selected, |this| {
            this.hover(|style| {
                style
                    .bg(rgba(theme::hover()))
                    .text_color(rgb(theme::text()))
                    .opacity(1.)
            })
        })
        .when_some(recipient, |this, user_id| {
            let entity = app.entity.clone();
            this.on_hover(move |hovered, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        if *hovered || app.hover.member == Some(user_id) {
                            app.hover.member = hovered.then_some(user_id);
                            cx.notify();
                        }
                    })
                    .ok();
            })
        })
        .without_text_selection()
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.select_dm(id, window, cx))
                .ok();
        })
        .on_mouse_down(MouseButton::Right, {
            let entity = app.entity.clone();
            move |event, _, cx| {
                let position = event.position;
                entity
                    .update(cx, |app, cx| app.open_dm_menu(id, position, cx))
                    .ok();
            }
        })
        .group("dm-row")
        .children(nameplate)
        .when(plated, |this| {
            this.child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(rpx(8. + DM_AVATAR + 12.))
                    .rounded_l(rpx(member_list::ROW_RADIUS))
                    .bg(dim),
            )
        })
        .child(dm_avatar(app, dm, ring, hovered, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .items_start()
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .max_w_full()
                        .min_w_0()
                        .when(plated, |this| {
                            this.children(member_list::nameplate_mask(dim))
                        })
                        .child(dm_name_line)
                        .children(dm_subtitle),
                ),
        )
        .when(!dm.message_request, |this| {
            let close = app.entity.clone();
            let group = dm.group;
            this.child(
                div()
                    .id(("dm-close", id))
                    .relative()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(blocked, |this| {
                        this.child(
                            div()
                                .absolute()
                                .group_hover("dm-row", |style| style.opacity(0.))
                                .child(theme::icon("ban", 18., theme::muted())),
                        )
                    })
                    .child(
                        div()
                            .opacity(0.)
                            .group_hover("dm-row", |style| style.opacity(1.))
                            .child(theme::icon("x", 16., theme::muted())),
                    )
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        close
                            .update(cx, |app, cx| {
                                if group {
                                    app.open_dm_dialog(
                                        dm_menu::DmDialog::LeaveGroup {
                                            channel_id: id,
                                            silent: false,
                                        },
                                        cx,
                                    );
                                } else {
                                    app.close_dm(id, window, cx);
                                }
                            })
                            .ok();
                    }),
            )
        });
    div()
        .relative()
        .flex_none()
        .mt(rpx(2.))
        .when(unread, |this| {
            this.child(
                div()
                    .absolute()
                    .left(rpx(-8.))
                    .top(rpx(17.))
                    .w(rpx(4.))
                    .h(rpx(8.))
                    .rounded_r(rpx(4.))
                    .bg(rgb(theme::strong())),
            )
        })
        .child(row)
}
