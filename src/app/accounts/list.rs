use gpui_component::ElementExt as _;

use super::modal::button;
use super::*;
use crate::theme::WithoutTextSelection;

const AVATAR: f32 = 40.;

#[derive(Clone, Copy)]
pub(crate) enum ListPlace {
    Manage,
    Chooser { busy: bool },
}

pub(crate) fn account_list(
    saved: &[SavedAccount],
    state: &Accounts,
    place: ListPlace,
    current: Option<u64>,
    entity: WeakEntity<DiscordApp>,
    cx: &App,
) -> AnyElement {
    let mut list = div()
        .flex()
        .flex_col()
        .w_full()
        .rounded(rpx(8.))
        .bg(rgba(theme::control_bg()));
    for (index, account) in saved.iter().enumerate() {
        if index > 0 {
            list = list.child(
                div()
                    .w_full()
                    .pl(rpx(16.))
                    .child(div().h(px(1.)).w_full().bg(rgba(theme::border_subtle()))),
            );
        }
        list = list.child(row(account, state, place, current, entity.clone(), cx));
    }
    list.into_any_element()
}

fn row(
    account: &SavedAccount,
    state: &Accounts,
    place: ListPlace,
    current: Option<u64>,
    entity: WeakEntity<DiscordApp>,
    cx: &App,
) -> AnyElement {
    let user_id = account.id;
    let is_current = current == Some(user_id);
    let waiting = state.validating(user_id) || state.busy() == Some(user_id);
    let locked = match place {
        ListPlace::Manage => state.busy().is_some(),
        ListPlace::Chooser { busy } => busy || state.busy().is_some(),
    };
    let hint = if is_current {
        Some(("Active account", theme::GREEN))
    } else if account.invalid {
        Some(("Please log in again.", theme::RED))
    } else {
        None
    };
    let action = match place {
        ListPlace::Manage if is_current => None,
        ListPlace::Manage if account.invalid => Some(("Log in", true)),
        ListPlace::Manage => Some(("Switch", false)),
        ListPlace::Chooser { .. } => Some(("Log in", true)),
    };
    let discriminator = account
        .discriminator
        .as_deref()
        .filter(|discriminator| !discriminator.is_empty() && *discriminator != "0");
    let spec = crate::store::avatar_spec(user_id, account.avatar.as_deref(), discriminator);
    let names = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .ml(rpx(8.))
        .child(
            div()
                .flex()
                .items_center()
                .min_w_0()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(theme::strong()))
                        .child(account.username.clone()),
                )
                .children(discriminator.map(|discriminator| {
                    div()
                        .flex_none()
                        .text_color(rgb(theme::muted()))
                        .child(format!("#{discriminator}"))
                })),
        )
        .children(hint.map(|(text, color)| {
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(color))
                .child(text)
        }));
    let actions = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .ml_auto()
        .pl(rpx(8.))
        .map(|this| {
            if waiting {
                return this.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(rpx(40.))
                        .child(theme::spinner(16., theme::muted())),
                );
            }
            this.children(action.map(|(label, primary)| {
                let entity = entity.clone();
                button(("account-action", user_id), label, primary, !locked).when(!locked, |this| {
                    this.on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        entity
                            .update(cx, |app, cx| match place {
                                ListPlace::Manage if label == "Log in" => {
                                    app.open_add_account(window, cx)
                                }
                                _ => app.switch_account(user_id, window, cx),
                            })
                            .ok();
                    })
                })
            }))
            .child(more_button(
                user_id,
                state.menu == Some(user_id),
                entity.clone(),
            ))
        });
    div()
        .id(("account-row", user_id))
        .flex()
        .items_center()
        .w_full()
        .px(rpx(16.))
        .py(rpx(12.))
        .child(theme::spec_avatar(
            &spec,
            user_id,
            &account.username,
            AVATAR,
            cx,
        ))
        .child(names)
        .child(actions)
        .into_any_element()
}

fn more_button(user_id: u64, open: bool, entity: WeakEntity<DiscordApp>) -> AnyElement {
    let trigger: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
    let paint = trigger.clone();
    let toggle = entity.clone();
    let button = theme::icon_button(("account-more", user_id), "ellipsis", 20., theme::muted())
        .role(Role::Button)
        .aria_label("More Options")
        .aria_expanded(open)
        .size(rpx(40.))
        .rounded(rpx(8.))
        .when(open, |this| this.bg(rgba(theme::hover())))
        .on_prepaint(move |bounds, _, _| paint.set(Some(bounds)))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            toggle
                .update(cx, |app, cx| app.toggle_account_menu(user_id, cx))
                .ok();
        });
    div()
        .relative()
        .child(button)
        .when(open, |this| {
            let close = entity.clone();
            let log_out = entity;
            let menu = div()
                .id(("account-menu", user_id))
                .role(Role::Menu)
                .flex()
                .flex_col()
                .w(rpx(144.))
                .p(rpx(8.))
                .rounded(rpx(8.))
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgba(theme::border()))
                .shadow_lg()
                .occlude()
                .on_mouse_down_out(move |event, _, cx| {
                    if trigger
                        .get()
                        .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        return;
                    }
                    close.update(cx, |app, cx| app.close_account_menu(cx)).ok();
                })
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id(("account-log-out", user_id))
                        .role(Role::MenuItem)
                        .flex()
                        .items_center()
                        .p(rpx(8.))
                        .rounded(rpx(4.))
                        .cursor_pointer()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::RED))
                        .hover(|style| style.bg(rgba(theme::hover())))
                        .without_text_selection()
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            log_out
                                .update(cx, |app, cx| app.log_out_account(user_id, window, cx))
                                .ok();
                        })
                        .child("Log out"),
                );
            this.child(
                div()
                    .absolute()
                    .top(rpx(20.))
                    .left(rpx(20.))
                    .size_0()
                    .child(
                        deferred(
                            anchored()
                                .anchor(Anchor::TopLeft)
                                .snap_to_window_with_margin(Edges::all(px(8.)))
                                .child(menu),
                        )
                        .with_priority(4),
                    ),
            )
        })
        .into_any_element()
}

pub(crate) fn add_account_link(
    id: &'static str,
    enabled: bool,
    activate: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::link()))
        .without_text_selection()
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.underline())
                .on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    activate(event, window, cx);
                })
        })
        .when(!enabled, |this| this.opacity(0.5))
        .child("Add an account")
}
