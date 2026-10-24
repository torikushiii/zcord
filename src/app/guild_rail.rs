use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};

use crate::{
    animated,
    app::{
        DiscordApp,
        message_body::emoji_pieces,
        rich_text::{PieceStyle, rich_text},
    },
    images::{self, ImageSpec},
    store::{Folder, Guild, RailEntry},
    theme::{self, rpx},
};

pub(crate) const WIDTH: f32 = 72.;
pub const ICON: f32 = 48.;
const DEFAULT_FOLDER_COLOR: u32 = 0x5865F2;
const HOME_KEY: u64 = 1;
const ADD_SERVER_KEY: u64 = 2;
const DISCOVER_KEY: u64 = 3;
const CIRCLE: f32 = ICON / 2.;
const SQUARE: f32 = 16.;
const MORPH: Duration = Duration::from_millis(180);

struct Morph {
    id: u64,
    started: Instant,
    from: f32,
    to: f32,
}

#[derive(Default)]
pub struct Morphs {
    active: HashMap<u64, Morph>,
    next_id: u64,
}

impl Morphs {
    pub fn start(&mut self, key: u64, hovered: bool) {
        let now = Instant::now();
        let (from, to) = if hovered {
            (CIRCLE, SQUARE)
        } else {
            (SQUARE, CIRCLE)
        };
        let from = self.running(key, now).map_or(from, |morph| {
            let progress = (now - morph.started).as_secs_f32() / MORPH.as_secs_f32();
            morph.from + (morph.to - morph.from) * ease_out_quint()(progress)
        });
        self.active
            .retain(|_, morph| now.duration_since(morph.started) < MORPH);
        self.next_id += 1;
        self.active.insert(
            key,
            Morph {
                id: self.next_id,
                started: now,
                from,
                to,
            },
        );
    }

    fn running(&self, key: u64, now: Instant) -> Option<&Morph> {
        self.active
            .get(&key)
            .filter(|morph| now.duration_since(morph.started) < MORPH)
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
) -> AnyElement {
    let (key, selected) = match corners {
        Corners::Fixed(radius) => return element.rounded(rpx(radius)).into_any_element(),
        Corners::Morph(key, selected) => (key, selected),
    };
    if selected {
        return element.rounded(rpx(SQUARE)).into_any_element();
    }
    match app.rail_morphs.running(key, Instant::now()) {
        Some(morph) => {
            let (from, to) = (morph.from, morph.to);
            element
                .with_animation(
                    ("rail-morph", morph.id),
                    Animation::new(MORPH).with_easing(ease_out_quint()),
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
                "Direct Messages".into(),
                false,
                false,
                app,
            )
            .child(round(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(ICON))
                    .bg(rgb(theme::ELEVATED))
                    .group_hover("rail-item", |style| style.bg(rgb(theme::BLURPLE)))
                    .child(theme::icon("discord", 28., theme::STRONG)),
                Corners::Morph(HOME_KEY, false),
                app,
            ))
            .children(corner_badge(app.store.dm_mentions)),
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
        .child(action_item(
            "add-server",
            ADD_SERVER_KEY,
            "plus",
            "Add a Server",
            app,
        ))
        .child(action_item(
            "discover",
            DISCOVER_KEY,
            "compass",
            "Discover",
            app,
        ))
}

fn guild_item(app: &DiscordApp, guild: &Guild, cx: &App) -> Stateful<Div> {
    let id = guild.id;
    let selected = app.selected_guild_id == id;
    let entity = app.entity.clone();
    rail_item(
        ("guild", id),
        id,
        guild.name.clone().into(),
        selected,
        guild.unread,
        app,
    )
    .on_click(move |_, window, cx| {
        entity
            .update(cx, |this, cx| this.select_guild(id, window, cx))
            .ok();
    })
    .child(guild_icon(
        guild,
        ICON,
        Corners::Morph(id, selected),
        app,
        cx,
    ))
    .children(corner_badge(guild.mentions))
}

fn guild_icon(
    guild: &Guild,
    size: f32,
    corners: Corners,
    app: &DiscordApp,
    cx: &App,
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
        );
    }
    let image = spec.and_then(|spec| images::resolve(&spec, pixels, cx));
    match image {
        Some(path) => round(img(path).size(rpx(size)).flex_none(), corners, app),
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
        ),
    }
}

fn folder_item(app: &DiscordApp, folder: &Folder, cx: &App) -> AnyElement {
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

    let entity = app.entity.clone();
    let header = rail_item(
        ("folder", id),
        id,
        name,
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
                                .map(|guild| guild_icon(guild, 16., Corners::Fixed(8.), app, cx)),
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

fn rail_item(
    id: impl Into<ElementId>,
    key: u64,
    label: SharedString,
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
        .when(hovered, |this| this.child(tooltip(key, &label)))
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

fn tooltip(key: u64, label: &str) -> impl IntoElement {
    div()
        .absolute()
        .left(rpx(WIDTH - 4.))
        .top(rpx(ICON / 2.))
        .size_0()
        .child(deferred(
            anchored().anchor(Anchor::LeftCenter).child(
                div()
                    .flex()
                    .items_center()
                    .child(theme::icon("tooltip-arrow", 6., theme::TOOLTIP_BG).h(rpx(12.)))
                    .child(
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
                    ),
            ),
        ))
}

fn action_item(
    id: &'static str,
    key: u64,
    icon: &'static str,
    label: &'static str,
    app: &DiscordApp,
) -> impl IntoElement {
    rail_item(id, key, label.into(), false, false, app).child(round(
        div()
            .flex()
            .items_center()
            .justify_center()
            .size(rpx(ICON))
            .bg(rgb(theme::ELEVATED))
            .child(theme::icon(icon, 24., theme::TEXT)),
        Corners::Morph(key, false),
        app,
    ))
}
