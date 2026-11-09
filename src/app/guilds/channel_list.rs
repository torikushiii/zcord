mod actions;
mod header;
mod rows;
mod user_panel;

pub(crate) use user_panel::{
    USER_PANEL_HEIGHT, hide_account_tooltip, self_invisible, sharing_paused_card, user_panel,
};

use header::{banner_row, corner_cover, guild_header};
use rows::{favorites_sidebar, row_content, thread_row};

use crate::app::menu::PopupButton as _;
use gpui_component::scroll::ScrollableElement as _;
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

use gpui::{prelude::*, *};
use gpui_component::ElementExt;

use crate::{
    app::{
        DiscordApp,
        calls::voice::{self, device_menu::DeviceKind},
        direct::home,
        members::member_list,
        messages::message_body::label,
        profile,
    },
    discord::{SelfStatus, StatusDuration},
    media::animated,
    media::images::{self, ImageSpec},
    store::{Channel, ChannelKind, Guild, Status, StatusEmoji, ThreadRow, User},
    text::emoji,
    theme::{self, WithoutTextSelection, rpx},
};

pub const WIDTH: f32 = 303.;
const HEADER_HEIGHT: f32 = 48.;
const BANNER_HEIGHT: f32 = WIDTH * 9. / 21.;
const BANNER_ROW_HEIGHT: f32 = 133.;
const BANNER_HOVER_FADE: Duration = Duration::from_millis(150);
pub(crate) const FOLD: Duration = Duration::from_millis(200);

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Banner,
    ActionSpacer,
    BoostGoal {
        spaced: bool,
    },
    ServerGuide,
    Events,
    ChannelsAndRoles,
    ActionDivider,
    Category(usize),
    Channel {
        category: usize,
        channel: usize,
    },
    VoiceMember {
        category: usize,
        channel: usize,
        index: usize,
    },
    Thread {
        category: usize,
        channel: usize,
        index: usize,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccountControl {
    ActivityToggle,
    Mute,
    Deafen,
    Settings,
    InputOptions,
    OutputOptions,
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

#[derive(Default)]
struct BannerHover {
    hovered: bool,
    from: f32,
    started: Option<Instant>,
}

impl BannerHover {
    fn opacity(&self, now: Instant) -> f32 {
        let target = if self.hovered { 1. } else { 0. };
        let progress = self.started.map_or(1., |started| {
            (now.duration_since(started).as_secs_f32() / BANNER_HOVER_FADE.as_secs_f32())
                .clamp(0., 1.)
        });
        let eased = progress * progress * (3. - 2. * progress);
        self.from + (target - self.from) * eased
    }

    fn animating(&self, now: Instant) -> bool {
        self.started
            .is_some_and(|started| now.duration_since(started) < BANNER_HOVER_FADE)
    }

    fn set(&mut self, hovered: bool, now: Instant) -> bool {
        if self.hovered == hovered {
            return false;
        }
        self.from = self.opacity(now);
        self.hovered = hovered;
        self.started = Some(now);
        true
    }
}

pub(crate) struct Rows {
    state: ListState,
    pub(crate) account_bounds: profile::AccountPanelBounds,
    rows: Vec<Row>,
    guild_id: Option<u64>,
    banner: Option<ImageSpec>,
    banner_hover: BannerHover,
    collapsed: HashSet<u64>,
    folds: HashMap<u64, Fold>,
    kept: u64,
    subtitle_hovered_once: bool,
    hovered_account_control: Option<AccountControl>,
}

impl Rows {
    pub fn new() -> Self {
        Self {
            state: ListState::new(0, ListAlignment::Top, px(200.)),
            account_bounds: Rc::new(Cell::new(None)),
            rows: Vec::new(),
            guild_id: None,
            banner: None,
            banner_hover: BannerHover::default(),
            collapsed: HashSet::new(),
            folds: HashMap::new(),
            kept: 0,
            subtitle_hovered_once: false,
            hovered_account_control: None,
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
        self.sync_with_banner(guild, None, selected_channel_id, |_| 0, |_| 0, None);
    }

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
        voice_members: impl Fn(u64) -> usize,
        threads: impl Fn(u64) -> usize,
        voice_channel_id: Option<u64>,
    ) {
        let now = Instant::now();
        let mut rows = Vec::new();
        if banner.is_some() {
            rows.push(Row::Banner);
        }
        if let Some(guild) = guild {
            let links = [
                (guild.server_guide, Row::ServerGuide),
                (guild.events.is_some(), Row::Events),
                (guild.channels_and_roles, Row::ChannelsAndRoles),
            ];
            let other_actions = links.iter().any(|(shown, _)| *shown);
            if guild.boost_goal.is_some() || other_actions {
                rows.push(Row::ActionSpacer);
            }
            if guild.boost_goal.is_some() {
                rows.push(Row::BoostGoal {
                    spaced: other_actions,
                });
            }
            rows.extend(
                links
                    .into_iter()
                    .filter_map(|(shown, row)| shown.then_some(row)),
            );
            if other_actions {
                rows.push(Row::ActionDivider);
            }
        }
        for (category, group) in guild
            .into_iter()
            .flat_map(|guild| &guild.categories)
            .enumerate()
        {
            if group.id != 0 {
                rows.push(Row::Category(category));
            }
            let collapsed = self.collapsed.contains(&group.id) && !self.folding(group.id, now);
            for (channel, entry) in group.channels.iter().enumerate() {
                let joined = voice_channel_id == Some(entry.id);
                if collapsed && entry.id != selected_channel_id && !joined {
                    continue;
                }
                rows.push(Row::Channel { category, channel });
                if entry.kind == ChannelKind::Voice && (!collapsed || joined) {
                    rows.extend((0..voice_members(entry.id)).map(|index| Row::VoiceMember {
                        category,
                        channel,
                        index,
                    }));
                }
                if !collapsed || entry.id == selected_channel_id {
                    rows.extend((0..threads(entry.id)).map(|index| Row::Thread {
                        category,
                        channel,
                        index,
                    }));
                }
            }
        }
        let guild_id = guild.map(|guild| guild.id);
        if self.guild_id != guild_id || banner.is_none() {
            self.banner_hover = BannerHover::default();
        }
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

    /// GPUI's `scroll_to_reveal_item` re-anchors the scroll whenever the row's bottom-aligned
    /// position starts in the current top item, which jumps the list even for rows already in view.
    pub(crate) fn reveal_if_hidden(&self, ix: usize) {
        let viewport = self.state.viewport_bounds();
        let visible = self
            .state
            .bounds_for_item(ix)
            .is_some_and(|item| item.top() >= viewport.top() && item.bottom() <= viewport.bottom());
        if !visible {
            self.state.scroll_to_reveal_item(ix);
        }
    }

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

pub fn render(app: &DiscordApp, window: &Window, cx: &App) -> AnyElement {
    if app.favorites_view {
        return div()
            .flex()
            .flex_col()
            .w(rpx(WIDTH))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(rgba(theme::border()))
            .child(favorites_sidebar(app))
            .child(user_panel_slot(app, cx))
            .into_any_element();
    }
    if app.is_home() {
        return div()
            .flex()
            .flex_col()
            .w(rpx(WIDTH))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(rgba(theme::border()))
            .child(home::sidebar(app, cx))
            .child(user_panel_slot(app, cx))
            .into_any_element();
    }
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
        .border_color(rgba(theme::border()))
        .when(!banner, |this| {
            this.child(guild_header(app, 1., window, cx))
        })
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(
                    list(app.channel_rows.state.clone(), move |index, window, cx| {
                        let Some(app_entity) = entity.upgrade() else {
                            return div().into_any_element();
                        };
                        let hovered =
                            window.use_keyed_state(("channel-row-hover", index), cx, |_, _| false);
                        let app = app_entity.read(cx);
                        let Some(row) = app.channel_rows.rows.get(index).copied() else {
                            return div().into_any_element();
                        };
                        match row {
                            Row::ActionSpacer => {
                                return actions::spacer(app.channel_rows.banner.is_some())
                                    .into_any_element();
                            }
                            Row::BoostGoal { spaced } => return actions::boost_goal(app, spaced),
                            Row::ServerGuide | Row::Events | Row::ChannelsAndRoles => {
                                return actions::link(app, row);
                            }
                            Row::ActionDivider => return actions::divider().into_any_element(),
                            _ => {}
                        }
                        let channel = match row {
                            Row::Channel { category, channel } => app
                                .guild()
                                .and_then(|guild| guild.categories.get(category))
                                .and_then(|category| category.channels.get(channel))
                                .filter(|channel| {
                                    channel.kind.is_selectable()
                                        || channel.kind == ChannelKind::Voice
                                })
                                .map(|channel| (channel.id, channel.kind)),
                            _ => None,
                        };
                        if let Row::VoiceMember {
                            category,
                            channel,
                            index,
                        } = row
                        {
                            let guild_id = app.selected_guild_id;
                            let member = app
                                .guild()
                                .and_then(|guild| guild.categories.get(category))
                                .and_then(|category| category.channels.get(channel))
                                .and_then(|channel| {
                                    app.store
                                        .voice_members(guild_id, channel.id)
                                        .into_iter()
                                        .nth(index)
                                        .map(|member| (channel.id, member))
                                });
                            return match member {
                                Some((channel_id, member)) => {
                                    voice::member_row(app, guild_id, channel_id, &member, cx)
                                        .into_any_element()
                                }
                                None => div().into_any_element(),
                            };
                        }
                        if let Row::Thread {
                            category,
                            channel,
                            index: thread_index,
                        } = row
                        {
                            let thread = app
                                .guild()
                                .and_then(|guild| guild.categories.get(category))
                                .and_then(|category| category.channels.get(channel))
                                .and_then(|channel| {
                                    app.store
                                        .sidebar_threads(channel.id, app.selected_channel_id)
                                        .into_iter()
                                        .nth(thread_index)
                                });
                            return match thread {
                                Some(thread) => thread_row(app, &thread, index).into_any_element(),
                                None => div().into_any_element(),
                            };
                        }
                        let group = match row {
                            Row::Category(category)
                            | Row::Channel { category, .. }
                            | Row::VoiceMember { category, .. }
                            | Row::Thread { category, .. } => {
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
                            return banner_row(app.channel_rows.banner.as_ref(), cx)
                                .into_any_element();
                        }
                        let entity = app.entity.clone();
                        let hovered = channel.map(|_| hovered);
                        let over = hovered.as_ref().is_some_and(|hovered| *hovered.read(cx));
                        row_content(
                            app.guild(),
                            row,
                            app.highlighted_channel(),
                            openness,
                            app.voice.joined(&app.store),
                            Some((app, over)),
                        )
                        .id(("channel-list-row", index))
                        .when_some(hovered, |this, hovered| {
                            this.on_hover(move |over, _, cx| {
                                hovered.update(cx, |hovered, cx| {
                                    *hovered = *over;
                                    cx.notify();
                                });
                            })
                        })
                        .map(|this| match (channel, category) {
                            (Some((id, ChannelKind::Voice)), _) => {
                                this.on_click(move |_, _, cx| {
                                    entity
                                        .update(cx, |app, cx| {
                                            app.channel_rows.reveal_if_hidden(index);
                                            app.open_voice_channel(app.selected_guild_id, id, cx)
                                        })
                                        .ok();
                                })
                            }
                            (Some((id, _)), _) => this.on_click(move |_, window, cx| {
                                entity
                                    .update(cx, |app, cx| {
                                        app.channel_rows.reveal_if_hidden(index);
                                        app.select_channel(id, window, cx)
                                    })
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
                .vertical_scrollbar(&app.channel_rows.state),
        )
        .when(banner, |this| {
            this.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .child(guild_header(app, solidity, window, cx)),
            )
        })
        .child(corner_cover())
        .child(user_panel_slot(app, cx))
        .into_any_element()
}

fn user_panel_slot(app: &DiscordApp, cx: &App) -> AnyElement {
    match super::super::views::user_panel_reserve(app) {
        Some(reserve) => div().flex_none().h(reserve).into_any_element(),
        None => user_panel(app, cx).into_any_element(),
    }
}
