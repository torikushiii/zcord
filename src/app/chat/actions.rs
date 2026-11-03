use super::rows::is_system;
use super::*;
use crate::app::emoji_picker::{PickerTarget, choice_name, emoji_image};
use crate::store::EmojiChoice;

const BAR_REACTIONS: usize = 3;
const MENU_REACTIONS: usize = 4;
const MENU_WIDTH: f32 = 220.;
const SUBMENU_HEIGHT: f32 = 360.;

#[derive(Clone, Copy)]
pub(in crate::app) struct MessageMenu {
    message_id: u64,
    position: Point<Pixels>,
    anchor: Anchor,
    reactions_open: bool,
}

impl DiscordApp {
    fn toggle_message_menu(
        &mut self,
        message_id: u64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let open = self
            .message_menu
            .is_some_and(|menu| menu.message_id == message_id);
        self.message_menu = (!open).then_some(MessageMenu {
            message_id,
            position,
            anchor: Anchor::TopRight,
            reactions_open: false,
        });
        cx.notify();
    }

    pub(super) fn open_message_menu_at(
        &mut self,
        message_id: u64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.message_menu = Some(MessageMenu {
            message_id,
            position,
            anchor: Anchor::TopLeft,
            reactions_open: false,
        });
        cx.notify();
    }

    fn show_reaction_submenu(&mut self, open: bool, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.message_menu
            && menu.reactions_open != open
        {
            menu.reactions_open = open;
            cx.notify();
        }
    }

    pub(in crate::app) fn close_message_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.message_menu.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn copy_message_id(&self, message_id: u64, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(message_id.to_string()));
    }

    fn copy_message_text(&self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn copy_message_link(&self, message_id: u64, cx: &mut Context<Self>) {
        let guild = match self.selected_guild_id {
            0 => "@me".to_owned(),
            id => id.to_string(),
        };
        let channel = self.selected_channel_id;
        let link = format!("https://discord.com/channels/{guild}/{channel}/{message_id}");
        cx.write_to_clipboard(ClipboardItem::new_string(link));
    }
}

struct Permissions {
    forward: bool,
    thread: bool,
    edit: bool,
    reply: bool,
    delete: bool,
    unread: bool,
}

fn permissions(app: &DiscordApp, message: &Message) -> Permissions {
    let channel_id = app.selected_channel_id;
    let own = message.author_id == app.store.current_user_id;
    Permissions {
        forward: app.live && message.forwardable,
        thread: app.thread_parent().is_none()
            && message.thread.is_none()
            && app.store.can_create_thread(channel_id, false),
        edit: own && matches!(message.kind, 0 | 19),
        reply: app.store.can_send(channel_id),
        delete: own || app.store.can_manage_messages(channel_id),
        unread: app.live
            && app
                .store
                .last_read(channel_id)
                .is_some_and(|read| message.id <= read),
    }
}

fn quick_reaction(
    app: &DiscordApp,
    id: impl Into<ElementId>,
    message_id: u64,
    choice: EmojiChoice,
    size: f32,
    cx: &App,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    let image = emoji_image(&choice, size, false, cx);
    let id = id.into();
    let label = format!(":{}:", choice_name(&choice));
    div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(rpx(8.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::hover())))
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| {
                    app.close_message_menu(cx);
                    app.react(message_id, &choice, cx);
                })
                .ok();
        })
        .child(image)
        .child(theme::hover_tooltip_detail(
            ElementId::NamedChild(std::sync::Arc::new(id), "tip".into()),
            label,
            "Click to react",
        ))
}

/// Discord's "Add Reaction" submenu: recent emoji by name, then the full picker.
fn reaction_submenu(app: &DiscordApp, message_id: u64, cx: &App) -> Div {
    let rows = app
        .recent_emojis(cx)
        .into_iter()
        .enumerate()
        .map(|(index, choice)| {
            let entity = app.entity.clone();
            let image = emoji_image(&choice, 20., false, cx);
            let label = format!(":{}:", choice_name(&choice));
            div()
                .id(("submenu-reaction", index))
                .flex()
                .flex_none()
                .items_center()
                .gap(rpx(10.))
                .h(rpx(36.))
                .px(rpx(8.))
                .rounded(rpx(4.))
                .cursor_pointer()
                .hover(|style| style.bg(rgba(theme::selected())))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| {
                            app.close_message_menu(cx);
                            app.react(message_id, &choice, cx);
                        })
                        .ok();
                })
                .child(image)
                .child(div().flex_1().min_w_0().truncate().child(label))
        });
    let entity = app.entity.clone();
    let more = div()
        .id("submenu-reaction-more")
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(36.))
        .px(rpx(8.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::selected())))
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            let position = event.position;
            entity
                .update(cx, |app, cx| {
                    app.close_message_menu(cx);
                    app.toggle_emoji_picker(
                        PickerTarget::Reaction(message_id),
                        position,
                        window,
                        cx,
                    );
                })
                .ok();
        })
        .child("View More");
    div()
        .absolute()
        .top(rpx(-8.))
        .left(relative(1.))
        .pl(rpx(16.))
        .child(
            div()
                .id("reaction-submenu")
                .flex()
                .flex_col()
                .w(rpx(MENU_WIDTH))
                .max_h(rpx(SUBMENU_HEIGHT))
                .overflow_y_scroll()
                .p(rpx(8.))
                .rounded(rpx(8.))
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgba(theme::border()))
                .shadow_lg()
                .occlude()
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::text()))
                .children(rows)
                .child(more),
        )
}

pub(super) fn has_actions(message: &Message) -> bool {
    message.delivery == Delivery::Sent && (!is_system(message.kind) || matches!(message.kind, 19))
}

pub(super) fn message_actions(app: &DiscordApp, message: &Message, cx: &App) -> Option<AnyElement> {
    let id = message.id;
    let menu_open = app.message_menu.is_some_and(|menu| menu.message_id == id);
    if !(app.hovered_message == Some(id) || app.hovered_actions == Some(id) || menu_open)
        || !has_actions(message)
    {
        return None;
    }
    let allowed = permissions(app, message);
    let extended = app.shift_held;
    let button = |key: &'static str, icon: &str, color: u32, tip: &'static str| {
        theme::icon_button((key, id), icon, 20., color)
            .relative()
            .child(theme::hover_tooltip((tip, id), tip))
    };
    let hover = app.entity.clone();
    let edit = app.entity.clone();
    let reply = app.entity.clone();
    let delete = app.entity.clone();
    let unread = app.entity.clone();
    let react = app.entity.clone();
    let more = app.entity.clone();
    let copy_id = app.entity.clone();
    let copy_link = app.entity.clone();
    let thread = app.entity.clone();
    let forward = app.entity.clone();
    let channel_id = app.selected_channel_id;
    let reactions: Vec<_> = (app.live && !extended)
        .then(|| app.recent_emojis(cx))
        .into_iter()
        .flatten()
        .take(BAR_REACTIONS)
        .enumerate()
        .map(|(index, choice)| {
            quick_reaction(
                app,
                ("bar-reaction", id + index as u64),
                id,
                choice,
                22.,
                cx,
            )
            .size(rpx(32.))
        })
        .collect();
    let divider = !reactions.is_empty();
    let edit_button = allowed.edit.then(|| {
        button("edit", "pencil", theme::muted(), "Edit").on_click(move |_, window, cx| {
            edit.update(cx, |app, cx| app.start_edit(id, window, cx))
                .ok();
        })
    });
    let reply_button = allowed.reply.then(|| {
        button("reply", "reply", theme::muted(), "Reply").on_click(move |_, window, cx| {
            reply
                .update(cx, |app, cx| app.start_reply(id, window, cx))
                .ok();
        })
    });
    let react_button = app.live.then(|| {
        button("react", "smile", theme::muted(), "Add Reaction").on_mouse_down(
            MouseButton::Left,
            move |event, window, cx| {
                let position = event.position - point(px(0.), px(8.));
                react
                    .update(cx, |app, cx| {
                        app.toggle_emoji_picker(PickerTarget::Reaction(id), position, window, cx)
                    })
                    .ok();
            },
        )
    });
    let forward_button = allowed.forward.then(|| {
        button("forward", "forward", theme::muted(), "Forward").on_click(move |_, window, cx| {
            forward
                .update(cx, |app, cx| {
                    app.open_forward_dialog(id, channel_id, window, cx)
                })
                .ok();
        })
    });
    let mut buttons = Vec::new();
    if extended {
        buttons.push(
            button("copy-id", "id-badge", theme::muted(), "Copy Message ID").on_click(
                move |_, _, cx| {
                    copy_id
                        .update(cx, |app, cx| app.copy_message_id(id, cx))
                        .ok();
                },
            ),
        );
        buttons.push(
            button("copy-link", "link", theme::muted(), "Copy Message Link").on_click(
                move |_, _, cx| {
                    copy_link
                        .update(cx, |app, cx| app.copy_message_link(id, cx))
                        .ok();
                },
            ),
        );
        if allowed.unread {
            buttons.push(
                button("unread", "mail", theme::muted(), "Mark Unread").on_click(
                    move |_, _, cx| {
                        unread.update(cx, |app, cx| app.mark_unread(id, cx)).ok();
                    },
                ),
            );
        }
        if allowed.thread {
            buttons.push(
                button("thread", "messages-square", theme::muted(), "Create Thread").on_click(
                    move |_, window, cx| {
                        thread
                            .update(cx, |app, cx| app.open_create_thread(Some(id), window, cx))
                            .ok();
                    },
                ),
            );
        }
        buttons.extend(reply_button);
        buttons.extend(react_button);
        buttons.extend(edit_button);
        buttons.extend(forward_button);
        if allowed.delete {
            buttons.push(button("delete", "trash-2", theme::RED, "Delete").on_click(
                move |event, window, cx| {
                    let confirmed = event.modifiers().shift;
                    delete
                        .update(cx, |app, cx| app.request_delete(id, confirmed, window, cx))
                        .ok();
                },
            ));
        }
    } else {
        buttons.extend(react_button);
        buttons.extend(edit_button.or(reply_button));
        buttons.extend(forward_button);
    }
    Some(
        div()
            .id(("message-actions", id))
            .absolute()
            .top(rpx(-16.))
            .right(rpx(14.))
            .flex()
            .items_center()
            .p(rpx(2.))
            .rounded(rpx(8.))
            .bg(rgb(theme::elevated()))
            .border_1()
            .border_color(rgba(theme::border()))
            .block_mouse_except_scroll()
            .on_hover(move |hovered, _, cx| {
                hover
                    .update(cx, |app, cx| app.set_message_hover(id, *hovered, true, cx))
                    .ok();
            })
            .children(reactions)
            .when(divider, |this| {
                this.child(
                    div()
                        .flex_none()
                        .w(rpx(1.))
                        .h(rpx(20.))
                        .mx(rpx(4.))
                        .bg(rgba(theme::border())),
                )
            })
            .children(buttons)
            .when(!extended || !allowed.delete, |this| {
                this.child(button("more", "ellipsis", theme::muted(), "More").on_click(
                    move |event, _, cx| {
                        let position = event.position();
                        more.update(cx, |app, cx| app.toggle_message_menu(id, position, cx))
                            .ok();
                    },
                ))
            })
            .into_any_element(),
    )
}

pub(super) fn menu_entry(
    app: &DiscordApp,
    key: &'static str,
    icon: &str,
    title: &'static str,
    color: u32,
    run: impl Fn(&mut DiscordApp, Point<Pixels>, &mut Window, &mut Context<DiscordApp>) + 'static,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    menu_row(app, key, icon, title, color, run).on_hover(move |hovered, _, cx| {
        if *hovered {
            entity
                .update(cx, |app, cx| app.show_reaction_submenu(false, cx))
                .ok();
        }
    })
}

fn menu_row(
    app: &DiscordApp,
    key: &'static str,
    icon: &str,
    title: &'static str,
    color: u32,
    run: impl Fn(&mut DiscordApp, Point<Pixels>, &mut Window, &mut Context<DiscordApp>) + 'static,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    div()
        .id(key)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(10.))
        .h(rpx(36.))
        .px(rpx(8.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::selected())))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .without_text_selection()
        .on_click(move |event, window, cx| {
            let position = event.position();
            entity
                .update(cx, |app, cx| {
                    app.close_message_menu(cx);
                    run(app, position, window, cx);
                })
                .ok();
        })
        .child(theme::icon(icon, 20., color))
        .child(div().flex_1().min_w_0().truncate().child(title))
}

pub(super) fn menu_divider() -> Div {
    div()
        .flex_none()
        .h(rpx(1.))
        .my(rpx(4.))
        .mx(rpx(6.))
        .bg(rgba(theme::border()))
}

pub(in crate::app) fn message_menu(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let menu = app.message_menu?;
    let id = menu.message_id;
    let message = app
        .channel()?
        .messages
        .iter()
        .find(|message| message.id == id)?;
    let allowed = permissions(app, message);
    let copy_text = message.content.clone();
    let close = app.entity.clone();
    let reactions: Vec<_> = app
        .live
        .then(|| app.recent_emojis(cx))
        .into_iter()
        .flatten()
        .take(MENU_REACTIONS)
        .enumerate()
        .map(|(index, choice)| {
            quick_reaction(app, ("menu-reaction", index), id, choice, 24., cx)
                .flex_1()
                .h(rpx(40.))
                .bg(rgba(theme::hover()))
        })
        .collect();
    let text = theme::text();
    let body = div()
        .id("message-menu")
        .flex()
        .flex_col()
        .w(rpx(MENU_WIDTH))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(move |_, _, cx| {
            close.update(cx, |app, cx| app.close_message_menu(cx)).ok();
        })
        .when(!reactions.is_empty(), |this| {
            this.child(
                div()
                    .flex()
                    .flex_none()
                    .gap(rpx(4.))
                    .mb(rpx(4.))
                    .children(reactions),
            )
        })
        .when(app.live, |this| {
            let hover = app.entity.clone();
            this.child(
                menu_row(
                    app,
                    "menu-react",
                    "smile",
                    "Add Reaction",
                    text,
                    move |app, position, window, cx| {
                        app.toggle_emoji_picker(PickerTarget::Reaction(id), position, window, cx);
                    },
                )
                .relative()
                .when(menu.reactions_open, |this| this.bg(rgba(theme::selected())))
                .on_hover(move |hovered, _, cx| {
                    if *hovered {
                        hover
                            .update(cx, |app, cx| app.show_reaction_submenu(true, cx))
                            .ok();
                    }
                })
                .child(theme::icon("chevron-right", 16., theme::muted()))
                .when(menu.reactions_open, |this| {
                    this.child(reaction_submenu(app, id, cx))
                }),
            )
            .child(menu_divider())
        })
        .when(allowed.edit, |this| {
            this.child(menu_entry(
                app,
                "menu-edit",
                "pencil",
                "Edit Message",
                text,
                move |app, _, window, cx| app.start_edit(id, window, cx),
            ))
        })
        .when(allowed.reply, |this| {
            this.child(menu_entry(
                app,
                "menu-reply",
                "reply",
                "Reply",
                text,
                move |app, _, window, cx| app.start_reply(id, window, cx),
            ))
        })
        .when(allowed.forward, |this| {
            let channel_id = app
                .thread_pane_channel()
                .filter(|thread| thread.message_index(id).is_some())
                .map_or(app.selected_channel_id, |thread| thread.id);
            this.child(menu_entry(
                app,
                "menu-forward",
                "forward",
                "Forward",
                text,
                move |app, _, window, cx| app.open_forward_dialog(id, channel_id, window, cx),
            ))
        })
        .when(allowed.thread, |this| {
            this.child(menu_entry(
                app,
                "menu-thread",
                "messages-square",
                "Create Thread",
                text,
                move |app, _, window, cx| app.open_create_thread(Some(id), window, cx),
            ))
        })
        .when(
            allowed.edit || allowed.reply || allowed.forward || allowed.thread,
            |this| this.child(menu_divider()),
        )
        .when(!copy_text.is_empty(), |this| {
            this.child(menu_entry(
                app,
                "menu-copy-text",
                "copy",
                "Copy Text",
                text,
                move |app, _, _, cx| app.copy_message_text(copy_text.clone(), cx),
            ))
        })
        .when(
            app.store.can_manage_messages(app.selected_channel_id),
            |this| {
                this.child(menu_entry(
                    app,
                    "menu-pin",
                    "pin",
                    "Pin Message",
                    text,
                    move |app, _, _, cx| app.pin_message(id, cx),
                ))
            },
        )
        .when(allowed.unread, |this| {
            this.child(menu_entry(
                app,
                "menu-unread",
                "mail",
                "Mark Unread",
                text,
                move |app, _, _, cx| app.mark_unread(id, cx),
            ))
        })
        .child(menu_entry(
            app,
            "menu-copy-link",
            "link",
            "Copy Message Link",
            text,
            move |app, _, _, cx| app.copy_message_link(id, cx),
        ))
        .when(allowed.delete, |this| {
            this.child(menu_divider()).child(menu_entry(
                app,
                "menu-delete",
                "trash-2",
                "Delete Message",
                theme::RED,
                move |app, _, window, cx| {
                    let confirmed = window.modifiers().shift;
                    app.request_delete(id, confirmed, window, cx);
                },
            ))
        })
        .child(menu_divider())
        .child(menu_entry(
            app,
            "menu-copy-id",
            "id-badge",
            "Copy Message ID",
            text,
            move |app, _, _, cx| app.copy_message_id(id, cx),
        ));
    Some(
        deferred(
            anchored()
                .position(menu.position)
                .anchor(menu.anchor)
                .snap_to_window_with_margin(px(8.))
                .child(body),
        )
        .with_priority(2)
        .into_any_element(),
    )
}
