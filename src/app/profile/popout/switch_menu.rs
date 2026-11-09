use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{prelude::*, *};

use super::super::ProfileCard;
use super::status::{flyout, flyout_position, menu_width};
use crate::{
    app::DiscordApp,
    settings::SavedAccount,
    theme::{self, WithoutTextSelection, rpx},
};

const MENU_WIDTH: f32 = 220.;
const CLOSE_DELAY: Duration = Duration::from_millis(180);
const AVATAR: f32 = 24.;

#[derive(Clone, Copy)]
enum Part {
    Trigger,
    Menu,
}

type Bounded = Rc<Cell<[Option<Bounds<Pixels>>; 2]>>;

#[derive(Default)]
pub(in crate::app::profile) struct State {
    open: bool,
    hovered: [bool; 2],
    bounds: Bounded,
    close_task: Option<Task<()>>,
}

impl State {
    pub(in crate::app::profile) fn hit_test(&self) -> impl Fn(Point<Pixels>) -> bool + use<> {
        let bounds = self.bounds.clone();
        move |point| {
            bounds
                .get()
                .iter()
                .flatten()
                .any(|bounds| bounds.contains(&point))
        }
    }

    fn close(&mut self) {
        self.open = false;
        self.hovered = [false; 2];
        self.close_task = None;
        let mut bounds = self.bounds.get();
        bounds[1] = None;
        self.bounds.set(bounds);
    }

    fn idle(&self) -> bool {
        !self.hovered.iter().any(|hovered| *hovered)
    }
}

impl DiscordApp {
    fn switch_menu_mut(&mut self) -> Option<&mut State> {
        self.profile
            .as_mut()
            .filter(|card| card.is_account())
            .map(|card| &mut card.switch_menu)
    }

    fn hover_switch_menu(&mut self, part: Part, hovered: bool, cx: &mut Context<Self>) {
        let Some(menu) = self.switch_menu_mut() else {
            return;
        };
        menu.close_task = None;
        menu.hovered[part as usize] = hovered;
        if hovered && matches!(part, Part::Trigger) {
            menu.open = true;
        }
        if menu.open && menu.idle() {
            let bounds = menu.bounds.clone();
            menu.close_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CLOSE_DELAY).await;
                this.update(cx, |app, cx| {
                    if let Some(menu) = app.switch_menu_mut()
                        && Rc::ptr_eq(&menu.bounds, &bounds)
                        && menu.idle()
                    {
                        menu.close();
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        cx.notify();
    }

    fn open_switch_menu(&mut self, cx: &mut Context<Self>) {
        if let Some(menu) = self.switch_menu_mut() {
            menu.open = true;
            menu.close_task = None;
            cx.notify();
        }
    }

    pub(crate) fn dismiss_switch_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.switch_menu_mut().filter(|menu| menu.open) else {
            return false;
        };
        menu.close();
        cx.notify();
        true
    }

    fn pick_account(&mut self, user_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.close_profile(cx);
        self.switch_account(user_id, window, cx);
    }
}

fn record(bounds: &Bounded, index: usize, open: bool) -> impl IntoElement {
    let bounds = bounds.clone();
    canvas(
        move |value, window, _| {
            let mut saved = bounds.get();
            if saved[index] != Some(value) {
                saved[index] = Some(value);
                bounds.set(saved);
                if open {
                    window.refresh();
                }
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub(super) fn trigger(
    app: &DiscordApp,
    card: &ProfileCard,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let state = &card.switch_menu;
    let palette = card.palette();
    let hover = app.entity.clone();
    let click = app.entity.clone();
    let key = app.entity.clone();
    let row = div()
        .id("account-switch-trigger")
        .role(Role::MenuItem)
        .aria_label("Switch Accounts")
        .aria_expanded(state.open)
        .focusable()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .min_h(rpx(36.))
        .px(rpx(12.))
        .py(rpx(8.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .text_size(rpx(14.))
        .line_height(rpx(20.))
        .text_color(rgb(palette.text))
        .hover(|style| style.bg(palette.hover).text_color(rgb(palette.strong)))
        .focus_visible(|style| style.bg(palette.hover))
        .when(state.open, |this| this.bg(palette.hover))
        .without_text_selection()
        .child(record(&state.bounds, 0, state.open))
        .on_hover(move |hovered, _, cx| {
            hover
                .update(cx, |app, cx| {
                    app.hover_switch_menu(Part::Trigger, *hovered, cx)
                })
                .ok();
        })
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            click.update(cx, |app, cx| app.open_switch_menu(cx)).ok();
        })
        .on_key_down(move |event, _, cx| {
            if matches!(event.keystroke.key.as_str(), "right" | "enter" | "space") {
                cx.stop_propagation();
                key.update(cx, |app, cx| app.open_switch_menu(cx)).ok();
            }
        })
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(18.))
                .child(theme::icon("user-circle", 18., palette.muted)),
        )
        .child(div().flex_1().min_w_0().truncate().child("Switch Accounts"))
        .child(
            theme::icon("chevron-down", 16., palette.muted).with_transformation(
                Transformation::rotate(radians(-std::f32::consts::FRAC_PI_2)),
            ),
        );
    let mut root = div().relative().child(row);
    if state.open
        && let Some(anchor) = state.bounds.get()[0]
    {
        let width = menu_width(MENU_WIDTH, window);
        let accounts = app.switchable_accounts(cx);
        let estimate = 50. + 40. * accounts.len() as f32 + 17.;
        let height = state.bounds.get()[1]
            .map_or(rpx(estimate).to_pixels(window.rem_size()), |bounds| {
                bounds.size.height
            });
        let position = flyout_position(
            anchor,
            size(width, height),
            window.viewport_size(),
            rpx(4.).to_pixels(window.rem_size()),
            rpx(8.).to_pixels(window.rem_size()),
            anchor.size.height / 2.,
        );
        root = root.child(flyout(
            menu(app, state, &accounts, width, cx),
            position,
            window,
            3,
        ));
    }
    root.into_any_element()
}

fn menu(
    app: &DiscordApp,
    state: &State,
    accounts: &[SavedAccount],
    width: Pixels,
    cx: &App,
) -> AnyElement {
    let hover = app.entity.clone();
    let outside = app.entity.clone();
    let manage = app.entity.clone();
    let bounds = state.bounds.clone();
    let current = app.current_account_id();
    div()
        .id("account-switch-menu")
        .role(Role::Menu)
        .aria_label("Switch Accounts")
        .flex()
        .flex_col()
        .w(width)
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .text_color(rgb(theme::text()))
        .shadow_lg()
        .occlude()
        .child(record(&state.bounds, 1, state.open))
        .on_hover(move |hovered, _, cx| {
            hover
                .update(cx, |app, cx| {
                    app.hover_switch_menu(Part::Menu, *hovered, cx)
                })
                .ok();
        })
        .on_mouse_down_out(move |event, _, cx| {
            if !bounds
                .get()
                .iter()
                .flatten()
                .any(|bounds| bounds.contains(&event.position))
            {
                outside
                    .update(cx, |app, cx| app.dismiss_switch_menu(cx))
                    .ok();
            }
        })
        .on_click(|_, _, cx| cx.stop_propagation())
        .children(
            accounts
                .iter()
                .map(|account| account_item(app, account, current == Some(account.id), cx)),
        )
        .child(
            div()
                .flex_none()
                .h(px(1.))
                .my(rpx(8.))
                .bg(rgba(theme::border())),
        )
        .child(
            item("account-switch-manage")
                .min_h(rpx(34.))
                .font_weight(FontWeight::MEDIUM)
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    manage
                        .update(cx, |app, cx| app.open_manage_accounts(cx))
                        .ok();
                })
                .child("Manage Accounts"),
        )
        .into_any_element()
}

fn item(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::MenuItem)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(8.))
        .p(rpx(8.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .hover(|style| style.bg(rgba(theme::hover())))
        .without_text_selection()
}

fn account_item(app: &DiscordApp, account: &SavedAccount, current: bool, cx: &App) -> AnyElement {
    let user_id = account.id;
    let entity = app.entity.clone();
    let discriminator = account
        .discriminator
        .as_deref()
        .filter(|discriminator| !discriminator.is_empty() && *discriminator != "0");
    let spec = crate::store::avatar_spec(user_id, account.avatar.as_deref(), discriminator);
    let marker = if app.accounts.busy() == Some(user_id) {
        Some(theme::spinner(16., theme::muted()).into_any_element())
    } else if current {
        Some(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(18.))
                .rounded_full()
                .bg(rgb(theme::BLURPLE))
                .child(theme::icon("check-bold", 10., 0xFFFFFF))
                .into_any_element(),
        )
    } else if account.invalid {
        Some(theme::icon("circle-alert-filled", 16., theme::RED).into_any_element())
    } else {
        None
    };
    item(("account-switch", user_id))
        .min_h(rpx(40.))
        .aria_label(account.username.clone())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity
                .update(cx, |app, cx| app.pick_account(user_id, window, cx))
                .ok();
        })
        .child(theme::spec_avatar(
            &spec,
            user_id,
            &account.username,
            AVATAR,
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .child(div().min_w_0().truncate().child(account.username.clone()))
                .children(discriminator.map(|discriminator| {
                    div()
                        .flex_none()
                        .text_color(rgb(theme::muted()))
                        .child(format!("#{discriminator}"))
                })),
        )
        .children(marker)
        .into_any_element()
}
