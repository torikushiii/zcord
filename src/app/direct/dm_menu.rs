use std::{cell::Cell, rc::Rc};

use gpui::{prelude::*, *};
use gpui_component::ElementExt as _;
use gpui_component::input::{Input, InputEvent, InputState};

use crate::{
    app::DiscordApp,
    store::RelationshipKind,
    theme::{self, WithoutTextSelection, rpx},
};

const MENU_WIDTH: f32 = 190.;
const MUTE_WIDTH: f32 = 200.;
const GROUP_SIZE: usize = 10;
pub(crate) const MUTE_CHOICES: [(&str, i64); 6] = [
    ("For 15 Minutes", 15 * 60),
    ("For 1 Hour", 60 * 60),
    ("For 3 Hours", 3 * 60 * 60),
    ("For 8 Hours", 8 * 60 * 60),
    ("For 24 Hours", 24 * 60 * 60),
    ("Until I turn it back on", -1),
];

#[derive(Clone, Copy)]
pub(crate) struct DmMenu {
    channel_id: u64,
    position: Point<Pixels>,
    mute_open: bool,
}

pub(crate) enum DmDialog {
    Block(u64),
    LeaveGroup {
        channel_id: u64,
        silent: bool,
    },
    Nickname {
        user_id: u64,
        input: Entity<InputState>,
        _events: Subscription,
    },
    Recipients {
        channel_id: Option<u64>,
        selected: Vec<u64>,
    },
}

impl DiscordApp {
    pub(crate) fn open_dm_menu(
        &mut self,
        channel_id: u64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.menus.dm = Some(DmMenu {
            channel_id,
            position,
            mute_open: false,
        });
        cx.notify();
    }

    pub(crate) fn close_dm_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.menus.dm.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn show_mute_choices(&mut self, open: bool, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.menus.dm
            && menu.mute_open != open
        {
            menu.mute_open = open;
            cx.notify();
        }
    }

    pub(crate) fn open_dm_dialog(&mut self, dialog: DmDialog, cx: &mut Context<Self>) {
        self.menus.dm = None;
        self.menus.dm_dialog = Some(dialog);
        cx.notify();
    }

    pub(crate) fn close_dm_dialog(&mut self, cx: &mut Context<Self>) -> bool {
        if self.menus.dm_dialog.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    pub(crate) fn open_nickname_dialog(
        &mut self,
        user_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .store
            .relationship(user_id)
            .and_then(|relationship| relationship.nickname.clone())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("No nickname");
            state.set_value(current, window, cx);
            state
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let events = cx.subscribe(&input, |app, _, event, cx| {
            if let InputEvent::PressEnter { .. } = event {
                app.save_nickname(cx);
            }
        });
        self.open_dm_dialog(
            DmDialog::Nickname {
                user_id,
                input,
                _events: events,
            },
            cx,
        );
    }

    fn save_nickname(&mut self, cx: &mut Context<Self>) {
        let Some(DmDialog::Nickname { user_id, input, .. }) = &self.menus.dm_dialog else {
            return;
        };
        let user_id = *user_id;
        let value = input.read(cx).value().trim().to_owned();
        self.menus.dm_dialog = None;
        self.set_friend_nickname(user_id, (!value.is_empty()).then_some(value), cx);
    }

    fn toggle_leave_silently(&mut self, cx: &mut Context<Self>) {
        if let Some(DmDialog::LeaveGroup { silent, .. }) = &mut self.menus.dm_dialog {
            *silent = !*silent;
            cx.notify();
        }
    }

    fn toggle_recipient(&mut self, user_id: u64, cx: &mut Context<Self>) {
        let Some(DmDialog::Recipients { channel_id, .. }) = &self.menus.dm_dialog else {
            return;
        };
        let room = self.recipient_room(*channel_id);
        let Some(DmDialog::Recipients { selected, .. }) = &mut self.menus.dm_dialog else {
            return;
        };
        if let Some(index) = selected.iter().position(|id| *id == user_id) {
            selected.remove(index);
        } else if selected.len() < room {
            selected.push(user_id);
        }
        cx.notify();
    }

    fn recipient_room(&self, channel_id: Option<u64>) -> usize {
        let present = channel_id
            .and_then(|id| self.store.private_channel(id))
            .map_or(0, |dm| dm.recipient_ids.len());
        GROUP_SIZE.saturating_sub(present + 1)
    }

    fn recipient_candidates(&self, channel_id: Option<u64>) -> Vec<u64> {
        let present = channel_id
            .and_then(|id| self.store.private_channel(id))
            .map(|dm| dm.recipient_ids.clone())
            .unwrap_or_default();
        self.store
            .friends()
            .into_iter()
            .filter(|id| !present.contains(id))
            .collect()
    }

    fn confirm_dm_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.menus.dm_dialog.take() {
            Some(DmDialog::Block(user_id)) => self.block_user(user_id, cx),
            Some(DmDialog::LeaveGroup { channel_id, silent }) => {
                self.leave_group(channel_id, silent, window, cx)
            }
            nickname @ Some(DmDialog::Nickname { .. }) => {
                self.menus.dm_dialog = nickname;
                self.save_nickname(cx);
            }
            Some(DmDialog::Recipients {
                channel_id,
                selected,
            }) => {
                if selected.is_empty() {
                    self.menus.dm_dialog = Some(DmDialog::Recipients {
                        channel_id,
                        selected,
                    });
                    return;
                }
                let existing = channel_id
                    .and_then(|id| self.store.private_channel(id))
                    .map(|dm| (dm.channel.id, dm.group, dm.recipient_ids.clone()));
                match existing {
                    Some((id, true, _)) => self.add_group_recipients(id, selected, cx),
                    Some((_, false, mut recipients)) => {
                        recipients.extend(selected);
                        self.create_group_dm(recipients, window, cx);
                    }
                    None if selected.len() == 1 => self.open_dm(selected[0], window, cx),
                    None => self.create_group_dm(selected, window, cx),
                }
            }
            None => {}
        }
        cx.notify();
    }

    fn copy_id(&self, id: u64, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()));
    }
}

fn item(app: &DiscordApp, key: &'static str, opens_mute: bool) -> Stateful<Div> {
    let entity = app.entity.clone();
    div()
        .id(key)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .p(rpx(8.))
        .rounded(rpx(4.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .without_text_selection()
        .on_hover(move |hovered, _, cx| {
            if *hovered {
                entity
                    .update(cx, |app, cx| app.show_mute_choices(opens_mute, cx))
                    .ok();
            }
        })
}

fn menu_row(
    app: &DiscordApp,
    key: &'static str,
    icon: Option<&'static str>,
    title: impl Into<SharedString>,
    color: u32,
    run: impl Fn(&mut DiscordApp, &mut Window, &mut Context<DiscordApp>) + 'static,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    item(app, key, false)
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::selected())))
        .text_color(rgb(color))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity
                .update(cx, |app, cx| {
                    app.close_dm_menu(cx);
                    run(app, window, cx);
                })
                .ok();
        })
        .children(icon.map(|icon| theme::icon(icon, 20., color)))
        .child(div().flex_1().min_w_0().truncate().child(title.into()))
}

fn divider() -> Div {
    div()
        .flex_none()
        .h(rpx(1.))
        .my(rpx(8.))
        .bg(rgba(theme::border()))
}

fn mute_item(
    app: &DiscordApp,
    channel_id: u64,
    label: SharedString,
    open: bool,
    choices: Rc<Cell<Option<Bounds<Pixels>>>>,
) -> Stateful<Div> {
    let click = app.entity.clone();
    item(app, "dm-menu-mute", true)
        .relative()
        .cursor_pointer()
        .text_color(rgb(theme::text()))
        .hover(|style| style.bg(rgba(theme::selected())))
        .when(open, |this| this.bg(rgba(theme::selected())))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            click
                .update(cx, |app, cx| app.show_mute_choices(true, cx))
                .ok();
        })
        .child(div().flex_1().min_w_0().truncate().child(label))
        .child(theme::icon("chevron-right", 16., theme::muted()))
        .when(open, |this| {
            let choices_menu = div()
                .id("dm-menu-mute-choices")
                .flex()
                .flex_col()
                .w(rpx(MUTE_WIDTH))
                .p(rpx(8.))
                .rounded(rpx(8.))
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgba(theme::border()))
                .text_color(rgb(theme::text()))
                .shadow_lg()
                .occlude()
                .on_prepaint(move |bounds, _, _| choices.set(Some(bounds)))
                .children(
                    MUTE_CHOICES
                        .iter()
                        .enumerate()
                        .map(|(index, (label, seconds))| {
                            let entity = app.entity.clone();
                            let seconds = *seconds;
                            div()
                                .id(("dm-menu-mute-choice", index))
                                .flex()
                                .flex_none()
                                .items_center()
                                .p(rpx(8.))
                                .rounded(rpx(4.))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(theme::selected())))
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .font_weight(FontWeight::MEDIUM)
                                .without_text_selection()
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    entity
                                        .update(cx, |app, cx| {
                                            app.close_dm_menu(cx);
                                            app.set_dm_mute(channel_id, Some(seconds), cx);
                                        })
                                        .ok();
                                })
                                .child(*label)
                        }),
                );
            this.child(
                div()
                    .absolute()
                    .top(rpx(-8.))
                    .right(rpx(-12.))
                    .size_0()
                    .child(
                        deferred(
                            anchored()
                                .anchor(Anchor::TopLeft)
                                .snap_to_window_with_margin(px(8.))
                                .child(choices_menu),
                        )
                        .with_priority(3),
                    ),
            )
        })
}

pub(crate) fn menu(app: &DiscordApp) -> Option<AnyElement> {
    let menu = app.menus.dm?;
    let channel_id = menu.channel_id;
    let dm = app.store.private_channel(channel_id)?;
    let user_id = dm.recipient_ids.first().copied().filter(|_| !dm.group);
    let user = user_id.and_then(|id| app.store.user(id));
    let relationship = user_id.and_then(|id| app.store.relationship(id));
    let kind = relationship.map(|found| found.kind);
    let friend = kind == Some(RelationshipKind::Friend);
    let blocked = kind == Some(RelationshipKind::Blocked);
    let ignored = relationship.is_some_and(|found| found.ignored);
    let nicknamed = relationship.is_some_and(|found| found.nickname.is_some());
    let bot = user.is_some_and(|user| user.bot);
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let muted = app.store.dm_muted(channel_id, now_ms);
    let in_call = app
        .voice
        .call
        .as_ref()
        .is_some_and(|call| call.guild_id == 0 && call.channel_id == channel_id);
    let unread = app.live && app.store.channel_ack(channel_id).is_some();
    let settled = !dm.message_request;
    let mute_label: SharedString = match user {
        Some(user) => format!("Mute @{}", user.username).into(),
        None => "Mute Conversation".into(),
    };
    let unmute_label: SharedString = match user {
        Some(user) => format!("Unmute @{}", user.username).into(),
        None => "Unmute Conversation".into(),
    };
    let text = theme::text();
    let choices: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
    let outside = choices.clone();
    let close = app.entity.clone();
    let relation = user_id.and_then(|user_id| {
        let (title, run): (
            &'static str,
            fn(&mut DiscordApp, u64, &mut Context<DiscordApp>),
        ) = match kind {
            Some(RelationshipKind::Friend) => {
                ("Remove Friend", |app, id, cx| app.ask_remove_friend(id, cx))
            }
            Some(RelationshipKind::Incoming) => ("Accept Friend Request", |app, id, cx| {
                app.add_friend(id, cx)
            }),
            Some(RelationshipKind::Outgoing) => ("Cancel Friend Request", |app, id, cx| {
                app.drop_friend_request(id, cx)
            }),
            None if !bot && app.live => ("Add Friend", |app, id, cx| app.add_friend(id, cx)),
            _ => return None,
        };
        Some(menu_row(
            app,
            "dm-menu-relationship",
            None,
            title,
            text,
            move |app, _, cx| run(app, user_id, cx),
        ))
    });
    let body = div()
        .id("dm-menu")
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
        .on_mouse_down_out(move |event, _, cx| {
            if outside
                .get()
                .is_some_and(|bounds| bounds.contains(&event.position))
            {
                return;
            }
            close.update(cx, |app, cx| app.close_dm_menu(cx)).ok();
        })
        .child(if unread {
            menu_row(
                app,
                "dm-menu-read",
                None,
                "Mark As Read",
                text,
                move |app, _, cx| app.mark_dm_read(channel_id, cx),
            )
        } else {
            item(app, "dm-menu-read", false)
                .text_color(rgb(theme::faint()))
                .child("Mark As Read")
        })
        .child(divider())
        .children(user_id.map(|user_id| {
            menu_row(
                app,
                "dm-menu-profile",
                None,
                "Profile",
                text,
                move |app, _, cx| app.open_full_profile_for(user_id, cx),
            )
        }))
        .when(settled && app.live, |this| {
            this.child(if in_call {
                menu_row(
                    app,
                    "dm-menu-call",
                    None,
                    "Leave Call",
                    theme::RED,
                    |app, _, cx| app.leave_voice(cx),
                )
            } else {
                menu_row(
                    app,
                    "dm-menu-call",
                    None,
                    "Start a Call",
                    text,
                    move |app, _, cx| app.join_dm_call(channel_id, cx),
                )
            })
        })
        .when(friend, |this| {
            let user_id = user_id.unwrap_or_default();
            this.child(menu_row(
                app,
                "dm-menu-nickname",
                None,
                if nicknamed {
                    "Edit Friend Nickname"
                } else {
                    "Add Friend Nickname"
                },
                text,
                move |app, window, cx| app.open_nickname_dialog(user_id, window, cx),
            ))
        })
        .when(settled, |this| {
            this.child(if dm.group {
                menu_row(
                    app,
                    "dm-menu-leave",
                    None,
                    "Leave Group",
                    theme::RED,
                    move |app, _, cx| {
                        app.open_dm_dialog(
                            DmDialog::LeaveGroup {
                                channel_id,
                                silent: false,
                            },
                            cx,
                        )
                    },
                )
            } else {
                menu_row(
                    app,
                    "dm-menu-close",
                    None,
                    "Close DM",
                    text,
                    move |app, window, cx| app.close_dm(channel_id, window, cx),
                )
            })
        })
        .when_some(user_id, |this, user_id| {
            this.child(divider())
                .children(relation)
                .child(if ignored {
                    menu_row(
                        app,
                        "dm-menu-ignore",
                        None,
                        "Unignore",
                        text,
                        move |app, _, cx| app.unignore_user(user_id, cx),
                    )
                } else {
                    menu_row(
                        app,
                        "dm-menu-ignore",
                        None,
                        "Ignore",
                        text,
                        move |app, _, cx| app.ignore_user(user_id, cx),
                    )
                })
                .child(if blocked {
                    menu_row(
                        app,
                        "dm-menu-block",
                        None,
                        "Unblock",
                        text,
                        move |app, _, cx| app.unblock_user(user_id, cx),
                    )
                } else {
                    menu_row(
                        app,
                        "dm-menu-block",
                        None,
                        "Block",
                        theme::RED,
                        move |app, _, cx| app.open_dm_dialog(DmDialog::Block(user_id), cx),
                    )
                })
        })
        .when(settled, |this| {
            this.child(divider()).child(if muted {
                menu_row(
                    app,
                    "dm-menu-mute",
                    None,
                    unmute_label,
                    text,
                    move |app, _, cx| app.set_dm_mute(channel_id, None, cx),
                )
            } else {
                mute_item(app, channel_id, mute_label, menu.mute_open, choices)
            })
        })
        .child(divider())
        .children(user_id.map(|user_id| {
            menu_row(
                app,
                "dm-menu-copy-user",
                Some("id-badge"),
                "Copy User ID",
                text,
                move |app, _, cx| app.copy_id(user_id, cx),
            )
        }))
        .child(menu_row(
            app,
            "dm-menu-copy-channel",
            Some("id-badge"),
            "Copy Channel ID",
            text,
            move |app, _, cx| app.copy_id(channel_id, cx),
        ));
    Some(
        deferred(
            anchored()
                .position(menu.position)
                .anchor(Anchor::TopLeft)
                .snap_to_window_with_margin(px(8.))
                .child(body),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

fn dialog_button(id: &'static str, text: &'static str, bg: u32, hover: u32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .h(rpx(44.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(rgb(bg))
        .hover(move |style| style.bg(rgb(hover)))
        .text_size(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::strong()))
        .child(text)
}

fn checkbox(checked: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(20.))
        .rounded(rpx(4.))
        .border_1()
        .border_color(rgba(theme::border()))
        .when(checked, |this| {
            this.bg(rgb(theme::BLURPLE))
                .child(theme::icon("check", 14., 0xFFFFFF))
        })
}

pub(crate) fn dialog(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let dialog = app.menus.dm_dialog.as_ref()?;
    let (title, confirm_label, danger): (String, &'static str, bool) = match dialog {
        DmDialog::Block(user_id) => (
            format!("Block '{}'?", app.store.friend_name(*user_id)),
            "Block",
            true,
        ),
        DmDialog::LeaveGroup { channel_id, .. } => (
            format!("Leave '{}'", app.store.private_channel_name(*channel_id)),
            "Leave Group",
            true,
        ),
        DmDialog::Nickname { .. } => ("Edit Friend Nickname".to_owned(), "Save", false),
        DmDialog::Recipients { channel_id, .. } => (
            if channel_id.is_some() {
                "Add Friends".to_owned()
            } else {
                "Select Friends".to_owned()
            },
            if channel_id.is_some() {
                "Add"
            } else {
                "Create DM"
            },
            false,
        ),
    };
    let content: AnyElement = match dialog {
        DmDialog::Block(_) => body_text(
            "They won't be able to message you or see your posts, and you'll stop seeing their messages.",
        ),
        DmDialog::LeaveGroup { silent, .. } => {
            let toggle = app.entity.clone();
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(body_text(
                    "You won't be able to rejoin this group unless you're re-invited.",
                ))
                .child(
                    div()
                        .id("dm-leave-silently")
                        .flex()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .text_size(rpx(14.))
                        .text_color(rgb(theme::text()))
                        .on_click(move |_, _, cx| {
                            toggle
                                .update(cx, |app, cx| app.toggle_leave_silently(cx))
                                .ok();
                        })
                        .child(checkbox(*silent))
                        .child("Leave without notifying other members"),
                )
                .into_any_element()
        }
        DmDialog::Nickname { input, .. } => div()
            .h(rpx(40.))
            .px(rpx(12.))
            .flex()
            .items_center()
            .rounded(rpx(8.))
            .bg(rgba(theme::recessed()))
            .border_1()
            .border_color(rgba(theme::border()))
            .child(Input::new(input).appearance(false).px_0())
            .into_any_element(),
        DmDialog::Recipients {
            channel_id,
            selected,
        } => recipients(app, *channel_id, selected, cx),
    };
    let backdrop = app.entity.clone();
    let cancel = app.entity.clone();
    let confirm = app.entity.clone();
    Some(
        div()
            .id("dm-dialog-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .bg(rgba(theme::SCRIM))
            .on_click(move |_, _, cx| {
                backdrop.update(cx, |app, cx| app.close_dm_dialog(cx)).ok();
            })
            .child(
                div()
                    .id("dm-dialog")
                    .flex()
                    .flex_col()
                    .w(rpx(440.))
                    .max_w_full()
                    .p(rpx(24.))
                    .rounded(rpx(12.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .occlude()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .mb_4()
                            .text_size(rpx(20.))
                            .line_height(rpx(24.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::strong()))
                            .child(title),
                    )
                    .child(content)
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .mt_6()
                            .child(
                                dialog_button(
                                    "dm-dialog-cancel",
                                    "Cancel",
                                    theme::button_subtle(),
                                    theme::button_subtle_hover(),
                                )
                                .border_1()
                                .border_color(rgba(theme::border()))
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |app, cx| app.close_dm_dialog(cx)).ok();
                                }),
                            )
                            .child(
                                if danger {
                                    dialog_button(
                                        "dm-dialog-confirm",
                                        confirm_label,
                                        0xD22D39,
                                        0xB82A33,
                                    )
                                } else {
                                    dialog_button(
                                        "dm-dialog-confirm",
                                        confirm_label,
                                        theme::BLURPLE,
                                        theme::BLURPLE_ACTIVE,
                                    )
                                }
                                .on_click(move |_, window, cx| {
                                    confirm
                                        .update(cx, |app, cx| app.confirm_dm_dialog(window, cx))
                                        .ok();
                                }),
                            ),
                    ),
            )
            .into_any_element(),
    )
}

fn body_text(text: &'static str) -> AnyElement {
    div()
        .text_size(rpx(16.))
        .text_color(rgb(theme::muted()))
        .child(text)
        .into_any_element()
}

fn recipients(app: &DiscordApp, channel_id: Option<u64>, selected: &[u64], cx: &App) -> AnyElement {
    let room = app.recipient_room(channel_id);
    let candidates = app.recipient_candidates(channel_id);
    let left = room.saturating_sub(selected.len());
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_size(rpx(14.))
                .text_color(rgb(theme::muted()))
                .child(if left == 1 {
                    "You can add 1 more friend.".to_owned()
                } else {
                    format!("You can add {left} more friends.")
                }),
        )
        .child(
            div()
                .id("dm-recipient-list")
                .flex()
                .flex_col()
                .max_h(rpx(320.))
                .overflow_y_scroll()
                .when(candidates.is_empty(), |this| {
                    this.child(body_text("No friends to add."))
                })
                .children(candidates.into_iter().map(|user_id| {
                    let checked = selected.contains(&user_id);
                    let entity = app.entity.clone();
                    let user = app.store.user(user_id);
                    div()
                        .id(("dm-recipient", user_id))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(rpx(10.))
                        .h(rpx(44.))
                        .px(rpx(8.))
                        .rounded(rpx(4.))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgba(theme::hover())))
                        .when(!checked && left == 0, |this| this.opacity(0.5))
                        .without_text_selection()
                        .on_click(move |_, _, cx| {
                            entity
                                .update(cx, |app, cx| app.toggle_recipient(user_id, cx))
                                .ok();
                        })
                        .child(theme::user_avatar(user, user_id, 32., None, false, cx))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(rpx(16.))
                                .text_color(rgb(theme::strong()))
                                .child(app.store.friend_name(user_id)),
                        )
                        .child(checkbox(checked))
                })),
        )
        .into_any_element()
}

pub(crate) fn header_actions(app: &DiscordApp, channel_id: u64) -> impl IntoElement + use<> {
    let dm = app.store.private_channel(channel_id);
    let settled = dm.is_some_and(|dm| !dm.message_request);
    let in_call = app
        .voice
        .call
        .as_ref()
        .is_some_and(|call| call.guild_id == 0 && call.channel_id == channel_id);
    let ongoing = app
        .dm_call(channel_id)
        .map_or(0, |call| call.voice_states.len());
    let call = app.entity.clone();
    let add = app.entity.clone();
    div()
        .flex()
        .items_center()
        .gap_2()
        .when(settled && app.live, |this| {
            this.child(
                div()
                    .id("dm-call")
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .cursor_pointer()
                    .child(theme::icon(
                        "phone",
                        20.,
                        if in_call {
                            theme::RED
                        } else if ongoing > 0 {
                            theme::GREEN
                        } else {
                            theme::muted()
                        },
                    ))
                    .when(ongoing > 0 && !in_call, |this| {
                        this.text_size(rpx(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::GREEN))
                            .child(format!("Join ({ongoing})"))
                    })
                    .on_click(move |_, _, cx| {
                        call.update(cx, |app, cx| {
                            if in_call {
                                app.leave_voice(cx);
                            } else {
                                app.join_dm_call(channel_id, cx);
                            }
                        })
                        .ok();
                    }),
            )
            .child(
                theme::icon_button("dm-add-people", "user-plus", 20., theme::muted()).on_click(
                    move |_, _, cx| {
                        add.update(cx, |app, cx| {
                            app.open_dm_dialog(
                                DmDialog::Recipients {
                                    channel_id: Some(channel_id),
                                    selected: Vec::new(),
                                },
                                cx,
                            )
                        })
                        .ok();
                    },
                ),
            )
        })
}

pub(crate) fn header_error(app: &DiscordApp, channel_id: u64) -> Option<impl IntoElement + use<>> {
    let person = app
        .store
        .private_channel(channel_id)
        .filter(|dm| !dm.group)
        .and_then(|dm| dm.recipient_ids.first().copied());
    let text = app
        .dm_error
        .as_ref()
        .filter(|(id, _)| *id == channel_id || Some(*id) == person)
        .map(|(_, text)| text.clone())
        .or_else(|| {
            app.dm_call_error()
                .filter(|(id, _)| *id == channel_id)
                .map(|(_, text)| text.to_owned())
        })?;
    Some(
        div()
            .min_w_0()
            .truncate()
            .text_size(rpx(13.))
            .text_color(rgb(theme::RED))
            .child(text),
    )
}

pub(crate) fn incoming_call(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let channel_id = app.incoming_call()?;
    if app
        .voice
        .call
        .as_ref()
        .is_some_and(|call| call.channel_id == channel_id)
    {
        return None;
    }
    let dm = app.store.private_channel(channel_id)?;
    let name = app.store.private_channel_name(channel_id);
    let avatar = dm
        .recipient_ids
        .first()
        .copied()
        .filter(|_| !dm.group)
        .map(|user_id| {
            theme::user_avatar(app.store.user(user_id), user_id, 40., None, false, cx)
                .into_any_element()
        })
        .unwrap_or_else(|| theme::icon("users", 24., theme::strong()).into_any_element());
    let decline = app.entity.clone();
    let accept = app.entity.clone();
    let round = |id: &'static str, color: u32| {
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(40.))
            .rounded_full()
            .cursor_pointer()
            .bg(rgb(color))
    };
    Some(
        div()
            .absolute()
            .top(rpx(16.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("incoming-call")
                    .flex()
                    .items_center()
                    .gap(rpx(12.))
                    .w(rpx(320.))
                    .p(rpx(12.))
                    .rounded(rpx(12.))
                    .bg(rgb(theme::elevated()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .shadow_lg()
                    .occlude()
                    .child(avatar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rpx(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::strong()))
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_size(rpx(13.))
                                    .text_color(rgb(theme::muted()))
                                    .child("Incoming call…"),
                            ),
                    )
                    .child(
                        round("incoming-call-decline", theme::RED)
                            .child(theme::icon("x", 20., 0xFFFFFF))
                            .on_click(move |_, _, cx| {
                                decline
                                    .update(cx, |app, cx| app.decline_dm_call(channel_id, cx))
                                    .ok();
                            }),
                    )
                    .child(
                        round("incoming-call-accept", theme::GREEN)
                            .child(theme::icon("phone", 20., 0xFFFFFF))
                            .on_click(move |_, window, cx| {
                                accept
                                    .update(cx, |app, cx| {
                                        app.select_dm(channel_id, window, cx);
                                        app.join_dm_call(channel_id, cx);
                                    })
                                    .ok();
                            }),
                    ),
            )
            .into_any_element(),
    )
}
