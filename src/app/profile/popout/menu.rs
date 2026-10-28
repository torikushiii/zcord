use gpui_component::ElementExt;

use super::super::*;

pub(in crate::app::profile) type MenuBounds = Rc<Cell<Option<Bounds<Pixels>>>>;

struct MoreTooltip;

impl Render for MoreTooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        theme::tooltip_bubble()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .child("More")
    }
}

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
        .text_color(rgb(theme::STRONG))
        .hover(|style| style.bg(rgba(theme::SELECTED)))
        .focus_visible(|style| style.bg(rgba(theme::SELECTED)))
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
                .text_color(rgb(theme::MUTED))
                .child(label("profile-menu-subtitle", &subtitle))
        }))
}

pub(in crate::app::profile) fn profile_actions(
    app: &DiscordApp,
    card: &ProfileCard,
    window: &Window,
    bounds: Option<MenuBounds>,
) -> Option<AnyElement> {
    if card.is_account() {
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
            .tooltip(|_, cx| cx.new(|_| MoreTooltip).into())
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                toggle
                    .update(cx, |app, cx| app.set_profile_menu(!open, cx))
                    .ok();
            });
    Some(
        div()
            .flex()
            .items_center()
            .child(div().relative().child(button).when(open, |this| {
                let close = app.entity.clone();
                let switch = app.entity.clone();
                let copy = app.entity.clone();
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
                    .bg(rgb(theme::ELEVATED))
                    .shadow_lg()
                    .occlude()
                    .on_prepaint(move |menu_bounds, _, _| {
                        if let Some(bounds) = &bounds {
                            bounds.set(Some(menu_bounds));
                        }
                    })
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
                                .bg(rgba(theme::BORDER)),
                        )
                    })
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
            }))
            .into_any_element(),
    )
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
