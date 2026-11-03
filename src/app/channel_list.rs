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
    app::{DiscordApp, home, member_list, message_body::label, profile, voice},
    discord::{SelfStatus, StatusDuration},
    media::animated,
    media::images::{self, ImageSpec},
    store::{Channel, ChannelKind, Guild, Status, StatusEmoji, ThreadRow, User},
    text::emoji,
    theme::{self, WithoutTextSelection, rpx},
};

pub const WIDTH: f32 = 303.;
const HEADER_HEIGHT: f32 = 48.;
/// Discord's banner is 135px tall in its 240px sidebar; keep the height and crop wider.
const BANNER_HEIGHT: f32 = 135.;
const BANNER_HOVER_FADE: Duration = Duration::from_millis(150);
const BANNER_BLUR_SOURCE: u32 = 128;
const BANNER_BLUR_RADIUS: f32 = 30.;
const CURRENT_GAME_ICON: f32 = 32.;
const SHARING_CARD_WIDTH: f32 = 240.;
const SHARING_CARD_ANCHOR: f32 = 12.;
const CHANNEL_HEIGHT: f32 = 32.;
const CHANNEL_GAP: f32 = 2.;
pub(crate) const FOLD: Duration = Duration::from_millis(200);

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Banner,
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
    Mute,
    Deafen,
    Settings,
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
        // Reverse an interrupted fade from its current opacity, not an endpoint.
        self.from = self.opacity(now);
        self.hovered = hovered;
        self.started = Some(now);
        true
    }
}

/// Flatten the channel hierarchy only when the server or its structure changes.
pub(crate) struct Rows {
    state: ListState,
    /// Live window-space bounds of the bottom account strip, shared with its popout.
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
        voice_members: impl Fn(u64) -> usize,
        threads: impl Fn(u64) -> usize,
        voice_channel_id: Option<u64>,
    ) {
        let now = Instant::now();
        let mut rows = Vec::new();
        if banner.is_some() {
            rows.push(Row::Banner);
        }
        for (category, group) in guild
            .into_iter()
            .flat_map(|guild| &guild.categories)
            .enumerate()
        {
            // Channels outside any category sit in group 0, which Discord shows without a header.
            if group.id != 0 {
                rows.push(Row::Category(category));
            }
            // Collapsing channels stay listed until they have folded away.
            let collapsed = self.collapsed.contains(&group.id) && !self.folding(group.id, now);
            for (channel, entry) in group.channels.iter().enumerate() {
                // Discord keeps the selected and the joined voice channel visible under a
                // collapsed category.
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
        let visible = self.state.bounds_for_item(ix).is_some_and(|item| {
            item.top() >= viewport.top() && item.bottom() <= viewport.bottom()
        });
        if !visible {
            self.state.scroll_to_reveal_item(ix);
        }
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

pub fn render(app: &DiscordApp, window: &Window, cx: &App) -> AnyElement {
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
            .child(user_panel(app, cx))
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
                        let app = app_entity.read(cx);
                        let Some(row) = app.channel_rows.rows.get(index).copied() else {
                            return div().into_any_element();
                        };
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
                        row_content(
                            app.guild(),
                            row,
                            app.highlighted_channel(),
                            openness,
                            app.voice.joined(&app.store),
                        )
                        .id(("channel-list-row", index))
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
        .child(user_panel(app, cx))
        .into_any_element()
}

fn banner_hover_listener(app: &DiscordApp) -> impl Fn(&bool, &mut Window, &mut App) + use<> {
    let entity = app.entity.clone();
    let guild_id = app.selected_guild_id;
    move |hovered, _, cx| {
        entity
            .update(cx, |app, cx| {
                if app.selected_guild_id != guild_id {
                    return;
                }
                if app.channel_rows.banner_hover.set(*hovered, Instant::now()) {
                    cx.notify();
                }
            })
            .ok();
    }
}

fn guild_header(
    app: &DiscordApp,
    solidity: f32,
    window: &Window,
    cx: &App,
) -> impl IntoElement + use<> {
    let guild = app.guild();
    let solid = solidity >= 1.;
    let now = Instant::now();
    let glass = app.channel_rows.banner_hover.opacity(now);
    if !solid && app.channel_rows.banner_hover.animating(now) {
        window.request_animation_frame();
    }
    let hover: Hsla = if solid {
        rgba(theme::hover()).into()
    } else {
        rgba(0xFFFFFF14).into()
    };
    div()
        .id(("guild-header", guild.map_or(0, |guild| guild.id)))
        .relative()
        .rounded_tl(rpx(8.))
        .flex()
        .items_center()
        .gap_1()
        .h(rpx(HEADER_HEIGHT))
        .flex_none()
        .px_2()
        .bg(rgb(theme::base_lower()).opacity(solidity))
        .border_b_1()
        .border_color(rgba(theme::border()).opacity(solidity))
        .when(app.channel_rows.banner.is_some(), |this| {
            this.on_hover(banner_hover_listener(app))
        })
        .when(!solid, |this| {
            this.children(
                app.channel_rows
                    .banner
                    .as_ref()
                    .map(|spec| banner_glass(spec, solidity, glass, cx)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded_tl(rpx(8.))
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
                .id("guild-dropdown")
                .relative()
                .flex()
                .items_center()
                .gap_1()
                .h(rpx(32.))
                .min_w_0()
                .px_2()
                .rounded(rpx(8.))
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child(label(
                            ("guild-header", guild.map_or(0, |guild| guild.id)),
                            guild.map_or("No server", |guild| &guild.name),
                        )),
                )
                .child(theme::icon("chevron-down", 16., theme::strong())),
        )
        .child(div().flex_1())
        .child(
            theme::icon_button_with_hover(
                "invite",
                "user-plus",
                18.,
                if solid {
                    theme::muted()
                } else {
                    theme::strong()
                },
                hover,
            )
            .size(rpx(32.))
            .rounded(rpx(8.)),
        )
}

/// Content masks are rectangular, so rows scrolled under the header would show through the
/// panel's rounded corner. This repaints the corner as the panel draws it: window background,
/// then the 1px border, then the panel background down to the header's own curve.
fn corner_cover() -> impl IntoElement {
    const RADIUS: f32 = 8.;
    canvas(
        |_, _, _| {},
        |bounds, _, window, _| {
            let radius = rpx(RADIUS).to_pixels(window.rem_size());
            let border = px(1.);
            let outside = bounds.origin;
            let inside = outside + point(border, border);
            let outer = outside + point(radius, radius);
            let layers = [
                (inside + point(radius, radius), radius, theme::base_lower()),
                (
                    outer,
                    radius - border,
                    theme::composite(theme::border(), theme::base_lower()),
                ),
                (outer, radius, theme::base_lowest()),
            ];
            for (center, radius, color) in layers {
                let mut path = PathBuilder::fill();
                path.move_to(outside);
                path.line_to(point(center.x, outside.y));
                for step in 0..=12 {
                    let angle = (3. - step as f32 / 12.) * std::f32::consts::FRAC_PI_2;
                    path.line_to(center + point(radius * angle.cos(), radius * angle.sin()));
                }
                path.line_to(point(outside.x, center.y));
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            }
        },
    )
    .absolute()
    // Starts at the outer edge of the panel's 1px border.
    .top(px(-1.))
    .left(px(-1.))
    .size(rpx(RADIUS + 1.))
}

fn banner_glass(spec: &ImageSpec, solidity: f32, opacity: f32, cx: &App) -> Div {
    let sigma = BANNER_BLUR_RADIUS * BANNER_BLUR_SOURCE as f32 / WIDTH;
    let image = images::blurred_with(spec, BANNER_BLUR_SOURCE, sigma, 1., cx);
    div()
        .absolute()
        .inset_0()
        .opacity(opacity * (1. - solidity))
        .children(image.map(|image| {
            canvas(
                move |_, _, _| image,
                move |bounds, image, window, _| {
                    if bounds.is_empty() {
                        return;
                    }
                    // Fit the full banner before cropping to the fixed, rounded header.
                    let mut banner = bounds;
                    banner.origin.y -= rpx(solidity * (BANNER_HEIGHT - HEADER_HEIGHT))
                        .to_pixels(window.rem_size());
                    banner.size.height = rpx(BANNER_HEIGHT).to_pixels(window.rem_size());
                    let fitted = ObjectFit::Cover.get_bounds(banner, image.size(0));
                    let corners = Corners {
                        top_left: rpx(8.).to_pixels(window.rem_size()),
                        ..Default::default()
                    };
                    if let Err(error) =
                        window.paint_image_fitted(bounds, fitted, corners, image, 0, false)
                    {
                        log::warn!("Could not paint guild banner glass: {error}");
                    }
                },
            )
            .size_full()
        }))
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
        .bg(rgb(theme::elevated()))
        .rounded_tl(rpx(8.))
        .children(image)
}

fn row_content(
    guild: Option<&Guild>,
    row: Row,
    selected: u64,
    openness: f32,
    voice: Option<voice::Joined>,
) -> Div {
    let content = match row {
        Row::Banner | Row::VoiceMember { .. } | Row::Thread { .. } => div().into_any_element(),
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
                        .text_color(rgb(theme::muted()))
                        .cursor_pointer()
                        .hover(|style| style.text_color(rgb(theme::text())))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .child(label(("category", category.id), &category.name)),
                        )
                        .child(theme::icon("chevron-down", 12., theme::muted()).when(
                            openness < 1.,
                            |icon| {
                                icon.with_transformation(Transformation::rotate(radians(
                                    -std::f32::consts::FRAC_PI_2 * (1. - openness),
                                )))
                            },
                        ))
                        .child(div().flex_1())
                        .child(theme::icon("plus", 16., theme::muted())),
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
                        .child(channel_row(channel, selected == channel.id, voice)),
                )
                .into_any_element(),
            Some(channel) => channel_row(channel, selected == channel.id, voice).into_any_element(),
            None => div().into_any_element(),
        },
    };
    div().w_full().px_2().child(content)
}

fn thread_row(app: &DiscordApp, thread: &ThreadRow, row: usize) -> impl IntoElement + use<> {
    let id = thread.id;
    let selected = app.selected_channel_id == id;
    let mentions = app.store.thread_mentions(id);
    let unread = (app.store.thread_unread(id) || mentions > 0) && !selected;
    let entity = app.entity.clone();
    div().w_full().pl(rpx(36.)).pr_2().child(
        div()
            .id(("thread-row", id))
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .h(rpx(CHANNEL_HEIGHT))
            .mb(rpx(CHANNEL_GAP))
            .px_2()
            .rounded(rpx(6.))
            .cursor_pointer()
            .text_size(rpx(14.))
            .font_weight(if unread {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            })
            .text_color(rgb(if selected || unread {
                theme::strong()
            } else {
                theme::muted()
            }))
            .when(selected, |this| this.bg(rgba(theme::selected())))
            .when(!selected, |this| {
                this.hover(|style| style.bg(rgba(theme::hover())))
            })
            .on_click(move |_, window, cx| {
                entity
                    .update(cx, |app, cx| {
                        app.channel_rows.reveal_if_hidden(row);
                        app.select_channel(id, window, cx)
                    })
                    .ok();
            })
            .child(
                div()
                    .absolute()
                    .left(rpx(-10.))
                    .top(rpx(-CHANNEL_GAP))
                    .w(rpx(10.))
                    .h(rpx(CHANNEL_HEIGHT / 2. + CHANNEL_GAP))
                    .border_l_1()
                    .border_b_1()
                    .rounded_bl(rpx(6.))
                    .border_color(rgba(theme::border())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(thread.name.clone()),
            )
            .when(mentions > 0, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .h(rpx(16.))
                        .min_w(rpx(16.))
                        .px(rpx(4.))
                        .rounded_full()
                        .bg(rgb(theme::NOTIFICATION))
                        .text_size(rpx(12.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(white())
                        .child(mentions.to_string()),
                )
            }),
    )
}

fn channel_row(
    channel: &Channel,
    selected: bool,
    voice: Option<voice::Joined>,
) -> impl IntoElement + use<> {
    let id = channel.id;
    let joined = voice.filter(|voice| voice.channel_id == id);
    let mentions = channel.mentions;
    let new_posts = channel.new_posts;
    let unread = (channel.unread || mentions > 0 || new_posts > 0) && !selected;
    let (icon, lock_built_in) = match (channel.kind, channel.private) {
        (ChannelKind::Text, true) => ("hash-lock", true),
        (ChannelKind::Text, false) => ("hash", false),
        (ChannelKind::Voice, true) => ("volume-2-lock", true),
        (ChannelKind::Voice, false) => ("volume-2", false),
        (ChannelKind::Announcement, _) => ("megaphone", false),
        (ChannelKind::Forum, _) => ("messages-square", false),
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
                    .bg(rgb(theme::NOTIFICATION))
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
        .text_color(rgb(if selected || unread || joined.is_some() {
            theme::strong()
        } else {
            theme::muted()
        }))
        .when(unread || joined.is_some(), |this| {
            this.font_weight(FontWeight::SEMIBOLD)
        })
        .when(selected, |this| this.bg(rgba(theme::selected())))
        .when(!selected, |this| {
            this.hover(|style| {
                style
                    .bg(rgba(theme::hover()))
                    .text_color(rgb(theme::text()))
            })
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
                    .bg(rgb(theme::strong())),
            )
        })
        .child({
            let color = if joined.is_some() {
                theme::GREEN
            } else if selected || unread {
                theme::strong()
            } else {
                theme::faint()
            };
            div()
                .relative()
                .flex_none()
                .size(rpx(20.))
                .child(theme::icon(icon, 20., color))
                .when(channel.private && !lock_built_in, |this| {
                    this.child(
                        div()
                            .absolute()
                            .top(rpx(-1.))
                            .right(rpx(-2.))
                            .child(theme::icon("lock", 10., color)),
                    )
                })
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(label(("channel-name", id), &channel.name)),
        )
        .children(badge)
        .children(joined.and_then(|joined| joined.elapsed).map(|elapsed| {
            div()
                .flex_none()
                .text_size(rpx(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::GREEN))
                .child(voice::format_elapsed(elapsed))
        }))
        .when(selected, |this| {
            this.child(theme::icon("user-plus", 14., theme::muted()))
                .child(theme::icon("settings", 14., theme::muted()))
        })
}

const SUBTITLE_HEIGHT: f32 = 14.;
const USER_PANEL_HEIGHT: f32 = 56.;
const ACCOUNT_AVATAR_SIZE: f32 = 32.;

/// The custom status (or the presence when there is none), which slides up to reveal the
/// username while the panel is hovered, like Discord's.
fn account_subtitle(user: &User, hovered: bool, animate: bool, cx: &App) -> AnyElement {
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
        .filter(|status| user.status != Status::Offline && !status.is_expired(SystemTime::now()));
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
                Some(
                    img(source)
                        .debug_selector(|| "account-status-emoji".into())
                        .size(rpx(SUBTITLE_HEIGHT))
                        .flex_none(),
                )
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
    let hovered = hovered && !user.username.is_empty();
    let lines = div()
        .absolute()
        .left_0()
        .right_0()
        .flex()
        .flex_col()
        .child(status_line)
        .child(line().child(div().truncate().child(user.username.clone())));
    let lines = if animate {
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
        .debug_selector(|| "account-subtitle".into())
        .relative()
        .h(rpx(SUBTITLE_HEIGHT))
        .min_w_0()
        .overflow_hidden()
        .text_size(rpx(12.))
        .line_height(rpx(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::muted()))
        .child(lines)
        .into_any_element()
}

fn account_identity(user: &User, hovered: bool, animate_subtitle: bool, cx: &App) -> Div {
    div()
        .debug_selector(|| "account-identity".into())
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(rpx(8.))
        .py(rpx(5.))
        .rounded(rpx(4.))
        .child(theme::user_avatar(
            Some(user),
            user.id,
            ACCOUNT_AVATAR_SIZE,
            Some((user.status, theme::elevated())),
            hovered,
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
                        .debug_selector(|| "account-name".into())
                        .h(rpx(22.))
                        .mt(rpx(-1.))
                        .mb(rpx(2.))
                        .min_w_0()
                        .text_size(rpx(16.))
                        .line_height(rpx(22.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .child(super::display_name::render(
                            ("self-name", user.id),
                            &user.name,
                            user.name_style.as_ref(),
                            super::display_name::Surface::Account { hovered },
                            cx,
                        )),
                )
                .child(account_subtitle(user, hovered, animate_subtitle, cx)),
        )
}

fn account_button(id: &'static str, icon: &str, color: u32) -> Stateful<Div> {
    theme::icon_button(id, icon, 20., color)
        .debug_selector(move || format!("account-{id}-button"))
        .size(rpx(32.))
        .rounded(rpx(8.))
}

fn account_row(
    nameplate: Option<AnyElement>,
    identity: Option<AnyElement>,
    controls: [AnyElement; 3],
) -> Div {
    div()
        .debug_selector(|| "account-strip".into())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(4.))
        .h(rpx(USER_PANEL_HEIGHT))
        .p(rpx(4.))
        .children(nameplate.map(|nameplate| {
            // The nameplate bleeds one CSS pixel past the account strip's border.
            div()
                .debug_selector(|| "account-nameplate".into())
                .absolute()
                .top(rpx(-1.))
                .bottom(rpx(-1.))
                .left_0()
                .right(rpx(-1.))
                .rounded(rpx(7.))
                .child(nameplate)
        }))
        .children(identity)
        .child(div().flex().flex_none().items_center().children(controls))
}

fn status_label(status: Status) -> &'static str {
    match status {
        Status::Online => "Online",
        Status::Idle => "Idle",
        Status::DoNotDisturb => "Do Not Disturb",
        Status::Offline => "Invisible",
    }
}

fn account_tooltip(
    element: Stateful<Div>,
    app: &DiscordApp,
    control: AccountControl,
    label: &'static str,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    element
        .relative()
        .on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, cx| {
                    if *hovered {
                        app.channel_rows.hovered_account_control = Some(control);
                    } else if app.channel_rows.hovered_account_control == Some(control) {
                        app.channel_rows.hovered_account_control = None;
                    } else {
                        return;
                    }
                    app.views.channel_list.update(cx, |_, cx| cx.notify());
                })
                .ok();
        })
        .when(
            app.channel_rows.hovered_account_control == Some(control),
            |this| this.child(theme::tooltip_above(label)),
        )
}

pub(crate) fn self_invisible(app: &DiscordApp) -> bool {
    app.store
        .user(app.store.current_user_id)
        .is_some_and(|user| user.status == Status::Offline)
}

pub(crate) fn sharing_paused_card(app: &DiscordApp, window: &Window) -> Option<AnyElement> {
    let position = app.sharing_paused?;
    app.current_game.as_ref()?;
    if !self_invisible(app) {
        return None;
    }
    let close = app.entity.clone();
    let go_online = app.entity.clone();
    Some(
        div()
            .id("sharing-paused")
            .absolute()
            .left(position.x - px(SHARING_CARD_ANCHOR))
            .bottom(window.viewport_size().height - position.y + px(SHARING_CARD_ANCHOR))
            .flex()
            .flex_col()
            .w(rpx(SHARING_CARD_WIDTH))
            .p_4()
            .rounded(rpx(12.))
            .bg(rgb(theme::surface_high()))
            .border_1()
            .border_color(rgba(theme::border()))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(move |_, _, cx| {
                close
                    .update(cx, |app, cx| {
                        app.sharing_paused = None;
                        cx.notify();
                    })
                    .ok();
            })
            .child(
                div()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .child("Activity sharing is paused"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(6.))
                    .mt(rpx(2.))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(theme::muted()))
                    .child(div().size(rpx(8.)).rounded_full().bg(rgb(theme::GREEN)))
                    .child("Go Online to share"),
            )
            .child(
                div()
                    .mt_2()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .text_color(rgb(theme::text()))
                    .child(
                        "Being Invisible hides my activity information about games and \
                         connected apps I use.",
                    ),
            )
            .child(
                div()
                    .id("sharing-go-online")
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(rpx(36.))
                    .mt_3()
                    .rounded(rpx(8.))
                    .cursor_pointer()
                    .bg(rgb(theme::BLURPLE))
                    .hover(|style| style.opacity(0.9))
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .on_click(move |_, _, cx| {
                        go_online
                            .update(cx, |app, cx| {
                                app.sharing_paused = None;
                                app.set_self_status(
                                    SelfStatus::Online,
                                    StatusDuration::Forever,
                                    cx,
                                );
                            })
                            .ok();
                    })
                    .child("Go Online"),
            )
            .into_any_element(),
    )
}

fn current_game_row(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let name = app.current_game.as_deref()?;
    let invisible = self_invisible(app);
    let entity = app.entity.clone();
    let icon = app
        .current_game_application()
        .and_then(|id| app.game_icon_spec(id))
        .and_then(|spec| images::resolve(&spec, theme::image_px(CURRENT_GAME_ICON), cx))
        .map(|path| img(path).size_full().rounded(rpx(8.)).into_any_element())
        .unwrap_or_else(|| theme::icon("gamepad-2", 20., theme::muted()).into_any_element());
    Some(
        div()
            .debug_selector(|| "account-game".into())
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(8.))
            .p(rpx(8.))
            .border_b_1()
            .border_color(rgba(theme::border()))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(rpx(CURRENT_GAME_ICON))
                    .child(icon),
            )
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
                            .line_height(rpx(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::strong()))
                            .child(label("account-game-name", name)),
                    )
                    .child(
                        div()
                            .id("account-game-sharing")
                            .self_start()
                            .text_size(rpx(12.))
                            .line_height(rpx(16.))
                            .text_color(rgb(theme::muted()))
                            .when(invisible, |this| {
                                this.cursor_pointer()
                                    .hover(|style| style.underline())
                                    .on_click(move |event, _, cx| {
                                        let position = event.position();
                                        entity
                                            .update(cx, |app, cx| {
                                                app.sharing_paused = Some(position);
                                                cx.notify();
                                            })
                                            .ok();
                                    })
                            })
                            .child("Not Sharing"),
                    ),
            )
            .into_any_element(),
    )
}

fn user_panel(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let account_bounds = app.channel_rows.account_bounds.clone();
    let user = app.store.user(app.store.current_user_id);
    let nameplate = user
        .and_then(|user| app.store.nameplate(None, user.id))
        .and_then(|nameplate| {
            member_list::nameplate_layer(
                nameplate,
                app.store.current_user_id,
                USER_PANEL_HEIGHT + 2.,
                app.hovered_account,
                cx,
            )
        });
    let voice = &app.voice;
    let (mic, mic_color, mic_tip) = if voice.self_mute || voice.self_deaf {
        ("mic-off", theme::RED, "Unmute")
    } else {
        ("mic", theme::text(), "Mute")
    };
    let (deafen, deafen_color, deafen_tip) = if voice.self_deaf {
        ("headphone-off", theme::RED, "Undeafen")
    } else {
        ("headphones", theme::text(), "Deafen")
    };
    let identity = user.map(|user| {
        account_identity(
            user,
            app.hovered_account,
            app.channel_rows.subtitle_hovered_once,
            cx,
        )
        .id("account")
        .cursor_pointer()
        // Translucent so a nameplate behind the panel stays visible.
        .hover(|style| style.bg(rgba(theme::account_hover())))
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
        .on_click(profile::account_opener(
            app,
            user.id,
            app.channel_rows.account_bounds.clone(),
        ))
        .into_any_element()
    });
    let controls = [
        account_button("mic", mic, mic_color)
            .map(|this| account_tooltip(this, app, AccountControl::Mute, mic_tip))
            .on_click({
                let entity = app.entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |app, cx| app.toggle_self_mute(cx)).ok();
                }
            })
            .into_any_element(),
        account_button("deafen", deafen, deafen_color)
            .map(|this| account_tooltip(this, app, AccountControl::Deafen, deafen_tip))
            .on_click({
                let entity = app.entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |app, cx| app.toggle_self_deaf(cx)).ok();
                }
            })
            .into_any_element(),
        account_button("user-settings", "settings", theme::text())
            .map(|this| account_tooltip(this, app, AccountControl::Settings, "User Settings"))
            .on_click({
                let entity = app.entity.clone();
                move |_, window, cx| {
                    entity
                        .update(cx, |app, cx| app.open_settings(window, cx))
                        .ok();
                }
            })
            .into_any_element(),
    ];
    let account = account_row(nameplate, identity, controls)
        // The account strip, not the outer footer: a connected voice panel must not move the
        // popout or change the height available above the account row.
        .on_prepaint(move |bounds, _, _| account_bounds.set(Some(bounds)));
    div()
        .flex()
        .flex_col()
        .flex_none()
        .m_2()
        .rounded(rpx(8.))
        .overflow_hidden()
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .children(voice::panel(app))
        .children(current_game_row(app, cx))
        .child(account)
}
