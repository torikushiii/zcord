use gpui_component::ElementExt;

use super::super::*;
use crate::app::friends::FriendAction;

type MenuBounds = Rc<Cell<Option<Bounds<Pixels>>>>;

fn menu_entry(
    id: &'static str,
    title: &'static str,
    subtitle: Option<String>,
    activate: impl Fn(&mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::MenuItem)
        .focusable()
        .flex()
        .flex_col()
        .justify_center()
        .gap(rpx(2.))
        .min_h(rpx(36.))
        .px(rpx(12.))
        .py(rpx(8.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .text_size(rpx(14.))
        .line_height(rpx(20.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .hover(|style| style.bg(rgba(theme::selected())))
        .focus_visible(|style| style.bg(rgba(theme::selected())))
        .without_text_selection()
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            activate(cx);
        })
        .child(div().min_w_0().truncate().child(title))
        .children(subtitle.map(|subtitle| {
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::NORMAL)
                .text_color(rgb(theme::muted()))
                .child(label("profile-menu-subtitle", &subtitle))
        }))
}

pub(in crate::app::profile) fn profile_actions(
    app: &DiscordApp,
    card: &ProfileCard,
    window: &Window,
) -> Option<AnyElement> {
    if card.is_account() || card.user_id == app.store.current_user_id {
        return None;
    }
    let open = card.more_menu_open;
    let margin = rpx(8.).to_pixels(window.rem_size());
    let max_width = (window.viewport_size().width - margin * 2.).max(px(0.));
    let trigger_bounds: MenuBounds = Rc::new(Cell::new(None));
    let trigger_paint = trigger_bounds.clone();
    let palette = card.palette();
    let toggle = app.entity.clone();
    let button =
        theme::icon_button_with_hover("profile-more", "ellipsis", 20., palette.text, palette.hover)
            .role(Role::Button)
            .aria_label("More")
            .aria_expanded(open)
            .focusable()
            .size(rpx(32.))
            .rounded(rpx(8.))
            .bg(if open { palette.raised } else { palette.button })
            .focus_visible(move |style| style.bg(palette.raised))
            .without_text_selection()
            .on_prepaint(move |bounds, _, _| trigger_paint.set(Some(bounds)))
            .map(|this| action_tooltip(this, app, card, "More", !open))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                toggle
                    .update(cx, |app, cx| app.set_profile_menu(!open, cx))
                    .ok();
            });
    let friend_action = app.friend_action(card.user_id);
    let friend_error = app.friend_error(card.user_id);
    let friends = friend_action == Some(FriendAction::Friends);
    let more = div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .when(friends, |this| this.child(message_button(app, card, true)))
        .children(friend_action.map(|action| friend_button(app, card, action)))
        .when(!friends, |this| {
            this.child(message_button(app, card, false))
        })
        .child(div().relative().child(button).when(open, |this| {
            let close = app.entity.clone();
            let switch = app.entity.clone();
            let copy = app.entity.clone();
            let friend = app.entity.clone();
            let user_id = card.user_id;
            let server = card.guild_id.is_some();
            let subtitle = if server {
                app.store
                    .user(user_id)
                    .filter(|user| !user.name.is_empty())
                    .map(|user| format!("AKA {}", user.name))
            } else {
                card.source_guild_id
                    .and_then(|id| app.store.guild_by_id(id))
                    .map(|guild| guild.name.clone())
            };
            let menu = div()
                .id("profile-more-menu")
                .role(Role::Menu)
                .flex()
                .flex_col()
                .w(rpx(196.))
                .max_w(max_width)
                .p(rpx(6.))
                .rounded(rpx(8.))
                .bg(rgb(theme::elevated()))
                .shadow_lg()
                .occlude()
                .on_mouse_down_out(move |event, _, cx| {
                    // The trigger closes on click; closing on mouse-down would reopen it.
                    if trigger_bounds
                        .get()
                        .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        return;
                    }
                    close
                        .update(cx, |app, cx| app.set_profile_menu(false, cx))
                        .ok();
                })
                .on_click(|_, _, cx| cx.stop_propagation())
                .when(card.can_switch_scope(), |this| {
                    this.child(menu_entry(
                        "profile-switch-scope",
                        if server {
                            "View Main Profile"
                        } else {
                            "View Server Profile"
                        },
                        subtitle,
                        move |cx| {
                            switch
                                .update(cx, |app, cx| app.switch_profile_scope(cx))
                                .ok();
                        },
                    ))
                    .child(
                        div()
                            .h(rpx(1.))
                            .my(rpx(4.))
                            .mx(rpx(6.))
                            .bg(rgba(theme::border())),
                    )
                })
                .children(friend_action.and_then(|action| {
                    let (id, title) = match action {
                        FriendAction::Friends => ("profile-menu-remove-friend", "Remove Friend"),
                        FriendAction::Accept => {
                            ("profile-menu-ignore-friend", "Ignore Friend Request")
                        }
                        FriendAction::Sent => {
                            ("profile-menu-cancel-friend", "Cancel Friend Request")
                        }
                        FriendAction::Add => return None,
                    };
                    Some(
                        menu_entry(id, title, None, move |cx| {
                            friend
                                .update(cx, |app, cx| {
                                    if action == FriendAction::Friends {
                                        app.ask_remove_friend(user_id, cx);
                                    } else {
                                        app.drop_friend_request(user_id, cx);
                                        app.set_profile_menu(false, cx);
                                    }
                                })
                                .ok();
                        })
                        .text_color(rgb(theme::RED)),
                    )
                }))
                .child(menu_entry(
                    "profile-menu-copy-id",
                    "Copy User ID",
                    None,
                    move |cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(user_id.to_string()));
                        copy.update(cx, |app, cx| app.set_profile_menu(false, cx))
                            .ok();
                    },
                ));
            this.child(
                div().absolute().top_0().left(rpx(40.)).size_0().child(
                    deferred(
                        anchored()
                            .anchor(Anchor::TopLeft)
                            .snap_to_window_with_margin(Edges::all(margin))
                            .child(menu),
                    )
                    .with_priority(3),
                ),
            )
        }));
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(6.))
            .child(more)
            .children(friend_error.map(|text| {
                div()
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(theme::RED))
                    .child(text)
            }))
            .into_any_element(),
    )
}

fn action_tooltip(
    button: Stateful<Div>,
    app: &DiscordApp,
    card: &ProfileCard,
    tip: &'static str,
    enabled: bool,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    button
        .relative()
        .on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, cx| {
                    if let Some(card) = &mut app.profile {
                        if *hovered {
                            card.hovered_action = Some(tip);
                        } else if card.hovered_action == Some(tip) {
                            card.hovered_action = None;
                        }
                        cx.notify();
                    }
                })
                .ok();
        })
        .when(enabled && card.hovered_action == Some(tip), |this| {
            this.child(theme::tooltip_above(tip))
        })
}

fn message_button(app: &DiscordApp, card: &ProfileCard, labelled: bool) -> AnyElement {
    let user_id = card.user_id;
    let palette = card.palette();
    let entity = app.entity.clone();
    let open = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
        cx.stop_propagation();
        entity
            .update(cx, |app, cx| app.open_dm(user_id, window, cx))
            .ok();
    };
    if !labelled {
        return theme::icon_button_with_hover(
            "profile-message",
            "message-circle",
            18.,
            palette.text,
            palette.hover,
        )
        .role(Role::Button)
        .aria_label("Message")
        .size(rpx(32.))
        .rounded(rpx(8.))
        .bg(palette.button)
        .without_text_selection()
        .map(|this| action_tooltip(this, app, card, "Message", true))
        .on_click(open)
        .into_any_element();
    }
    let (fill, hover): (Hsla, Hsla) = if palette.light {
        (rgb(palette.strong).into(), rgb(palette.text).into())
    } else {
        (
            rgb(theme::BLURPLE).into(),
            rgb(theme::BLURPLE_ACTIVE).into(),
        )
    };
    div()
        .id("profile-message")
        .role(Role::Button)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(32.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(fill)
        .hover(move |style| style.bg(hover))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(white())
        .without_text_selection()
        .on_click(open)
        .child(theme::icon("message-circle", 16., 0xFFFFFF))
        .child("Message")
        .into_any_element()
}

fn friend_button(app: &DiscordApp, card: &ProfileCard, action: FriendAction) -> AnyElement {
    let user_id = card.user_id;
    let palette = card.palette();
    let entity = app.entity.clone();
    // A status badge, not a button: Discord only removes friends from the Home friends list.
    if action == FriendAction::Friends {
        return div()
            .id("profile-friend")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .aria_label("Friends")
            .size(rpx(32.))
            .rounded(rpx(8.))
            .bg(palette.button)
            .without_text_selection()
            .child(theme::icon("user-check", 18., palette.text))
            .map(|this| action_tooltip(this, app, card, "Friends", true))
            .into_any_element();
    }
    let (title, primary) = match action {
        FriendAction::Add => ("Add Friend", true),
        FriendAction::Accept => ("Accept", true),
        _ => ("Request Sent", false),
    };
    let needs_captcha = action == FriendAction::Add;
    let busy = app.friend_busy(user_id);
    let color = if primary { 0xFFFFFF } else { theme::muted() };
    div()
        .id("profile-friend")
        .role(Role::Button)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(32.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .without_text_selection()
        .text_color(rgb(color))
        .when(!primary, |this| this.bg(rgba(theme::selected())))
        .when(primary, |this| this.bg(rgb(theme::BLURPLE)))
        .when(primary && (busy || needs_captcha), |this| this.opacity(0.5))
        .when(needs_captcha, |this| {
            action_tooltip(
                this,
                app,
                card,
                "Send friend requests from Discord for now",
                true,
            )
        })
        .when(primary && !busy && !needs_captcha, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    entity
                        .update(cx, |app, cx| app.add_friend(user_id, cx))
                        .ok();
                })
        })
        .child(theme::icon("user-plus", 16., color))
        .child(title)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{
        Context, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, Render, TestAppContext, Window,
        div, prelude::*, px,
    };

    use super::menu_entry;
    use crate::theme::rpx;

    struct MenuSample {
        activated: Rc<Cell<usize>>,
    }

    impl Render for MenuSample {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let activated = self.activated.clone();
            div()
                .flex()
                .flex_col()
                .w(rpx(196.))
                .p(rpx(6.))
                .debug_selector(|| "profile-menu-sample".into())
                .child(
                    menu_entry(
                        "profile-menu-sample-scope",
                        "View Main Profile",
                        Some("AKA Example\u{1f343} with a very long display name".into()),
                        move |_| activated.set(activated.get() + 1),
                    )
                    .debug_selector(|| "profile-menu-sample-scope".into()),
                )
                .child(
                    menu_entry("profile-menu-sample-copy", "Copy User ID", None, |_| {})
                        .debug_selector(|| "profile-menu-sample-copy".into()),
                )
        }
    }

    #[gpui::test]
    fn menu_entries_fit_and_activate_with_mouse_and_keyboard(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let activated = Rc::new(Cell::new(0));
        let (_, cx) = cx.add_window_view(|_, _| MenuSample {
            activated: activated.clone(),
        });
        for zoom in [1., 1.25, 1.5] {
            cx.update(|window, cx| {
                window.set_rem_size(px(16. * zoom));
                window.refresh();
                let _ = window.draw(cx);
            });
            let menu = cx.debug_bounds("profile-menu-sample").unwrap();
            let scope = cx.debug_bounds("profile-menu-sample-scope").unwrap();
            let copy = cx.debug_bounds("profile-menu-sample-copy").unwrap();
            for entry in [scope, copy] {
                assert!(entry.left() >= menu.left());
                assert!(entry.right() <= menu.right());
            }
            assert!(scope.bottom() <= copy.top());
            assert!(scope.size.height > copy.size.height);

            let before = activated.get();
            cx.simulate_click(scope.center(), Modifiers::none());
            assert_eq!(activated.get(), before + 1);
            cx.update(|window, cx| {
                window.refresh();
                let _ = window.draw(cx);
            });
            for key in ["enter", "space"] {
                let keystroke = Keystroke::parse(key).unwrap();
                cx.simulate_event(KeyDownEvent {
                    keystroke: keystroke.clone(),
                    is_held: false,
                    prefer_character_input: false,
                });
                cx.simulate_event(KeyUpEvent { keystroke });
            }
            assert_eq!(activated.get(), before + 3);
        }
    }
}
