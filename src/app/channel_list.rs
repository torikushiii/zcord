use gpui::{prelude::*, *};
use gpui_component::tooltip::Tooltip;

use crate::{
    app::{DiscordApp, message_body::label, profile},
    store::{Channel, ChannelKind, Guild},
    theme::{self, rpx},
};

pub const WIDTH: f32 = 303.;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Shortcuts,
    Category(usize),
    Channel { category: usize, channel: usize },
}

/// Flatten the channel hierarchy only when the server or its structure changes.
pub(crate) struct Rows {
    state: ListState,
    rows: Vec<Row>,
    guild_id: Option<u64>,
}

impl Rows {
    pub fn new() -> Self {
        Self {
            state: ListState::new(1, ListAlignment::Top, px(200.)),
            rows: vec![Row::Shortcuts],
            guild_id: None,
        }
    }

    pub fn sync(&mut self, guild: Option<&Guild>) {
        let mut rows = vec![Row::Shortcuts];
        for (category, group) in guild
            .into_iter()
            .flat_map(|guild| &guild.categories)
            .enumerate()
        {
            rows.push(Row::Category(category));
            rows.extend(
                (0..group.channels.len()).map(|channel| Row::Channel { category, channel }),
            );
        }
        let guild_id = guild.map(|guild| guild.id);
        if self.guild_id != guild_id {
            self.state.reset(rows.len());
        } else if self.rows != rows {
            let prefix = self
                .rows
                .iter()
                .zip(&rows)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = self.rows[prefix..]
                .iter()
                .rev()
                .zip(rows[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            self.state.splice(
                prefix..self.rows.len() - suffix,
                rows.len() - prefix - suffix,
            );
        }
        self.guild_id = guild_id;
        self.rows = rows;
    }
}

pub fn render(app: &DiscordApp, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    let guild = app.guild();
    let entity = cx.entity();
    div()
        .flex()
        .flex_col()
        .w(rpx(WIDTH))
        .h_full()
        .flex_none()
        .border_r_1()
        .border_color(rgb(theme::BORDER))
        .child(
            div()
                .id("guild-header")
                .flex()
                .items_center()
                .gap_1()
                .h(rpx(48.))
                .flex_none()
                .pl_4()
                .pr_2()
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(theme::HOVER)))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::STRONG))
                        .child(label(
                            ("guild-header", guild.map_or(0, |guild| guild.id)),
                            guild.map_or("No server", |guild| &guild.name),
                        )),
                )
                .child(theme::icon("chevron-down", 16., theme::STRONG))
                .child(div().flex_1())
                .child(theme::icon_button("invite", "user-plus", 18., theme::MUTED)),
        )
        .child(
            list(app.channel_rows.state.clone(), move |index, _, cx| {
                let app = entity.read(cx);
                let Some(row) = app.channel_rows.rows.get(index).copied() else {
                    return div().into_any_element();
                };
                let channel = match row {
                    Row::Channel { category, channel } => app
                        .guild()
                        .and_then(|guild| guild.categories.get(category))
                        .and_then(|category| category.channels.get(channel))
                        .filter(|channel| channel.kind == ChannelKind::Text)
                        .map(|channel| channel.id),
                    _ => None,
                };
                let entity = app.entity.clone();
                row_content(app.guild(), row, app.selected_channel_id)
                    .id(("channel-list-row", index))
                    .map(|this| match channel {
                        Some(id) => this.on_click(move |_, window, cx| {
                            entity
                                .update(cx, |app, cx| app.select_channel(id, window, cx))
                                .ok();
                        }),
                        None => this,
                    })
                    .into_any_element()
            })
            .flex_1()
            .min_h_0()
            .w_full()
            .py_2(),
        )
        .child(user_panel(app, cx))
}

fn row_content(guild: Option<&Guild>, row: Row, selected: u64) -> Div {
    let content = match row {
        Row::Shortcuts => div()
            .child(shortcut_row("events", "calendar", "Events"))
            .child(shortcut_row("boosts", "gem", "Server Boosts"))
            .child(div().mx_2().my_2().h(rpx(1.)).bg(rgb(theme::BORDER)))
            .into_any_element(),
        Row::Category(index) => match guild.and_then(|guild| guild.categories.get(index)) {
            Some(category) => div()
                .pt_4()
                .child(
                    div()
                        .id(("category", category.id))
                        .flex()
                        .items_center()
                        .gap(rpx(2.))
                        .h(rpx(24.))
                        .px_2()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::MUTED))
                        .cursor_pointer()
                        .hover(|style| style.text_color(rgb(theme::TEXT)))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .child(label(("category", category.id), &category.name)),
                        )
                        .child(theme::icon("chevron-down", 12., theme::MUTED))
                        .child(div().flex_1())
                        .child(theme::icon("plus", 16., theme::MUTED)),
                )
                .into_any_element(),
            None => div().into_any_element(),
        },
        Row::Channel { category, channel } => match guild
            .and_then(|guild| guild.categories.get(category))
            .and_then(|category| category.channels.get(channel))
        {
            Some(channel) => channel_row(channel, selected == channel.id).into_any_element(),
            None => div().into_any_element(),
        },
    };
    div().w_full().px_2().child(content)
}

fn shortcut_row(id: &'static str, icon: &str, label: &'static str) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(32.))
        .px_2()
        .rounded(rpx(6.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::MUTED))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(theme::HOVER)).text_color(rgb(theme::TEXT)))
        .child(theme::icon(icon, 20., theme::MUTED))
        .child(label)
}

fn channel_row(channel: &Channel, selected: bool) -> impl IntoElement + use<> {
    let id = channel.id;
    let text = channel.kind == ChannelKind::Text;
    let unread = channel.unread && !selected;
    div()
        .id(("channel", id))
        .relative()
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(32.))
        .mt(rpx(2.))
        .px_2()
        .rounded(rpx(6.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .text_color(rgb(if selected || unread {
            theme::STRONG
        } else {
            theme::MUTED
        }))
        .when(unread, |this| this.font_weight(FontWeight::SEMIBOLD))
        .when(selected, |this| this.bg(rgb(theme::SELECTED)))
        .when(!selected, |this| {
            this.hover(|style| style.bg(rgb(theme::HOVER)).text_color(rgb(theme::TEXT)))
        })
        .when(unread, |this| {
            this.child(
                div()
                    .absolute()
                    .left(rpx(-8.))
                    .top(rpx(12.))
                    .w(rpx(4.))
                    .h(rpx(8.))
                    .rounded_r(rpx(4.))
                    .bg(rgb(theme::STRONG)),
            )
        })
        .child(theme::icon(
            if text { "hash" } else { "volume-2" },
            20.,
            if selected || unread {
                theme::STRONG
            } else {
                theme::FAINT
            },
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(label(("channel-name", id), &channel.name)),
        )
        .when(selected, |this| {
            this.child(theme::icon("user-plus", 14., theme::MUTED))
                .child(theme::icon("settings", 14., theme::MUTED))
        })
}

fn user_panel(app: &DiscordApp, cx: &mut Context<DiscordApp>) -> impl IntoElement + use<> {
    let user = app.store.user(app.store.current_user_id);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .h(rpx(58.))
        .m_2()
        .px(rpx(6.))
        .rounded(rpx(8.))
        .bg(rgb(theme::ELEVATED))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .children(user.map(|user| {
            div()
                .id("account")
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_2()
                .p(rpx(2.))
                .rounded(rpx(6.))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(theme::SELECTED)))
                .on_hover(cx.listener(|app, hovered: &bool, _, cx| {
                    app.hovered_account = *hovered;
                    cx.notify();
                }))
                .on_click(profile::opener(app, user.id, Anchor::BottomLeft))
                .child(theme::user_avatar(
                    Some(user),
                    user.id,
                    36.,
                    Some((user.status, theme::ELEVATED)),
                    app.hovered_account,
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::STRONG))
                                .child(label(("self-name", user.id), &user.name)),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(12.))
                                .text_color(rgb(theme::MUTED))
                                .child(label(
                                    ("self-activity", user.id),
                                    user.activity.as_deref().unwrap_or("Online"),
                                )),
                        ),
                )
        }))
        .child(theme::icon_button("mic", "mic", 20., theme::TEXT))
        .child(theme::icon_button("deafen", "headphones", 20., theme::TEXT))
        .child(
            theme::icon_button("user-settings", "settings", 20., theme::TEXT)
                .tooltip(|window, cx| Tooltip::new("Log out").build(window, cx))
                .on_click(cx.listener(|this, _, window, cx| this.log_out(window, cx))),
        )
}
