use crate::app::menu::PopupButton as _;
use gpui::{prelude::*, *};
use gpui_component::ActiveTheme;

use super::WIDTH;
use crate::app::emoji_picker::{PickerTarget, emoji_image};
use crate::store::ActivityEntryView;
use crate::{
    app::DiscordApp,
    media::images,
    store::ActivityCard,
    theme::{self, WithoutTextSelection, rpx},
};

const THUMBNAIL: f32 = 48.;
const FACE: f32 = 16.;
const BADGE_ICON: f32 = 12.;
const COLLAPSED_CARDS: usize = 3;

pub(super) fn section(app: &DiscordApp, cards: &[ActivityCard], cx: &App) -> AnyElement {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let expanded = app.activity_expanded();
    let collapsible = cards.len() > COLLAPSED_CARDS;
    let shown = if collapsible && !expanded {
        COLLAPSED_CARDS
    } else {
        cards.len()
    };
    let toggle = collapsible.then(|| {
        let entity = app.entity.clone();
        theme::icon_button(
            "activity-expand",
            if expanded {
                "chevron-down"
            } else {
                "chevron-right"
            },
            20.,
            theme::muted(),
        )
        .w(rpx(24.))
        .h(rpx(18.))
        .rounded(rpx(4.))
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.toggle_activity_expanded(cx))
                .ok();
        })
    });
    div()
        .flex()
        .flex_col()
        .without_text_selection()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .pt(rpx(20.))
                .pb(rpx(4.))
                .pl(rpx(8.))
                .pr(rpx(4.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::muted()))
                .child(format!("Activity — {}", cards.len()))
                .children(toggle),
        )
        .children(
            cards[..shown]
                .iter()
                .map(|card| card_view(app, card, now_ms, cx)),
        )
        .into_any_element()
}

fn card_view(app: &DiscordApp, card: &ActivityCard, now_ms: i64, cx: &App) -> AnyElement {
    let guild_id = app.selected_guild_id;
    let user_id = card.user_id;
    let name = app
        .store
        .member_name(guild_id, user_id)
        .map(str::to_owned)
        .or_else(|| app.store.user(user_id).map(|user| user.name.clone()))
        .unwrap_or_default();
    let name_color = app
        .store
        .member_color(guild_id, user_id)
        .unwrap_or_else(theme::strong);
    let entity = app.entity.clone();
    let entry_id = card.entry_id;
    let tops = app.activity_card_tops.clone();
    let open = app
        .activity_popout
        .as_ref()
        .is_some_and(|popout| popout.entry_id == entry_id);
    div()
        .id(("activity-card", entry_id))
        .relative()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .mt(rpx(8.))
        .p(rpx(8.))
        .rounded(rpx(12.))
        .bg(rgb(theme::surface_high()))
        .border_1()
        .border_color(rgba(theme::border_soft()))
        .cursor_pointer()
        .when(open, |this| this.bg(rgba(theme::control_selected())))
        .hover(|style| style.bg(rgba(theme::control_selected())))
        .on_hover(move |hovered, window, cx| {
            entity
                .update(cx, |app, cx| {
                    app.hover_activity_card(entry_id, *hovered, window, cx)
                })
                .ok();
        })
        .child(
            canvas(
                move |bounds, _, _| {
                    tops.borrow_mut().insert(entry_id, bounds.top());
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .mb(rpx(-2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(6.))
                        .h(rpx(18.))
                        .mb(rpx(2.))
                        .child(theme::member_avatar(
                            &app.store, guild_id, user_id, FACE, None, false, cx,
                        ))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(name_color))
                                .child(name),
                        ),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::text()))
                        .child(card.game_name.clone()),
                )
                .child(badges(card, now_ms, cx)),
        )
        .child(thumbnail(app, card, cx))
        .into_any_element()
}

fn badges(card: &ActivityCard, now_ms: i64, cx: &App) -> Div {
    let mut row = div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(16.))
        .min_w_0()
        .overflow_hidden();
    if let Some(since) = card.live_since_ms {
        row = row.child(
            badge("activity-game", theme::GREEN)
                .font_family(cx.theme().mono_font_family.clone())
                .text_color(rgb(theme::GREEN))
                .child(timer((now_ms - since).max(0) / 1000)),
        );
    } else if let Some(hours) = card.most_played_hours {
        row = row.child(
            badge("activity-trophy", theme::muted()).child(format!("Most played: {hours}h")),
        );
    } else {
        row = row.child(
            badge("activity-game", theme::muted())
                .child(ago((now_ms - card.created_ms).max(0) / 1000)),
        );
    }
    if let Some(days) = card.streak_days {
        row = row.child(
            badge("activity-streak", theme::muted())
                .relative()
                .id(("activity-streak", card.entry_id))
                .child(format!("{days}x Streak"))
                .child(theme::hover_tooltip(
                    ("activity-streak-tip", card.entry_id),
                    format!("Played {days} days in a row"),
                )),
        );
    }
    row
}

fn badge(icon: &str, icon_color: u32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(4.))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .text_color(rgb(theme::muted()))
        .child(theme::icon(icon, BADGE_ICON, icon_color))
}

fn thumbnail(app: &DiscordApp, card: &ActivityCard, cx: &App) -> AnyElement {
    let image = card
        .application_id
        .and_then(|id| app.game_icon_spec(id))
        .and_then(|spec| images::resolve(&spec, theme::image_px(THUMBNAIL), cx));
    match image {
        Some(path) => img(path)
            .size(rpx(THUMBNAIL))
            .flex_none()
            .rounded(rpx(4.))
            .into_any_element(),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(rpx(THUMBNAIL))
            .rounded(rpx(8.))
            .bg(rgb(theme::icon_bg()))
            .child(theme::icon("activity-game", 24., theme::muted()))
            .into_any_element(),
    }
}

fn timer(seconds: i64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Discord's compact "ago" steps: seconds, minutes, hours under 12, days under 9, weeks
/// under 4, then 30-day months.
fn ago(seconds: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let round = |unit: i64| (seconds as f64 / unit as f64).round() as i64;
    if seconds < MINUTE {
        format!("{seconds}s ago")
    } else if seconds < HOUR {
        format!("{}m ago", round(MINUTE))
    } else if seconds < 12 * HOUR {
        format!("{}h ago", round(HOUR))
    } else if seconds < 9 * DAY {
        format!("{}d ago", round(DAY))
    } else if seconds < 28 * DAY {
        format!("{}w ago", round(7 * DAY))
    } else {
        format!("{}mo ago", round(30 * DAY))
    }
}

const POPOUT_WIDTH: f32 = 348.;
const COVER_WIDTH: f32 = 72.;
const COVER_HEIGHT: f32 = 96.;
const DM_CARD_WIDTH: f32 = 400.;
const DM_ICON: f32 = 64.;
const DM_FACE: f32 = 20.;
const HERO_TITLE: u32 = 0xFFFFFFD9;
const HERO_LABEL: u32 = 0xFFFFFFB3;
const QUICK_REACTIONS: usize = 6;

/// Discord tints activity heroes with a 45° gradient of the game icon's two main colours.
fn hero_background(
    app: &DiscordApp,
    application_id: Option<u64>,
    fallback: impl Into<Background>,
) -> Background {
    match application_id.and_then(|id| app.activity_colors(id)) {
        Some([from, to]) => linear_gradient(
            45.,
            linear_color_stop(rgb(from), 0.),
            linear_color_stop(rgb(to), 1.),
        ),
        None => fallback.into(),
    }
}

fn hero_badges(card: &ActivityCard, now_ms: i64, cx: &App) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(rpx(8.))
        .gap_y(rpx(4.))
        .children(badge_items(card, now_ms, cx))
}

fn badge_items(card: &ActivityCard, now_ms: i64, cx: &App) -> Vec<Div> {
    let label = |icon: &str, text: SharedString| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(4.))
            .overflow_hidden()
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .text_color(rgba(HERO_LABEL))
            .child(
                theme::icon(icon, BADGE_ICON, 0xFFFFFF)
                    .flex_none()
                    .opacity(0.7),
            )
            .child(div().min_w_0().truncate().child(text))
    };
    let mut badges = Vec::new();
    if let Some(since) = card.live_since_ms {
        badges.push(
            label(
                "activity-game",
                timer((now_ms - since).max(0) / 1000).into(),
            )
            .font_family(cx.theme().mono_font_family.clone()),
        );
    } else if let Some(hours) = card.most_played_hours {
        badges.push(label(
            "activity-trophy",
            format!("Most played — {hours}h this week").into(),
        ));
    } else {
        badges.push(label(
            "activity-game",
            ago((now_ms - card.created_ms).max(0) / 1000).into(),
        ));
    }
    if let Some(days) = card.streak_days {
        badges.push(label("activity-streak", format!("{days}x Streak").into()));
    }
    if card.first_time {
        badges.push(label("sprout", "New Player".into()));
    }
    if card.trending {
        badges.push(label("flame", "Trending".into()));
    }
    badges
}

fn cover(app: &DiscordApp, card: &ActivityCard, cx: &App) -> AnyElement {
    let path = card
        .application_id
        .and_then(|id| app.game_cover(id))
        .and_then(|url| {
            images::resolve(
                &images::ImageSpec::Media {
                    url,
                    width: 0,
                    height: 0,
                    animated: false,
                },
                theme::image_px(COVER_HEIGHT),
                cx,
            )
        });
    match path {
        Some(path) => img(path)
            .w(rpx(COVER_WIDTH))
            .h(rpx(COVER_HEIGHT))
            .flex_none()
            .rounded(rpx(4.))
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .w(rpx(COVER_WIDTH))
            .h(rpx(COVER_HEIGHT))
            .child(thumbnail(app, card, cx))
            .into_any_element(),
    }
}

fn display_name(app: &DiscordApp, user_id: u64) -> String {
    app.store
        .member_name(app.selected_guild_id, user_id)
        .map(str::to_owned)
        .or_else(|| app.store.user(user_id).map(|user| user.name.clone()))
        .unwrap_or_default()
}

pub(in crate::app) fn popout(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    let state = app.activity_popout.as_ref()?;
    let card = app
        .activity_cards()
        .into_iter()
        .find(|card| card.entry_id == state.entry_id)?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let entry_id = card.entry_id;
    let user_id = card.user_id;
    let list_left =
        window.viewport_size().width - window.rem_size() * (WIDTH / theme::BASE_FONT_SIZE);
    let hover = app.entity.clone();
    let menu = app.entity.clone();
    let picker = app.entity.clone();
    let reactions = app
        .recent_emojis(cx)
        .into_iter()
        .take(QUICK_REACTIONS)
        .enumerate()
        .map(|(index, choice)| {
            let entity = app.entity.clone();
            div()
                .id(("activity-reaction", index))
                .flex()
                .items_center()
                .justify_center()
                .w(rpx(24.))
                .h(rpx(32.))
                .cursor_pointer()
                .child(emoji_image(&choice, 24., false, cx))
                .on_click(move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| app.react_to_activity(entry_id, &choice, cx))
                        .ok();
                })
        });
    let header = div()
        .flex()
        .items_center()
        .gap(rpx(16.))
        .h(rpx(24.))
        .mb(rpx(16.))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(rpx(8.))
                .child(theme::member_avatar(
                    &app.store,
                    app.selected_guild_id,
                    user_id,
                    24.,
                    None,
                    false,
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .gap(rpx(4.))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::muted()))
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::strong()))
                                .child(display_name(app, user_id)),
                        )
                        .child(if card.live_since_ms.is_some() {
                            "is playing"
                        } else {
                            "played"
                        }),
                ),
        )
        .child(
            div()
                .relative()
                .child(
                    theme::icon_button("activity-popout-menu", "ellipsis", 16., theme::muted())
                        .on_popup_click(
                            crate::app::menu::shown(app, |app| {
                                app.activity_popout
                                    .as_ref()
                                    .is_some_and(|popout| popout.menu)
                            }),
                            move |_, _, cx| {
                                menu.update(cx, |app, cx| app.toggle_activity_menu(cx)).ok();
                            },
                        ),
                )
                .when(state.menu, |this| {
                    // A layer of its own, so the popout's hero and reactions can't paint over it.
                    this.child(
                        div().absolute().top(rpx(24.)).left_0().size_0().child(
                            deferred(
                                anchored()
                                    .anchor(Anchor::TopLeft)
                                    .snap_to_window_with_margin(px(8.))
                                    .child(popout_menu(app, user_id, card.application_id)),
                            )
                            .with_priority(2),
                        ),
                    )
                }),
        );
    let hero = div()
        .flex()
        .items_center()
        .gap(rpx(16.))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(hero_background(
            app,
            card.application_id,
            rgb(theme::surface_highest()),
        ))
        .child(cover(app, &card, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(8.))
                .child(
                    div()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgba(HERO_TITLE))
                        .child(card.game_name.clone()),
                )
                .child(hero_badges(&card, now_ms, cx)),
        );
    let reply = div()
        .flex()
        .items_center()
        .h(rpx(38.))
        .pl(rpx(15.))
        .pr(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgba(theme::recessed()))
        .border_1()
        .border_color(rgba(theme::control_selected()))
        .child(
            gpui_component::input::Input::new(&state.input)
                .appearance(false)
                .px_0()
                .text_size(rpx(16.)),
        );
    let body = div()
        .id("activity-popout")
        .flex()
        .flex_col()
        .w(rpx(POPOUT_WIDTH))
        .rounded(rpx(12.))
        .bg(rgb(theme::surface_high()))
        .border_1()
        .border_color(rgba(theme::border_soft()))
        .shadow_lg()
        .occlude()
        .without_text_selection()
        .on_mouse_down_out({
            let entity = app.entity.clone();
            move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.close_activity_popout(cx))
                    .ok();
            }
        })
        .on_hover(move |hovered, window, cx| {
            hover
                .update(cx, |app, cx| {
                    app.hover_activity_popout(*hovered, window, cx)
                })
                .ok();
        })
        .child(
            div()
                .flex()
                .flex_col()
                .p(rpx(16.))
                .child(header)
                .child(hero),
        )
        .child(div().h(px(1.)).mx(rpx(16.)).bg(rgba(theme::border_soft())))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(12.))
                .p(rpx(16.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px(rpx(12.))
                        .children(reactions)
                        .child(
                            theme::icon_button(
                                "activity-reaction-more",
                                "smile",
                                18.,
                                theme::muted(),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                move |event, window, cx| {
                                    let position = event.position;
                                    picker
                                        .update(cx, |app, cx| {
                                            app.toggle_emoji_picker(
                                                PickerTarget::Activity(entry_id),
                                                position,
                                                window,
                                                cx,
                                            )
                                        })
                                        .ok();
                                },
                            ),
                        ),
                )
                .child(reply),
        );
    Some(
        deferred(
            anchored()
                .position(point(list_left - px(8.), state.top))
                .anchor(Anchor::TopRight)
                .snap_to_window_with_margin(px(8.))
                .child(body),
        )
        .with_priority(1)
        .into_any_element(),
    )
}

fn popout_menu(app: &DiscordApp, user_id: u64, application_id: Option<u64>) -> impl IntoElement {
    let row = |key: &'static str, icon: Option<&'static str>, label: &'static str| {
        div()
            .id(key)
            .flex()
            .items_center()
            .gap(rpx(10.))
            .h(rpx(36.))
            .px(rpx(8.))
            .rounded(rpx(4.))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(theme::selected())))
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(theme::text()))
            .children(icon.map(|icon| theme::icon(icon, 20., theme::text())))
            .child(label)
    };
    let profile = app.entity.clone();
    let copy_user = app.entity.clone();
    let copy_app = app.entity.clone();
    div()
        .flex()
        .flex_col()
        .w(rpx(200.))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
        .occlude()
        .child(row("activity-menu-profile", None, "Profile").on_mouse_down(
            MouseButton::Left,
            move |event, _, cx| {
                let position = event.position;
                profile
                    .update(cx, |app, cx| {
                        app.close_activity_popout(cx);
                        app.open_profile(user_id, position, Anchor::TopRight, cx);
                    })
                    .ok();
            },
        ))
        .child(
            div()
                .h(px(1.))
                .my(rpx(4.))
                .mx(rpx(6.))
                .bg(rgba(theme::border())),
        )
        .child(
            row("activity-menu-user-id", Some("id-badge"), "Copy User ID").on_mouse_down(
                MouseButton::Left,
                move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(user_id.to_string()));
                    copy_user
                        .update(cx, |app, cx| app.close_activity_popout(cx))
                        .ok();
                },
            ),
        )
        .children(application_id.map(|application_id| {
            row(
                "activity-menu-app-id",
                Some("id-badge"),
                "Copy Application ID",
            )
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(application_id.to_string()));
                copy_app
                    .update(cx, |app, cx| app.close_activity_popout(cx))
                    .ok();
            })
        }))
}

/// The card a reply to someone's activity shows in chat (message component type 16).
pub(in crate::app) fn dm_card(app: &DiscordApp, entry: &ActivityEntryView, cx: &App) -> AnyElement {
    let card = ActivityCard {
        entry_id: entry.entry_id,
        user_id: entry.user_id,
        game_name: entry.game_name.clone(),
        application_id: entry.application_id,
        live_since_ms: entry.live_since_ms,
        streak_days: entry.streak_days,
        most_played_hours: entry.most_played_hours,
        created_ms: entry.created_ms,
        first_time: entry.first_time,
        trending: entry.trending,
    };
    let now_ms = chrono::Utc::now().timestamp_millis();
    let icon = card
        .application_id
        .and_then(|id| app.game_icon_spec(id))
        .and_then(|spec| images::resolve(&spec, theme::image_px(DM_ICON), cx));
    let badges = badge_items(&card, now_ms, cx)
        .into_iter()
        .take(2)
        .enumerate()
        .map(|(index, badge)| {
            if index == 1 {
                badge.flex_shrink(1.).min_w_0()
            } else {
                badge
            }
        });
    let strong = theme::dark_palette().strong;
    div()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .w_full()
        .max_w(rpx(DM_CARD_WIDTH))
        .mt(rpx(4.))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(hero_background(app, card.application_id, rgb(0x000000)))
        .child(
            div().flex_none().mb(rpx(-2.5)).child(match icon {
                Some(path) => img(path)
                    .size(rpx(DM_ICON))
                    .rounded(rpx(4.))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element(),
                None => div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(DM_ICON))
                    .rounded(rpx(4.))
                    .bg(rgb(theme::dark_palette().icon_bg))
                    .child(theme::icon(
                        "activity-game",
                        32.,
                        theme::dark_palette().muted,
                    ))
                    .into_any_element(),
            }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(6.))
                        .pr(rpx(30.))
                        .child(
                            div()
                                .id(("activity-card-avatar", card.entry_id))
                                .flex_none()
                                .cursor_pointer()
                                .on_popup_click(
                                    crate::app::profile::shown(app, card.user_id),
                                    crate::app::profile::opener(app, card.user_id, Anchor::TopLeft),
                                )
                                .child(theme::user_avatar(
                                    app.store.user(card.user_id),
                                    card.user_id,
                                    DM_FACE,
                                    None,
                                    false,
                                    cx,
                                )),
                        )
                        .child(
                            div()
                                .id(("activity-card-name", card.entry_id))
                                .min_w_0()
                                .truncate()
                                .cursor_pointer()
                                .hover(|style| style.underline())
                                .on_popup_click(
                                    crate::app::profile::shown(app, card.user_id),
                                    crate::app::profile::opener(app, card.user_id, Anchor::TopLeft),
                                )
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(strong))
                                .child(
                                    app.store
                                        .user(card.user_id)
                                        .map(|user| user.name.clone())
                                        .unwrap_or_default(),
                                ),
                        ),
                )
                .child(
                    div()
                        .mt(rpx(2.))
                        .truncate()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgba(HERO_TITLE))
                        .child(card.game_name.clone()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .mt(rpx(4.))
                        .min_w_0()
                        .overflow_hidden()
                        .children(badges),
                ),
        )
        .into_any_element()
}
