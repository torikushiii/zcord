use gpui::{prelude::*, *};

use crate::{
    app::{DiscordApp, dm_menu::DmDialog},
    store::RelationshipKind,
    theme::{self, WithoutTextSelection, rpx},
};

const MENU_WIDTH: f32 = 190.;

#[derive(Clone, Copy)]
pub(crate) struct MemberMenu {
    user_id: u64,
    position: Point<Pixels>,
}

impl DiscordApp {
    pub(crate) fn open_member_menu(
        &mut self,
        user_id: u64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.member_menu = Some(MemberMenu { user_id, position });
        cx.notify();
    }

    pub(crate) fn close_member_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.member_menu.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }
}

fn menu_row(
    app: &DiscordApp,
    key: &'static str,
    icon: Option<&'static str>,
    title: &'static str,
    color: u32,
    run: impl Fn(&mut DiscordApp, &mut Window, &mut Context<DiscordApp>) + 'static,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    div()
        .id(key)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(10.))
        .min_h(rpx(32.))
        .px(rpx(8.))
        .rounded(rpx(4.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::selected())))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .without_text_selection()
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity
                .update(cx, |app, cx| {
                    app.close_member_menu(cx);
                    run(app, window, cx);
                })
                .ok();
        })
        .children(icon.map(|icon| theme::icon(icon, 18., theme::muted())))
        .child(div().flex_1().min_w_0().truncate().child(title))
}

fn divider() -> Div {
    div()
        .flex_none()
        .h(rpx(1.))
        .my(rpx(8.))
        .mx(rpx(6.))
        .bg(rgba(theme::border()))
}

pub(crate) fn menu(app: &DiscordApp) -> Option<AnyElement> {
    let menu = app.member_menu?;
    let user_id = menu.user_id;
    let other = user_id != app.store.current_user_id;
    let relationship = app.store.relationship(user_id);
    let blocked = relationship.is_some_and(|found| found.kind == RelationshipKind::Blocked);
    let ignored = relationship.is_some_and(|found| found.ignored);
    let can_mention = app.store.can_send(app.selected_channel_id) && app.selected_forum().is_none();
    let actions = other && app.live;
    let text = theme::text();
    let close = app.entity.clone();
    let body = div()
        .id("member-menu")
        .flex()
        .flex_col()
        .gap(rpx(2.))
        .w(rpx(MENU_WIDTH))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
        .occlude()
        .on_mouse_down_out(move |_, _, cx| {
            close.update(cx, |app, cx| app.close_member_menu(cx)).ok();
        })
        .child(menu_row(
            app,
            "member-menu-profile",
            None,
            "Profile",
            text,
            move |app, _, cx| app.open_full_profile_for(user_id, cx),
        ))
        .when(can_mention, |this| {
            this.child(menu_row(
                app,
                "member-menu-mention",
                None,
                "Mention",
                text,
                move |app, window, cx| app.mention_user(user_id, window, cx),
            ))
        })
        .when(actions, |this| {
            this.child(menu_row(
                app,
                "member-menu-message",
                None,
                "Message",
                text,
                move |app, window, cx| app.open_dm(user_id, window, cx),
            ))
            .child(divider())
            .child(if ignored {
                menu_row(
                    app,
                    "member-menu-ignore",
                    None,
                    "Unignore",
                    text,
                    move |app, _, cx| app.unignore_user(user_id, cx),
                )
            } else {
                menu_row(
                    app,
                    "member-menu-ignore",
                    None,
                    "Ignore",
                    text,
                    move |app, _, cx| app.ignore_user(user_id, cx),
                )
            })
            .child(if blocked {
                menu_row(
                    app,
                    "member-menu-block",
                    None,
                    "Unblock",
                    text,
                    move |app, _, cx| app.unblock_user(user_id, cx),
                )
            } else {
                menu_row(
                    app,
                    "member-menu-block",
                    None,
                    "Block",
                    theme::RED,
                    move |app, _, cx| app.open_dm_dialog(DmDialog::Block(user_id), cx),
                )
            })
        })
        .child(divider())
        .child(menu_row(
            app,
            "member-menu-copy-id",
            Some("id-badge"),
            "Copy User ID",
            text,
            move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(user_id.to_string())),
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
