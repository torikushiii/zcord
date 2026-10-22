use gpui::{prelude::*, *};
use gpui_component::tooltip::Tooltip;

use crate::{
    app::DiscordApp,
    data::{Folder, Guild, RailEntry},
    images,
    theme::{self, rpx},
};

const WIDTH: f32 = 72.;
const ICON: f32 = 48.;
const DEFAULT_FOLDER_COLOR: u32 = 0x5865F2;

pub fn render(app: &DiscordApp, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    div()
        .id("guild-rail")
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .w(rpx(WIDTH))
        .h_full()
        .flex_none()
        .pb_3()
        .overflow_y_scroll()
        .child(
            rail_item("home", false, false)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(rpx(ICON))
                        .rounded(rpx(16.))
                        .bg(rgb(theme::ELEVATED))
                        .hover(|style| style.bg(rgb(theme::BLURPLE)))
                        .child(theme::icon("discord", 28., theme::STRONG)),
                )
                .children(corner_badge(app.store.dm_mentions))
                .tooltip(|window, cx| Tooltip::new("Direct Messages").build(window, cx)),
        )
        .child(
            div()
                .w(rpx(32.))
                .h(rpx(1.))
                .flex_none()
                .bg(rgb(theme::BORDER)),
        )
        .children(app.store.rail.iter().filter_map(|entry| {
            match entry {
                RailEntry::Guild(id) => app
                    .store
                    .guild_by_id(*id)
                    .map(|guild| guild_item(app, guild, cx).into_any_element()),
                RailEntry::Folder(folder) => Some(folder_item(app, folder, cx)),
            }
        }))
        .child(action_item("add-server", "plus", "Add a Server"))
        .child(action_item("discover", "compass", "Discover"))
}

fn guild_item(app: &DiscordApp, guild: &Guild, cx: &mut Context<DiscordApp>) -> Stateful<Div> {
    let id = guild.id;
    let selected = app.selected_guild_id == id;
    let name = guild.name.clone();
    rail_item(("guild", id), selected, guild.unread)
        .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
        .on_click(cx.listener(move |this, _, window, cx| this.select_guild(id, window, cx)))
        .child(guild_icon(
            guild,
            ICON,
            if selected { 16. } else { ICON / 2. },
            cx,
        ))
        .children(corner_badge(guild.mentions))
}

fn guild_icon(guild: &Guild, size: f32, radius: f32, cx: &App) -> AnyElement {
    let image = guild
        .icon_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(ICON), cx));
    match image {
        Some(path) => img(path)
            .size(rpx(size))
            .flex_none()
            .rounded(rpx(radius))
            .group_hover("rail-item", |style| style.rounded(rpx(16.)))
            .into_any_element(),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(size))
            .rounded(rpx(radius))
            .group_hover("rail-item", |style| style.rounded(rpx(16.)))
            .bg(rgb(guild.icon_color))
            .text_size(rpx(size / 3.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(white())
            .child(guild.initials.clone())
            .into_any_element(),
    }
}

fn folder_item(app: &DiscordApp, folder: &Folder, cx: &mut Context<DiscordApp>) -> AnyElement {
    let id = folder.id;
    let open = app.is_folder_open(id);
    let color = folder.color.unwrap_or(DEFAULT_FOLDER_COLOR);
    let guilds: Vec<&Guild> = folder
        .guild_ids
        .iter()
        .filter_map(|id| app.store.guild_by_id(*id))
        .collect();
    let name: SharedString = folder.name.clone().map_or_else(
        || {
            guilds
                .iter()
                .map(|guild| guild.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
                .into()
        },
        Into::into,
    );
    let contains_selected = guilds.iter().any(|guild| guild.id == app.selected_guild_id);
    let unread = guilds.iter().any(|guild| guild.unread);
    let mentions = guilds.iter().map(|guild| guild.mentions).sum::<u32>();
    let tint = rgba((color << 8) | 0x40);

    let header = rail_item(("folder", id), !open && contains_selected, !open && unread)
        .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_folder(id, cx)))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(ICON))
                .rounded(rpx(16.))
                .when(!open, |this| this.bg(tint))
                .map(|this| {
                    if open {
                        this.child(theme::icon("folder", 24., color))
                    } else {
                        this.child(
                            div().flex().flex_wrap().gap(rpx(4.)).w(rpx(36.)).children(
                                guilds
                                    .iter()
                                    .take(4)
                                    .map(|guild| guild_icon(guild, 16., 8., cx)),
                            ),
                        )
                    }
                }),
        )
        .when(!open, |this| this.children(corner_badge(mentions)));

    if !open {
        return header.into_any_element();
    }
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .w_full()
        .pb_2()
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(rpx((WIDTH - ICON) / 2.))
                .w(rpx(ICON))
                .rounded(rpx(16.))
                .bg(tint),
        )
        .child(header)
        .children(guilds.iter().map(|guild| guild_item(app, guild, cx)))
        .into_any_element()
}

fn rail_item(id: impl Into<ElementId>, selected: bool, unread: bool) -> Stateful<Div> {
    div()
        .id(id)
        .group("rail-item")
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .w_full()
        .h(rpx(ICON))
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .left_0()
                .w(rpx(4.))
                .rounded_r(rpx(4.))
                .bg(rgb(theme::STRONG))
                .map(|pill| {
                    if selected {
                        pill.h(rpx(40.)).top(rpx(4.))
                    } else if unread {
                        pill.h(rpx(8.)).top(rpx(20.))
                    } else {
                        pill.h(rpx(0.)).top(rpx(24.))
                    }
                })
                .when(!selected, |pill| {
                    pill.group_hover("rail-item", |style| style.h(rpx(20.)).top(rpx(14.)))
                }),
        )
}

fn corner_badge(count: u32) -> Option<impl IntoElement> {
    (count > 0).then(|| {
        div()
            .absolute()
            .bottom(rpx(-4.))
            .right(rpx(8.))
            .child(theme::badge(count, theme::BG))
    })
}

fn action_item(id: &'static str, icon: &'static str, label: &'static str) -> impl IntoElement {
    rail_item(id, false, false)
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(ICON))
                .rounded_full()
                .bg(rgb(theme::ELEVATED))
                .child(theme::icon(icon, 24., theme::TEXT)),
        )
}
