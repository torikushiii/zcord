use gpui::{prelude::*, *};

use crate::{
    app::{DiscordApp, direct::dm_menu::MUTE_CHOICES},
    discord::model::{GuildNotificationFlag, GuildNotificationFlags, NotificationLevel},
    theme::{self, WithoutTextSelection, rpx},
};

const MENU_WIDTH: f32 = 190.;
const MUTE_WIDTH: f32 = 188.;
const NOTIFICATIONS_WIDTH: f32 = 256.;
const NOTIFICATION_CHOICES: [(&str, NotificationLevel); 3] = [
    ("All Messages", NotificationLevel::All),
    ("Only @mentions", NotificationLevel::Mentions),
    ("Nothing", NotificationLevel::Nothing),
];
const SUPPRESSION_FLAGS: [(&str, GuildNotificationFlag); 4] = [
    (
        "Suppress @everyone and @here",
        GuildNotificationFlag::SuppressEveryone,
    ),
    (
        "Suppress All Role @mentions",
        GuildNotificationFlag::SuppressRoles,
    ),
    (
        "Suppress Highlights",
        GuildNotificationFlag::SuppressHighlights,
    ),
    (
        "Mute New Events",
        GuildNotificationFlag::MuteScheduledEvents,
    ),
];

#[derive(Clone, Copy, PartialEq)]
enum Submenu {
    None,
    Mute,
    Notifications,
}

#[derive(Clone, Copy)]
pub(crate) struct GuildMenu {
    guild_id: u64,
    position: Point<Pixels>,
    submenu: Submenu,
    /// The flyout lies outside the menu's bounds; a click there must not count as outside.
    pointer_in_flyout: bool,
}

impl DiscordApp {
    pub(crate) fn open_guild_menu(
        &mut self,
        guild_id: u64,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.menus.guild = Some(GuildMenu {
            guild_id,
            position,
            submenu: Submenu::None,
            pointer_in_flyout: false,
        });
        cx.notify();
    }

    pub(crate) fn close_guild_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.menus.guild.take().is_none() {
            return false;
        }
        cx.notify();
        true
    }

    fn show_guild_submenu(&mut self, submenu: Submenu, cx: &mut Context<Self>) {
        if let Some(menu) = &mut self.menus.guild
            && menu.submenu != submenu
        {
            menu.submenu = submenu;
            menu.pointer_in_flyout = false;
            cx.notify();
        }
    }

    fn set_pointer_in_guild_flyout(&mut self, inside: bool) {
        if let Some(menu) = &mut self.menus.guild {
            menu.pointer_in_flyout = inside;
        }
    }
}

fn flags_value(flags: GuildNotificationFlags, flag: GuildNotificationFlag) -> bool {
    match flag {
        GuildNotificationFlag::SuppressEveryone => flags.suppress_everyone,
        GuildNotificationFlag::SuppressRoles => flags.suppress_roles,
        GuildNotificationFlag::SuppressHighlights => flags.suppress_highlights,
        GuildNotificationFlag::MuteScheduledEvents => flags.mute_scheduled_events,
        GuildNotificationFlag::MobilePush => flags.mobile_push,
    }
}

fn row_base(key: impl Into<ElementId>, color: u32) -> Stateful<Div> {
    inert_row(key, color)
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::selected())))
}

fn inert_row(key: impl Into<ElementId>, color: u32) -> Stateful<Div> {
    div()
        .id(key)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(10.))
        .min_h(rpx(32.))
        .px(rpx(8.))
        .rounded(rpx(4.))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .without_text_selection()
}

fn closes_submenu(app: &DiscordApp, row: Stateful<Div>) -> Stateful<Div> {
    let entity = app.entity.clone();
    row.on_hover(move |hovered, _, cx| {
        if *hovered {
            entity
                .update(cx, |app, cx| app.show_guild_submenu(Submenu::None, cx))
                .ok();
        }
    })
}

fn title_block(title: &'static str, subtitle: Option<String>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(rpx(2.))
        .child(div().truncate().child(title))
        .children(subtitle.map(|subtitle| {
            div()
                .truncate()
                .text_size(rpx(12.))
                .font_weight(FontWeight::NORMAL)
                .text_color(rgb(theme::muted()))
                .child(subtitle)
        }))
}

fn menu_row(
    app: &DiscordApp,
    key: &'static str,
    icon: Option<&'static str>,
    title: &'static str,
    subtitle: Option<String>,
    run: impl Fn(&mut DiscordApp, &mut Window, &mut Context<DiscordApp>) + 'static,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    let row = row_base(key, theme::text())
        .when(subtitle.is_some(), |this| this.py(rpx(7.)))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            entity
                .update(cx, |app, cx| {
                    app.close_guild_menu(cx);
                    run(app, window, cx);
                })
                .ok();
        })
        .children(icon.map(|icon| theme::icon(icon, 18., theme::muted())))
        .child(title_block(title, subtitle));
    closes_submenu(app, row)
}

fn submenu_row(
    app: &DiscordApp,
    key: &'static str,
    title: &'static str,
    subtitle: Option<&'static str>,
    submenu: Submenu,
    flyout: Option<Div>,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    let open = flyout.is_some();
    row_base(key, theme::text())
        .relative()
        .when(subtitle.is_some(), |this| this.py(rpx(7.)))
        .when(open, |this| this.bg(rgba(theme::selected())))
        .on_hover(move |hovered, _, cx| {
            if *hovered {
                entity
                    .update(cx, |app, cx| app.show_guild_submenu(submenu, cx))
                    .ok();
            }
        })
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(title_block(title, subtitle.map(str::to_owned)))
        .child(theme::icon("chevron-right", 16., theme::muted()))
        .children(flyout)
}

fn panel(id: &'static str, width: f32) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(rpx(2.))
        .w(rpx(width))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
        .occlude()
}

/// Places a flyout panel beside the row that opened it, its first row level with that row.
fn flyout(app: &DiscordApp, panel: Stateful<Div>) -> Div {
    let entity = app.entity.clone();
    div()
        .absolute()
        .top(rpx(-9.))
        .left(relative(1.))
        .pl(rpx(14.))
        .child(panel.on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, _| app.set_pointer_in_guild_flyout(*hovered))
                .ok();
        }))
}

fn radio(selected: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(20.))
        .rounded_full()
        .map(|this| {
            if selected {
                this.bg(rgb(theme::BLURPLE))
                    .child(div().size(rpx(8.)).rounded_full().bg(rgb(0xFFFFFF)))
            } else {
                this.border_2().border_color(rgb(theme::muted()))
            }
        })
}

fn checkbox(checked: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(20.))
        .rounded(rpx(4.))
        .map(|this| {
            if checked {
                this.bg(rgb(theme::BLURPLE))
                    .child(theme::icon("check", 14., 0xFFFFFF))
            } else {
                this.border_1().border_color(rgb(theme::muted()))
            }
        })
}

/// A flyout row that stays open after the click, like Discord's toggles.
fn toggle_row(
    app: &DiscordApp,
    key: impl Into<ElementId>,
    label: &'static str,
    control: Div,
    run: impl Fn(&mut DiscordApp, &mut Context<DiscordApp>) + 'static,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    row_base(key, theme::text())
        .min_h(rpx(34.))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            entity.update(cx, |app, cx| run(app, cx)).ok();
        })
        .child(div().flex_1().min_w_0().truncate().child(label))
        .child(control)
}

fn divider() -> Div {
    div()
        .flex_none()
        .h(rpx(1.))
        .my(rpx(8.))
        .mx(rpx(6.))
        .bg(rgba(theme::border()))
}

fn mute_flyout(app: &DiscordApp, guild_id: u64) -> Div {
    flyout(
        app,
        panel("guild-menu-mute-flyout", MUTE_WIDTH).children(MUTE_CHOICES.iter().enumerate().map(
            |(index, (label, seconds))| {
                let entity = app.entity.clone();
                let seconds = *seconds;
                row_base(("guild-menu-mute-choice", index), theme::text())
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        entity
                            .update(cx, |app, cx| {
                                app.close_guild_menu(cx);
                                app.set_guild_mute(guild_id, Some(seconds), cx);
                            })
                            .ok();
                    })
                    .child(div().flex_1().min_w_0().truncate().child(*label))
            },
        )),
    )
}

fn notifications_flyout(app: &DiscordApp, guild_id: u64) -> Div {
    let level = app.store.guild_notification_level(guild_id);
    let flags = app.store.guild_notification_flags(guild_id);
    let flag_row = |index: usize, label: &'static str, flag: GuildNotificationFlag| {
        let on = flags_value(flags, flag);
        toggle_row(
            app,
            ("guild-menu-notification-flag", index),
            label,
            checkbox(on),
            move |app, cx| app.set_guild_notification_flag(guild_id, flag, !on, cx),
        )
    };
    flyout(
        app,
        panel("guild-menu-notifications-flyout", NOTIFICATIONS_WIDTH)
            .children(
                NOTIFICATION_CHOICES
                    .iter()
                    .enumerate()
                    .map(|(index, (label, choice))| {
                        let choice = *choice;
                        toggle_row(
                            app,
                            ("guild-menu-notification-level", index),
                            label,
                            radio(choice == level),
                            move |app, cx| app.set_guild_notifications(guild_id, choice, cx),
                        )
                    }),
            )
            .child(divider())
            .children(
                SUPPRESSION_FLAGS
                    .iter()
                    .enumerate()
                    .map(|(index, (label, flag))| flag_row(index, label, *flag)),
            )
            .child(divider())
            .child(flag_row(
                SUPPRESSION_FLAGS.len(),
                "Mobile Push Notifications",
                GuildNotificationFlag::MobilePush,
            )),
    )
}

pub(crate) fn menu(app: &DiscordApp) -> Option<AnyElement> {
    let menu = app.menus.guild?;
    let guild_id = menu.guild_id;
    let guild = app.store.guild_by_id(guild_id)?;
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let muted = app.store.guild_muted(guild_id, now_ms);
    let mute_end = app
        .store
        .guild_mute_end(guild_id, now_ms)
        .and_then(|end_ms| chrono::DateTime::from_timestamp_millis(end_ms as i64))
        .map(|end| {
            let time = crate::settings::clock_format("%-I:%M %p", "%H:%M");
            let end = end.with_timezone(&chrono::Local);
            format!("Muted until {}", end.format(&format!("%-m/%-d, {time}")))
        });
    let level_label = NOTIFICATION_CHOICES
        .iter()
        .find(|(_, choice)| *choice == app.store.guild_notification_level(guild_id))
        .map(|(label, _)| *label);
    let close = app.entity.clone();
    let mark_read = app.entity.clone();
    let unread = guild.unread && app.live;
    let read_row = if unread {
        row_base("guild-menu-read", theme::text())
    } else {
        inert_row("guild-menu-read", theme::muted())
    }
    .on_click(move |_, _, cx| {
        cx.stop_propagation();
        if unread {
            mark_read
                .update(cx, |app, cx| {
                    app.close_guild_menu(cx);
                    app.mark_guild_read(guild_id, cx);
                })
                .ok();
        }
    })
    .child(div().flex_1().child("Mark As Read"));
    let body = panel("guild-menu", MENU_WIDTH)
        .on_mouse_down_out(move |_, _, cx| {
            close
                .update(cx, |app, cx| {
                    if !app.menus.guild.is_some_and(|menu| menu.pointer_in_flyout) {
                        app.close_guild_menu(cx);
                    }
                })
                .ok();
        })
        .child(closes_submenu(app, read_row))
        .when(app.live, |this| {
            this.child(divider())
                .child(if muted {
                    menu_row(
                        app,
                        "guild-menu-unmute",
                        None,
                        "Unmute Server",
                        mute_end.clone(),
                        move |app, _, cx| app.set_guild_mute(guild_id, None, cx),
                    )
                } else {
                    submenu_row(
                        app,
                        "guild-menu-mute",
                        "Mute Server",
                        None,
                        Submenu::Mute,
                        (menu.submenu == Submenu::Mute).then(|| mute_flyout(app, guild_id)),
                    )
                })
                .child(submenu_row(
                    app,
                    "guild-menu-notifications",
                    "Notification Settings",
                    level_label,
                    Submenu::Notifications,
                    (menu.submenu == Submenu::Notifications)
                        .then(|| notifications_flyout(app, guild_id)),
                ))
        })
        .child(divider())
        .child(menu_row(
            app,
            "guild-menu-copy-id",
            Some("id-badge"),
            "Copy Server ID",
            None,
            move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(guild_id.to_string())),
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
