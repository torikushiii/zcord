use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant, SystemTime},
};

use gpui::{prelude::*, *};
use gpui_component::tooltip::Tooltip;

use crate::{
    animated,
    app::{DiscordApp, message_body::label, profile},
    emoji,
    images::{self, ImageSpec},
    store::{Channel, ChannelKind, Guild, Status, StatusEmoji, User},
    theme::{self, WithoutTextSelection, rpx},
};

pub const WIDTH: f32 = 303.;
const HEADER_HEIGHT: f32 = 48.;
/// Discord's banner is 135px tall in its 240px sidebar; keep the height and crop wider.
const BANNER_HEIGHT: f32 = 135.;
const CHANNEL_HEIGHT: f32 = 32.;
const CHANNEL_GAP: f32 = 2.;
pub(crate) const FOLD: Duration = Duration::from_millis(200);

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Banner,
    Shortcuts,
    Category(usize),
    Channel { category: usize, channel: usize },
}

struct Fold {
    started: Instant,
    from: f32,
    to: f32,
}

impl Fold {
    fn openness(&self, now: Instant) -> Option<f32> {
        let progress = now.duration_since(self.started).as_secs_f32() / FOLD.as_secs_f32();
        (progress < 1.).then(|| self.from + (self.to - self.from) * ease_out_quint()(progress))
    }
}

/// Flatten the channel hierarchy only when the server or its structure changes.
pub(crate) struct Rows {
    state: ListState,
    rows: Vec<Row>,
    guild_id: Option<u64>,
    banner: Option<ImageSpec>,
    collapsed: HashSet<u64>,
    folds: HashMap<u64, Fold>,
    kept: u64,
    account_menu: Option<Point<Pixels>>,
    account_menu_closed_at: Option<Instant>,
    subtitle_hovered_once: bool,
}

impl Rows {
    pub fn new() -> Self {
        Self {
            state: ListState::new(1, ListAlignment::Top, px(200.)),
            rows: vec![Row::Shortcuts],
            guild_id: None,
            banner: None,
            collapsed: HashSet::new(),
            folds: HashMap::new(),
            kept: 0,
            account_menu: None,
            account_menu_closed_at: None,
            subtitle_hovered_once: false,
        }
    }

    pub fn with_collapsed(collapsed: HashSet<u64>) -> Self {
        Self {
            collapsed,
            ..Self::new()
        }
    }

    #[cfg(test)]
    pub fn sync(&mut self, guild: Option<&Guild>, selected_channel_id: u64) {
        self.sync_with_banner(guild, None, selected_channel_id);
    }

    /// Returns whether the category is now collapsed.
    pub(crate) fn toggle_collapsed(&mut self, id: u64) -> bool {
        let now = Instant::now();
        let from = self.openness(id, now);
        let collapsed = !self.collapsed.remove(&id) && self.collapsed.insert(id);
        self.folds.retain(|_, fold| fold.openness(now).is_some());
        self.folds.insert(
            id,
            Fold {
                started: now,
                from,
                to: if collapsed { 0. } else { 1. },
            },
        );
        collapsed
    }

    /// 1 when the category is expanded, 0 when collapsed, in between while it folds.
    fn openness(&self, id: u64, now: Instant) -> f32 {
        self.folds
            .get(&id)
            .and_then(|fold| fold.openness(now))
            .unwrap_or(if self.collapsed.contains(&id) { 0. } else { 1. })
    }

    fn folding(&self, id: u64, now: Instant) -> bool {
        self.folds
            .get(&id)
            .is_some_and(|fold| fold.openness(now).is_some())
    }

    pub fn sync_with_banner(
        &mut self,
        guild: Option<&Guild>,
        banner: Option<ImageSpec>,
        selected_channel_id: u64,
    ) {
        let now = Instant::now();
        let mut rows = Vec::new();
        if banner.is_some() {
            rows.push(Row::Banner);
        }
        rows.push(Row::Shortcuts);
        for (category, group) in guild
            .into_iter()
            .flat_map(|guild| &guild.categories)
            .enumerate()
        {
            rows.push(Row::Category(category));
            // Collapsing channels stay listed until they have folded away.
            let collapsed = self.collapsed.contains(&group.id) && !self.folding(group.id, now);
            rows.extend(
                (0..group.channels.len())
                    // Discord keeps the selected channel visible under a collapsed category.
                    .filter(|&channel| {
                        !collapsed || group.channels[channel].id == selected_channel_id
                    })
                    .map(|channel| Row::Channel { category, channel }),
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
        self.kept = selected_channel_id;
        self.banner = banner;
    }

    /// How solid the header should be: 0 with the banner fully shown, rising to 1 as the
    /// banner scrolls up under the header. None when the server has no banner.
    fn header_solidity(&self, rem_size: Pixels) -> Option<f32> {
        self.banner.as_ref()?;
        let top = self.state.logical_scroll_top();
        if top.item_ix > 0 {
            return Some(1.);
        }
        let range = rpx(BANNER_HEIGHT - HEADER_HEIGHT).to_pixels(rem_size);
        Some((top.offset_in_item / range).clamp(0., 1.))
    }
}

pub fn render(app: &DiscordApp, window: &Window, cx: &App) -> impl IntoElement + use<> {
    let guild = app.guild();
    let entity = app.entity.clone();
    let banner = app.channel_rows.banner.is_some();
    let solidity = app
        .channel_rows
        .header_solidity(window.rem_size())
        .unwrap_or(1.);
    div()
        .flex()
        .flex_col()
        .relative()
        .w(rpx(WIDTH))
        .h_full()
        .flex_none()
        .border_r_1()
        .border_color(rgb(theme::BORDER))
        .when(!banner, |this| this.child(guild_header(guild, 1.)))
        .child(
            list(app.channel_rows.state.clone(), move |index, window, cx| {
                let Some(app_entity) = entity.upgrade() else {
                    return div().into_any_element();
                };
                let app = app_entity.read(cx);
                let Some(row) = app.channel_rows.rows.get(index).copied() else {
                    return div().into_any_element();
                };
                let channel = match row {
                    Row::Channel { category, channel } => app
                        .guild()
                        .and_then(|guild| guild.categories.get(category))
                        .and_then(|category| category.channels.get(channel))
                        .filter(|channel| channel.kind.is_selectable())
                        .map(|channel| channel.id),
                    _ => None,
                };
                let group = match row {
                    Row::Category(category) | Row::Channel { category, .. } => {
                        app.guild().and_then(|guild| guild.categories.get(category))
                    }
                    _ => None,
                };
                let rows = &app.channel_rows;
                let now = Instant::now();
                if group.is_some_and(|group| rows.folding(group.id, now)) {
                    window.request_animation_frame();
                }
                let openness = group.map_or(1., |group| match row {
                    Row::Channel { channel, .. }
                        if group
                            .channels
                            .get(channel)
                            .is_some_and(|channel| channel.id == rows.kept) =>
                    {
                        1.
                    }
                    _ => rows.openness(group.id, now),
                });
                let category = match row {
                    Row::Category(_) => group.map(|group| group.id).filter(|id| *id != 0),
                    _ => None,
                };
                if row == Row::Banner {
                    return banner_row(app.channel_rows.banner.as_ref(), cx).into_any_element();
                }
                let entity = app.entity.clone();
                row_content(app.guild(), row, app.selected_channel_id, openness)
                    .id(("channel-list-row", index))
                    .map(|this| match (channel, category) {
                        (Some(id), _) => this.on_click(move |_, window, cx| {
                            entity
                                .update(cx, |app, cx| app.select_channel(id, window, cx))
                                .ok();
                        }),
                        (None, Some(id)) => this.on_click(move |_, _, cx| {
                            entity
                                .update(cx, |app, cx| app.toggle_category(id, cx))
                                .ok();
                        }),
                        (None, None) => this,
                    })
                    .into_any_element()
            })
            .flex_1()
            .min_h_0()
            .w_full()
            .when(!banner, |this| this.py_2())
            .when(banner, |this| this.pb_2()),
        )
        .when(banner, |this| {
            this.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .child(guild_header(guild, solidity)),
            )
        })
        .child(user_panel(app, cx))
}

fn guild_header(guild: Option<&Guild>, solidity: f32) -> impl IntoElement + use<> {
    let solid = solidity >= 1.;
    let fade = |color: u32, alpha: f32| Hsla::from(rgb(color)).opacity(alpha);
    div()
        .id("guild-header")
        .relative()
        .rounded_tl(rpx(8.))
        .flex()
        .items_center()
        .gap_1()
        .h(rpx(HEADER_HEIGHT))
        .flex_none()
        .pl_4()
        .pr_2()
        .cursor_pointer()
        .bg(fade(theme::BG, solidity))
        .border_b_1()
        .border_color(fade(theme::BORDER, solidity))
        .hover(move |style| {
            if solid {
                style.bg(rgb(theme::HOVER))
            } else {
                style.bg(rgba(0x202226A6))
            }
        })
        .when(!solid, |this| {
            this.child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(1. - solidity)
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(rgba(0x00000099), 0.),
                        linear_color_stop(rgba(0x00000000), 1.),
                    )),
            )
        })
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
        .child(theme::icon_button(
            "invite",
            "user-plus",
            18.,
            if solid { theme::MUTED } else { theme::STRONG },
        ))
}

fn banner_row(spec: Option<&ImageSpec>, cx: &App) -> impl IntoElement + use<> {
    let pixels = theme::image_px(WIDTH);
    let fill = |image: Img| {
        image
            .size_full()
            .rounded_tl(rpx(8.))
            .object_fit(ObjectFit::Cover)
    };
    let image = spec.and_then(|spec| {
        if let Some(image) = spec
            .animated()
            .and_then(|animated| animated::resolve(&animated, pixels, cx))
        {
            return Some(
                animated::image(image)
                    .size_full()
                    .rounded_tl(rpx(8.))
                    .object_fit(ObjectFit::Cover)
                    .id("guild-banner-animated")
                    .into_any_element(),
            );
        }
        images::resolve(spec, pixels, cx).map(|path| fill(img(path)).into_any_element())
    });
    div()
        .w_full()
        .h(rpx(BANNER_HEIGHT))
        .mb_2()
        .bg(rgb(theme::ELEVATED))
        .rounded_tl(rpx(8.))
        .children(image)
}

fn row_content(guild: Option<&Guild>, row: Row, selected: u64, openness: f32) -> Div {
    let content = match row {
        Row::Banner => div().into_any_element(),
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
                        .child(theme::icon("chevron-down", 12., theme::MUTED).when(
                            openness < 1.,
                            |icon| {
                                icon.with_transformation(Transformation::rotate(radians(
                                    -std::f32::consts::FRAC_PI_2 * (1. - openness),
                                )))
                            },
                        ))
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
            Some(channel) if openness < 1. => div()
                .h(rpx((CHANNEL_HEIGHT + CHANNEL_GAP) * openness))
                .overflow_hidden()
                .opacity(openness)
                .child(
                    div()
                        .flex_none()
                        .h(rpx(CHANNEL_HEIGHT + CHANNEL_GAP))
                        .child(channel_row(channel, selected == channel.id)),
                )
                .into_any_element(),
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
    let mentions = channel.mentions;
    let new_posts = channel.new_posts;
    let unread = (channel.unread || mentions > 0 || new_posts > 0) && !selected;
    let icon = match channel.kind {
        ChannelKind::Text => "hash",
        ChannelKind::Announcement => "megaphone",
        ChannelKind::Forum => "messages-square",
        ChannelKind::Voice => "volume-2",
    };
    let badge = (new_posts > 0)
        .then(|| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(rpx(16.))
                .min_w(rpx(16.))
                .px(rpx(4.))
                .rounded_full()
                .bg(rgb(theme::BLURPLE))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::BOLD)
                .text_color(white())
                .child(format!("{new_posts} NEW"))
        })
        .or_else(|| {
            (mentions > 0).then(|| {
                let count = if mentions >= 1000 {
                    format!("{}K+", mentions / 1000)
                } else {
                    mentions.to_string()
                };
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(rpx(16.))
                    .min_w(rpx(16.))
                    .px(rpx(4.))
                    .rounded_full()
                    .bg(rgb(theme::RED))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(white())
                    .child(count)
            })
        });
    div()
        .id(("channel", id))
        .relative()
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(CHANNEL_HEIGHT))
        .mt(rpx(CHANNEL_GAP))
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
            icon,
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
        .children(badge)
        .when(selected, |this| {
            this.child(theme::icon("user-plus", 14., theme::MUTED))
                .child(theme::icon("settings", 14., theme::MUTED))
        })
}

const SUBTITLE_HEIGHT: f32 = 16.;

/// The custom status (or the presence when there is none), which slides up to reveal the
/// username while the panel is hovered, like Discord's.
fn account_subtitle(app: &DiscordApp, user: &User, cx: &App) -> AnyElement {
    let line = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(4.))
            .min_w_0()
            .h(rpx(SUBTITLE_HEIGHT))
    };
    let status = user
        .custom_status
        .as_ref()
        .filter(|status| !status.is_expired(SystemTime::now()));
    let status_line = match status {
        None => line().child(div().truncate().child(status_label(user.status))),
        Some(status) => {
            let emoji = status.emoji.as_ref().and_then(|emoji| {
                let source: ImageSource = match emoji {
                    StatusEmoji::Unicode(sequence) => emoji::image(sequence)?.into(),
                    StatusEmoji::Custom { id, .. } => images::resolve(
                        &ImageSpec::Emoji {
                            id: *id,
                            animated: false,
                        },
                        theme::image_px(SUBTITLE_HEIGHT),
                        cx,
                    )?
                    .into(),
                };
                Some(img(source).size(rpx(SUBTITLE_HEIGHT)).flex_none())
            });
            line()
                .children(emoji)
                .children(status.text.as_ref().map(|text| {
                    div()
                        .min_w_0()
                        .truncate()
                        .child(label(("self-status", user.id), text))
                }))
        }
    };
    let hovered = app.hovered_account && !user.username.is_empty();
    let lines = div()
        .absolute()
        .left_0()
        .right_0()
        .flex()
        .flex_col()
        .child(status_line)
        .child(line().child(div().truncate().child(user.username.clone())));
    let lines = if app.channel_rows.subtitle_hovered_once {
        let animation = Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint());
        lines
            .with_animation(
                ("account-subtitle", u64::from(hovered)),
                animation,
                move |this, delta| {
                    let shown = if hovered { delta } else { 1. - delta };
                    this.top(rpx(-SUBTITLE_HEIGHT * shown))
                },
            )
            .into_any_element()
    } else {
        lines.top_0().into_any_element()
    };
    div()
        .relative()
        .h(rpx(SUBTITLE_HEIGHT))
        .min_w_0()
        .overflow_hidden()
        .text_size(rpx(12.))
        .line_height(rpx(SUBTITLE_HEIGHT))
        .text_color(rgb(theme::MUTED))
        .child(lines)
        .into_any_element()
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Online => "Online",
        Status::Idle => "Idle",
        Status::DoNotDisturb => "Do Not Disturb",
        Status::Offline => "Invisible",
    }
}

fn user_panel(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
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
                .on_hover({
                    let entity = app.entity.clone();
                    move |hovered: &bool, _, cx| {
                        entity
                            .update(cx, |app, cx| {
                                app.hovered_account = *hovered;
                                // Until the first hover the subtitle sits still, so it doesn't
                                // play the slide-back animation when the panel first appears.
                                app.channel_rows.subtitle_hovered_once = true;
                                cx.notify();
                            })
                            .ok();
                    }
                })
                .without_text_selection()
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
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::STRONG))
                                .child(label(("self-name", user.id), &user.name)),
                        )
                        .child(account_subtitle(app, user, cx)),
                )
        }))
        .child(theme::icon_button("mic", "mic", 20., theme::TEXT))
        .child(theme::icon_button("deafen", "headphones", 20., theme::TEXT))
        .child(
            theme::icon_button("user-settings", "settings", 20., theme::TEXT)
                .tooltip(|window, cx| Tooltip::new("User Settings").build(window, cx))
                .on_mouse_down(MouseButton::Left, {
                    let entity = app.entity.clone();
                    move |event, _, cx| {
                        cx.stop_propagation();
                        let position = event.position;
                        entity
                            .update(cx, |app, cx| {
                                let rows = &mut app.channel_rows;
                                // The press that closed the menu via mouse-down-outside must
                                // not reopen it here.
                                let just_closed = rows
                                    .account_menu_closed_at
                                    .is_some_and(|at| at.elapsed() < Duration::from_millis(150));
                                if rows.account_menu.is_none() && !just_closed {
                                    rows.account_menu = Some(position);
                                } else {
                                    rows.account_menu = None;
                                }
                                cx.notify();
                            })
                            .ok();
                    }
                }),
        )
}

/// Settings menu above the user panel's gear button. Rendered from the overlay view because
/// deferred popups can't live inside the cached channel list view.
pub(crate) fn account_menu(app: &DiscordApp) -> Option<AnyElement> {
    let position = app.channel_rows.account_menu?;
    let close = app.entity.clone();
    let log_out = app.entity.clone();
    let item = div()
        .id("log-out")
        .flex()
        .items_center()
        .justify_between()
        .h(rpx(32.))
        .px(rpx(8.))
        .rounded(rpx(4.))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::RED))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(theme::RED)).text_color(white()))
        .child("Log Out")
        .on_click(move |_, window, cx| {
            log_out
                .update(cx, |app, cx| {
                    app.channel_rows.account_menu = None;
                    app.log_out(window, cx);
                })
                .ok();
        });
    Some(
        deferred(
            anchored()
                .position(position - point(px(0.), px(12.)))
                .anchor(Anchor::BottomRight)
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id("account-menu")
                        .occlude()
                        .w(rpx(188.))
                        .p(rpx(6.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::ELEVATED))
                        .border_1()
                        .border_color(rgb(theme::BORDER))
                        .shadow_lg()
                        .on_mouse_down_out(move |_, _, cx| {
                            close
                                .update(cx, |app, cx| {
                                    app.channel_rows.account_menu = None;
                                    app.channel_rows.account_menu_closed_at = Some(Instant::now());
                                    cx.notify();
                                })
                                .ok();
                        })
                        .child(item),
                ),
        )
        .with_priority(2)
        .into_any_element(),
    )
}
