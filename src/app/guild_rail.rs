use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};

use crate::{
    app::{
        DiscordApp,
        message_body::emoji_pieces,
        rich_text::{PieceStyle, rich_text},
    },
    media::animated,
    media::images::{self, ImageSpec},
    store::{Folder, Guild, RailEntry, VoiceActivity, VoiceMember},
    theme::{self, rpx},
};

#[cfg(test)]
mod tests;

pub(crate) const WIDTH: f32 = 72.;
pub const ICON: f32 = 48.;
const DEFAULT_FOLDER_COLOR: u32 = 0x5865F2;
const HOME_KEY: u64 = 1;
const ADD_SERVER_KEY: u64 = 2;
const DISCOVER_KEY: u64 = 3;
const CIRCLE: f32 = ICON / 2.;
const SQUARE: f32 = 16.;
const TOOLTIP_AVATAR_SIZE: f32 = 28.;
const TOOLTIP_AVATAR_OVERLAP: f32 = 6.;
const TOOLTIP_AVATAR_LIMIT: usize = 5;
const ACTIVITY_TOOLTIP_MAX_WIDTH: f32 = 264.;
/// Circle to square on hover-in is slower than the snap back on hover-out.
const MORPH_IN: Duration = Duration::from_millis(300);
const MORPH_OUT: Duration = Duration::from_millis(180);
pub(crate) const FOLDER_FOLD: Duration = Duration::from_millis(200);
const RAIL_GAP: f32 = 8.;

struct Morph {
    id: u64,
    started: Instant,
    duration: Duration,
    from: f32,
    to: f32,
}

impl Morph {
    fn running(&self, now: Instant) -> bool {
        now.duration_since(self.started) < self.duration
    }
}

#[derive(Default)]
pub struct Morphs {
    active: HashMap<u64, Morph>,
    next_id: u64,
}

impl Morphs {
    pub fn start(&mut self, key: u64, hovered: bool) {
        let now = Instant::now();
        let (from, to, duration) = if hovered {
            (CIRCLE, SQUARE, MORPH_IN)
        } else {
            (SQUARE, CIRCLE, MORPH_OUT)
        };
        let from = self.running(key, now).map_or(from, |morph| {
            let progress = (now - morph.started).as_secs_f32() / morph.duration.as_secs_f32();
            morph.from + (morph.to - morph.from) * ease_out_quint()(progress)
        });
        self.active.retain(|_, morph| morph.running(now));
        self.next_id += 1;
        self.active.insert(
            key,
            Morph {
                id: self.next_id,
                started: now,
                duration,
                from,
                to,
            },
        );
    }

    fn running(&self, key: u64, now: Instant) -> Option<&Morph> {
        self.active.get(&key).filter(|morph| morph.running(now))
    }

    fn clock(&self) -> Option<Instant> {
        if self.active.is_empty() {
            return None;
        }
        let now = Instant::now();
        self.active
            .values()
            .any(|morph| morph.running(now))
            .then_some(now)
    }
}

struct FolderFold {
    id: u64,
    started: Instant,
    from: f32,
    to: f32,
}

impl FolderFold {
    fn openness(&self, now: Instant) -> Option<f32> {
        let progress = now.duration_since(self.started).as_secs_f32() / FOLDER_FOLD.as_secs_f32();
        (progress < 1.).then(|| self.from + (self.to - self.from) * ease_out_quint()(progress))
    }
}

#[derive(Default)]
pub struct FolderFolds {
    active: HashMap<u64, FolderFold>,
    next_id: u64,
}

impl FolderFolds {
    pub fn start(&mut self, key: u64, opening: bool) {
        let now = Instant::now();
        let to = if opening { 1. } else { 0. };
        let from = self
            .active
            .get(&key)
            .and_then(|fold| fold.openness(now))
            .unwrap_or(1. - to);
        self.active.retain(|_, fold| fold.openness(now).is_some());
        self.next_id += 1;
        self.active.insert(
            key,
            FolderFold {
                id: self.next_id,
                started: now,
                from,
                to,
            },
        );
    }

    fn running(&self, key: u64, now: Instant) -> Option<&FolderFold> {
        self.active
            .get(&key)
            .filter(|fold| fold.openness(now).is_some())
    }
}

#[derive(Clone, Copy)]
enum Corners {
    Fixed(f32),
    Morph(u64, bool),
}

fn round<E: Styled + IntoElement + 'static>(
    element: E,
    corners: Corners,
    app: &DiscordApp,
    morphs: Option<Instant>,
) -> AnyElement {
    let (key, selected) = match corners {
        Corners::Fixed(radius) => return element.rounded(rpx(radius)).into_any_element(),
        Corners::Morph(key, selected) => (key, selected),
    };
    if selected {
        return element.rounded(rpx(SQUARE)).into_any_element();
    }
    match morphs.and_then(|now| app.rail_morphs.running(key, now)) {
        Some(morph) => {
            let (from, to) = (morph.from, morph.to);
            element
                .with_animation(
                    ("rail-morph", morph.id),
                    Animation::new(morph.duration).with_easing(ease_out_quint()),
                    move |element, delta| element.rounded(rpx(from + (to - from) * delta)),
                )
                .into_any_element()
        }
        None => {
            let radius = if app.hovered_rail == Some(key) {
                SQUARE
            } else {
                CIRCLE
            };
            element.rounded(rpx(radius)).into_any_element()
        }
    }
}

pub fn render(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let morphs = app.rail_morphs.clock();
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
            rail_item(
                "home",
                HOME_KEY,
                Some("Direct Messages"),
                app.is_home(),
                false,
                app,
            )
            .on_click({
                let entity = app.entity.clone();
                move |_, window, cx| {
                    entity
                        .update(cx, |this, cx| this.open_home(window, cx))
                        .ok();
                }
            })
            .child(round(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(ICON))
                    .bg(rgb(if app.is_home() {
                        theme::surface_highest()
                    } else {
                        theme::elevated()
                    }))
                    .group_hover("rail-item", |style| style.bg(rgb(theme::surface_highest())))
                    .child(theme::logo(32.)),
                Corners::Morph(HOME_KEY, app.is_home()),
                app,
                morphs,
            ))
            .children(corner_badge(app.store.home_badge())),
        )
        .child(
            div()
                .w(rpx(32.))
                .h(rpx(1.))
                .flex_none()
                .bg(rgba(theme::border())),
        )
        .children(app.store.rail.iter().filter_map(|entry| {
            match entry {
                RailEntry::Guild(id) => app
                    .store
                    .guild_by_id(*id)
                    .map(|guild| guild_item(app, guild, cx, morphs).into_any_element()),
                RailEntry::Folder(folder) => Some(folder_item(app, folder, cx, morphs)),
            }
        }))
        .child(action_item(
            "add-server",
            ADD_SERVER_KEY,
            "plus",
            "Add a Server",
            app,
            morphs,
        ))
        .child(action_item(
            "discover",
            DISCOVER_KEY,
            "compass",
            "Discover",
            app,
            morphs,
        ))
}

fn guild_item(app: &DiscordApp, guild: &Guild, cx: &App, morphs: Option<Instant>) -> Stateful<Div> {
    let id = guild.id;
    let selected = app.selected_guild_id == id;
    let connected = app
        .voice
        .call
        .as_ref()
        .is_some_and(|call| call.guild_id == id && call.connected);
    let entity = app.entity.clone();
    rail_item(("guild", id), id, None, selected, guild.unread, app)
        .when(app.hovered_rail == Some(id), |this| {
            this.child(guild_tooltip(app, guild, cx))
        })
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |this, cx| this.select_guild(id, window, cx))
                .ok();
        })
        .on_mouse_down(MouseButton::Right, {
            let entity = app.entity.clone();
            move |event, _, cx| {
                let position = event.position;
                entity
                    .update(cx, |app, cx| app.open_guild_menu(id, position, cx))
                    .ok();
            }
        })
        .child(guild_icon(
            guild,
            ICON,
            Corners::Morph(id, selected),
            app,
            cx,
            morphs,
        ))
        .children(corner_badge(guild.mentions))
        .children(
            app.store
                .guild_voice_activity(id)
                .map(|activity| voice_badge(activity, connected)),
        )
}

fn guild_icon(
    guild: &Guild,
    size: f32,
    corners: Corners,
    app: &DiscordApp,
    cx: &App,
    morphs: Option<Instant>,
) -> AnyElement {
    let pixels = theme::image_px(ICON);
    let spec = guild.icon_spec();
    // Like Discord, animated icons only play while their rail item is hovered.
    let animated = spec
        .as_ref()
        .filter(|_| app.hovered_rail == Some(guild.id))
        .and_then(ImageSpec::animated)
        .and_then(|animated| animated::resolve(&animated, pixels, cx));
    if let Some(image) = animated {
        return round(
            animated::image(image)
                .size(rpx(size))
                .flex_none()
                .id(("guild-icon-animated", guild.id)),
            corners,
            app,
            morphs,
        );
    }
    let image = spec.and_then(|spec| images::resolve(&spec, pixels, cx));
    match image {
        Some(path) => round(img(path).size(rpx(size)).flex_none(), corners, app, morphs),
        None => round(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(size))
                .bg(rgb(guild.icon_color))
                .text_size(rpx(size / 3.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(white())
                .child(guild.initials.clone()),
            corners,
            app,
            morphs,
        ),
    }
}

fn folder_item(app: &DiscordApp, folder: &Folder, cx: &App, morphs: Option<Instant>) -> AnyElement {
    let id = folder.id;
    let open = app.is_folder_open(id);
    let fold = app.folder_folds.running(id, Instant::now());
    let expanded = open || fold.is_some();
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

    let entity = app.entity.clone();
    let header = rail_item(
        ("folder", id),
        id,
        Some(&name),
        !open && contains_selected,
        !open && unread,
        app,
    )
    .on_click(move |_, _, cx| {
        entity
            .update(cx, |this, cx| this.toggle_folder(id, cx))
            .ok();
    })
    .child(
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(ICON))
            .rounded(rpx(16.))
            .when(!expanded, |this| this.bg(tint))
            .map(|this| {
                if expanded {
                    this.child(theme::icon("folder", 24., color))
                } else {
                    this.child(div().flex().flex_wrap().gap(rpx(4.)).w(rpx(36.)).children(
                        guilds.iter().take(4).map(|guild| {
                            guild_icon(guild, 16., Corners::Fixed(8.), app, cx, morphs)
                        }),
                    ))
                }
            }),
    )
    .when(!expanded, |this| this.children(corner_badge(mentions)));

    if !expanded {
        return header.into_any_element();
    }
    let list_height = guilds.len() as f32 * (ICON + RAIL_GAP) + RAIL_GAP;
    let body = div()
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
        .children(
            guilds
                .iter()
                .map(|guild| guild_item(app, guild, cx, morphs)),
        );
    match fold {
        Some(fold) => {
            let (from, to) = (fold.from, fold.to);
            body.overflow_hidden()
                .with_animation(
                    ("folder-fold", fold.id),
                    Animation::new(FOLDER_FOLD).with_easing(ease_out_quint()),
                    move |this, delta| {
                        this.h(rpx(ICON + list_height * (from + (to - from) * delta)))
                    },
                )
                .into_any_element()
        }
        None => body.into_any_element(),
    }
}

fn rail_item(
    id: impl Into<ElementId>,
    key: u64,
    label: Option<&str>,
    selected: bool,
    unread: bool,
    app: &DiscordApp,
) -> Stateful<Div> {
    let hovered = app.hovered_rail == Some(key);
    let entity = app.entity.clone();
    div()
        .id(id)
        .on_hover(move |hovered, _, cx| {
            entity
                .update(cx, |app, cx| app.set_rail_hover(key, *hovered, cx))
                .ok();
        })
        .when(hovered, |this| {
            this.children(label.map(|label| tooltip(key, label)))
        })
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
                .bg(rgb(theme::strong()))
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
            .child(theme::badge(count, theme::base_lowest()))
    })
}

fn voice_badge(activity: VoiceActivity, connected: bool) -> impl IntoElement {
    let (icon, size) = match activity {
        VoiceActivity::Voice => ("video-volume", 14.),
        VoiceActivity::Streaming => ("monitor-filled", 12.),
    };
    div()
        .absolute()
        .top(rpx(-4.))
        .right(rpx(8.))
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(20.))
        .rounded_full()
        .bg(rgb(theme::base_lowest()))
        .when(connected, |this| {
            this.bg(rgb(theme::GREEN))
                .border(rpx(2.))
                .border_color(rgb(theme::base_lowest()))
        })
        .child(theme::icon(
            icon,
            size,
            if connected { 0xFFFFFF } else { theme::strong() },
        ))
}

fn tooltip(key: u64, label: &str) -> impl IntoElement {
    positioned_tooltip(
        theme::tooltip_bubble()
            .text_size(rpx(16.))
            .line_height(rpx(20.))
            .child(
                rich_text(
                    ("rail-tooltip", key),
                    emoji_pieces(label, &PieceStyle::default()),
                )
                .emoji_scale(1.125),
            ),
    )
}

fn guild_tooltip(app: &DiscordApp, guild: &Guild, cx: &App) -> AnyElement {
    let members = app.store.guild_voice_members(guild.id);
    if members.is_empty() {
        return tooltip(guild.id, &guild.name).into_any_element();
    }
    let (streaming, voice): (Vec<_>, Vec<_>) = members.iter().partition(|member| member.streaming);
    let ids = |members: Vec<&VoiceMember>| -> Vec<u64> {
        members.iter().map(|member| member.user_id).collect()
    };
    let (streaming, voice) = (ids(streaming), ids(voice));
    let row = |icon, members: &[u64]| {
        activity_row(
            icon,
            members.len(),
            members.iter().map(|user_id| {
                theme::undecorated_member_avatar(
                    &app.store,
                    guild.id,
                    *user_id,
                    TOOLTIP_AVATAR_SIZE - 4.,
                    cx,
                )
                .into_any_element()
            }),
        )
    };
    positioned_tooltip(
        activity_tooltip_bubble(guild.id, &guild.name)
            .when(!voice.is_empty(), |this| {
                this.child(row("video-volume", &voice))
            })
            .when(!streaming.is_empty(), |this| {
                this.child(row("monitor-share", &streaming))
            }),
    )
    .into_any_element()
}

fn activity_tooltip_bubble(key: u64, name: &str) -> Div {
    theme::tooltip_bubble()
        .max_w(rpx(ACTIVITY_TOOLTIP_MAX_WIDTH))
        .flex_none()
        .flex()
        .flex_col()
        .gap(rpx(6.))
        .child(
            div()
                .id(("guild-tooltip-header", key))
                .min_w_0()
                .mb(rpx(2.))
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .child(
                    rich_text(
                        ("rail-tooltip", key),
                        emoji_pieces(name, &PieceStyle::default()),
                    )
                    .line_clamp(2)
                    .emoji_scale(1.125),
                ),
        )
}

fn activity_row(icon: &str, count: usize, avatars: impl Iterator<Item = AnyElement>) -> Div {
    let visible = count.min(TOOLTIP_AVATAR_LIMIT);
    let remaining = count.saturating_sub(TOOLTIP_AVATAR_LIMIT);
    let pitch = TOOLTIP_AVATAR_SIZE - TOOLTIP_AVATAR_OVERLAP;
    let overlap_count = visible.saturating_sub(1) + usize::from(remaining > 0);
    let stack_width =
        visible as f32 * TOOLTIP_AVATAR_SIZE - overlap_count as f32 * TOOLTIP_AVATAR_OVERLAP;
    div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(TOOLTIP_AVATAR_SIZE))
        .child(theme::icon(icon, 20., theme::muted()).flex_none())
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .child(
                    div()
                        .relative()
                        .flex_none()
                        .w(rpx(stack_width))
                        .h(rpx(TOOLTIP_AVATAR_SIZE))
                        .children(avatars.take(visible).enumerate().map(|(index, avatar)| {
                            div()
                                .absolute()
                                .left(rpx(index as f32 * pitch))
                                .top_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(rpx(TOOLTIP_AVATAR_SIZE))
                                .rounded_full()
                                .border(rpx(2.))
                                .border_color(rgb(theme::tooltip_bg()))
                                .bg(rgb(theme::tooltip_bg()))
                                .overflow_hidden()
                                .child(avatar)
                        })),
                )
                .when(remaining > 0, |this| {
                    let selector = format!("rail-activity-overflow-{icon}");
                    this.child(
                        div()
                            .debug_selector(move || selector.clone())
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .min_w(rpx(TOOLTIP_AVATAR_SIZE))
                            .h(rpx(TOOLTIP_AVATAR_SIZE))
                            .px(rpx(4.))
                            .rounded_full()
                            .border(rpx(2.))
                            .border_color(rgb(theme::tooltip_bg()))
                            .bg(rgb(theme::bg()))
                            .text_size(rpx(12.))
                            .line_height(rpx(16.))
                            .child(format!("+{remaining}")),
                    )
                }),
        )
}

fn positioned_tooltip(body: Div) -> Div {
    div()
        .absolute()
        .left(rpx(WIDTH - 4.))
        .top(rpx(ICON / 2.))
        .size_0()
        .child(
            canvas(
                move |bounds, window, cx| {
                    let mut popup = anchored()
                        .position(bounds.origin)
                        .anchor(Anchor::LeftCenter)
                        .snap_to_window_with_margin(px(8.))
                        .child(theme::tooltip_entrance(
                            div().relative().pl(rpx(8.)).child(body).child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left_0()
                                    .flex()
                                    .items_center()
                                    .child(theme::tooltip_arrow_left()),
                            ),
                            cx,
                        ))
                        .into_any_element();
                    // Measure outside the cached, scrollable rail; only its anchor belongs there.
                    popup.layout_as_root(
                        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                        window,
                        cx,
                    );
                    window.defer_draw(popup, Point::default(), 0, None);
                },
                |_, _, _, _| {},
            )
            .size_0(),
        )
}

fn action_item(
    id: &'static str,
    key: u64,
    icon: &'static str,
    label: &'static str,
    app: &DiscordApp,
    morphs: Option<Instant>,
) -> impl IntoElement {
    rail_item(id, key, Some(label), false, false, app).child(round(
        div()
            .flex()
            .items_center()
            .justify_center()
            .size(rpx(ICON))
            .bg(rgb(theme::elevated()))
            .child(theme::icon(icon, 24., theme::text())),
        Corners::Morph(key, false),
        app,
        morphs,
    ))
}
