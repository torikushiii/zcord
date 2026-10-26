use super::actions::{GameActions, play_button};
use super::art::{activity_art, current_art, presence_art, still_art};
use super::widgets::{stat_value, widget_image_art};
use super::*;

pub(in crate::app::profile) fn activity(
    app: &DiscordApp,
    card: &ProfileCard,
    cx: &App,
) -> Option<AnyElement> {
    let palette = card.palette();
    let game = app
        .store
        .user(card.user_id)
        .and_then(|user| user.game.as_ref());
    let now = Utc::now();
    let outbox = card
        .outbox
        .as_ref()
        .map_or(&[][..], |outbox| outbox.entries.as_slice());
    let current = game.and_then(|game| current_entry(game, outbox, now));
    let entries: Vec<_> = outbox
        .iter()
        .filter(|entry| !matches!(entry.kind, OutboxKind::Unknown))
        // The live entry for the game being played is the current-activity card instead.
        .filter(|entry| !current.is_some_and(|current| current.id == entry.id && live(entry, now)))
        .collect();
    if game.is_none() && entries.is_empty() {
        return None;
    }
    let heading = |text: &'static str| {
        div()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.strong))
            .child(text)
    };
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(24.))
            .children(game.map(|game| {
                div()
                    .flex()
                    .flex_col()
                    .gap(rpx(10.))
                    .child(heading("Current activity"))
                    .child(current_game(card, game, current, palette, cx))
            }))
            .when(!entries.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(10.))
                        .child(heading("Recent activity"))
                        .children(
                            entries
                                .into_iter()
                                .map(|entry| activity_row(card, entry, now, palette, cx)),
                        ),
                )
            })
            .into_any_element(),
    )
}

fn current_game(
    card: &ProfileCard,
    game: &GameActivity,
    entry: Option<&OutboxEntry>,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement {
    let actions = game.application_id.map_or_else(GameActions::default, |id| {
        GameActions::resolve(id, &card.games, &card.applications)
    });
    let art = game
        .large_image
        .as_ref()
        .and_then(|image| {
            presence_art(
                &image.url,
                CURRENT_ART,
                16.,
                ("profile-current-art", card.user_id),
                cx,
            )
        })
        .or_else(|| {
            let path = game
                .application_id
                .and_then(|id| current_art(card, id, cx))?;
            Some(still_art(path, 16.))
        });
    let badge = game.small_image.as_ref().and_then(|image| {
        presence_art(
            &image.url,
            CURRENT_BADGE,
            CURRENT_BADGE,
            ("profile-current-badge", card.user_id),
            cx,
        )
    });
    let elapsed = game
        .started_at
        .and_then(|started| SystemTime::now().duration_since(started).ok())
        .map(clock);
    let line = |id: &'static str, text: &String| {
        div()
            .truncate()
            .text_size(rpx(13.))
            .line_height(rpx(16.))
            .text_color(rgb(palette.strong))
            .child(label((id, card.user_id), text))
    };
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .p(rpx(14.))
        .rounded(rpx(12.))
        .bg(palette.panel)
        .child(
            playing_heading(game, palette)
                .text_size(rpx(13.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(14.))
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(rpx(CURRENT_ART))
                        .rounded(rpx(16.))
                        .map(|this| match art {
                            Some(art) => this.child(art),
                            None => this.bg(palette.hover).child(theme::icon(
                                "gamepad-2",
                                40.,
                                palette.muted,
                            )),
                        })
                        .children(badge.map(|badge| {
                            // Composite over the modal surface so the artwork cannot show through.
                            let backdrop = card
                                .theme()
                                .and_then(|theme| theme.modal_background().as_solid())
                                .unwrap_or_else(|| rgb(theme::BG).into())
                                .blend(palette.panel);
                            div()
                                .absolute()
                                .right(rpx(-8.))
                                .bottom(rpx(-8.))
                                .size(rpx(CURRENT_BADGE + 8.))
                                .p(rpx(4.))
                                .rounded_full()
                                .bg(backdrop)
                                .child(badge)
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .gap(rpx(4.))
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.strong))
                                .child(label(("profile-current-game", card.user_id), &game.name)),
                        )
                        .children(
                            game.details
                                .as_ref()
                                .map(|details| line("profile-current-details", details)),
                        )
                        .children(
                            game.state
                                .as_ref()
                                .map(|state| line("profile-current-state", state)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_x(rpx(10.))
                                .gap_y(rpx(2.))
                                .text_size(rpx(13.))
                                .line_height(rpx(16.))
                                .text_color(rgb(palette.text))
                                .children(elapsed.map(|elapsed| {
                                    activity_badge(timer_icon(game), elapsed, theme::GREEN)
                                        .font_family(cx.theme().mono_font_family.clone())
                                }))
                                .children(
                                    entry.into_iter().flat_map(activity_traits).map(
                                        |(icon, text)| activity_badge(icon, text, palette.text),
                                    ),
                                ),
                        )
                        .children(
                            actions
                                .buttons("profile-current-actions", card.user_id, palette)
                                .map(|buttons| {
                                    buttons.flex_row().flex_wrap().items_center().mt(rpx(4.))
                                }),
                        )
                        .children(
                            play_button(game, palette)
                                .map(|play| div().flex().mt(rpx(4.)).child(play)),
                        ),
                ),
        )
}

/// The playing app's `activity_accessory` widget surface ("Current Mood: …"), a strip along the
/// bottom of the popout's activity card. Discord shows "–" while the value is empty.
fn activity_accessory(
    app: &DiscordApp,
    card: &ProfileCard,
    game: &GameActivity,
    palette: ProfilePalette,
    cx: &App,
) -> Option<AnyElement> {
    let application_id = game.application_id?;
    let accessory = card
        .widget_configs
        .get(&application_id)?
        .accessory
        .as_ref()?;
    let profile = card
        .identities
        .iter()
        .flat_map(|identities| &identities.identities)
        .find(|identity| identity.application_id == application_id)
        .and_then(|identity| identity.profile.as_ref());
    let resolve = |text: &WidgetText| match text {
        WidgetText::Literal(text) => Some(text.clone()),
        WidgetText::Data(key) if key == "username" => profile?.username.clone(),
        WidgetText::Data(key) => profile?
            .stats
            .iter()
            .find(|stat| &stat.name == key)
            .map(|stat| stat_value(&stat.value)),
    };
    let label = accessory.label.as_ref().and_then(resolve)?;
    let value = accessory
        .text
        .as_ref()
        .and_then(resolve)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "–".to_owned());
    let icon = accessory.icon.as_ref().and_then(|icon| {
        widget_image_art(
            application_id,
            icon,
            profile,
            18.,
            4.,
            "profile-accessory-icon",
            cx,
        )
    });
    let entity = app.entity.clone();
    Some(
        div()
            .id("profile-activity-accessory")
            .flex()
            .items_center()
            .gap(rpx(8.))
            // Flush with the card's edges, under a divider, like Discord's footer strip.
            .mx(rpx(-8.))
            .mb(rpx(-8.))
            .px(rpx(10.))
            .py(rpx(10.))
            .bg(palette.raised)
            .rounded_b(rpx(8.))
            .cursor_pointer()
            .hover(move |style| style.bg(palette.raised.opacity(1.3)))
            .text_size(rpx(13.))
            .line_height(rpx(16.))
            .text_color(rgb(palette.strong))
            .children(icon.map(|icon| div().flex_none().size(rpx(18.)).child(icon)))
            .child(
                div().flex_1().min_w_0().truncate().child(
                    div()
                        .font_weight(FontWeight::BOLD)
                        .child(format!("{label}: {value}")),
                ),
            )
            .child(theme::icon("arrow-right", 16., palette.muted))
            .on_click(move |_, _, cx| {
                entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
            })
            .into_any_element(),
    )
}

/// "Playing", or "Playing on Xbox" plus the console's glyph for presences from a console.
fn playing_heading(game: &GameActivity, palette: ProfilePalette) -> Div {
    let platform = match game.platform.as_deref() {
        Some("xbox") => Some(("Xbox", Some("connections/xbox"))),
        Some("ps4" | "ps5" | "playstation") => {
            Some(("PlayStation", Some("connections/playstation")))
        }
        Some("samsung") => Some(("Samsung Galaxy", None)),
        Some("meta_quest") => Some(("Meta Quest", None)),
        _ => None,
    };
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .text_color(rgb(palette.text))
        .child(match platform {
            Some((name, _)) => format!("Playing on {name}"),
            None => "Playing".to_owned(),
        })
        .children(
            platform
                .and_then(|(_, icon)| icon)
                .map(|icon| theme::icon(icon, 14., palette.text)),
        )
}

fn timer_icon(game: &GameActivity) -> &'static str {
    if game.embedded {
        "activity"
    } else {
        "gamepad-2"
    }
}

pub(in crate::app::profile) fn popout_playing(
    app: &DiscordApp,
    card: &ProfileCard,
    cx: &App,
) -> Option<AnyElement> {
    let palette = card.palette();
    let game = app
        .store
        .user(card.user_id)
        .and_then(|user| user.game.as_ref())?;
    let actions = game.application_id.map_or_else(GameActions::default, |id| {
        GameActions::resolve(id, &card.games, &card.applications)
    });
    let now = Utc::now();
    let outbox = card
        .outbox
        .as_ref()
        .map_or(&[][..], |outbox| outbox.entries.as_slice());
    let entry = current_entry(game, outbox, now);
    let art = game
        .large_image
        .as_ref()
        .and_then(|image| {
            presence_art(
                &image.url,
                POPOUT_ART,
                8.,
                ("profile-popout-art", card.user_id),
                cx,
            )
        })
        .or_else(|| {
            let path = game
                .application_id
                .and_then(|id| current_art(card, id, cx))?;
            Some(still_art(path, 8.))
        });
    let badge = game.small_image.as_ref().and_then(|image| {
        presence_art(
            &image.url,
            POPOUT_BADGE,
            POPOUT_BADGE,
            ("profile-popout-badge", card.user_id),
            cx,
        )
    });
    let elapsed = game
        .started_at
        .and_then(|started| SystemTime::now().duration_since(started).ok())
        .map(clock);
    let line = |id: &'static str, text: &String| {
        div()
            .truncate()
            .text_size(rpx(13.))
            .line_height(rpx(16.))
            .text_color(rgb(palette.strong))
            .child(label((id, card.user_id), text))
    };
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(8.))
            .p(rpx(8.))
            .rounded(rpx(8.))
            .bg(palette.panel)
            .child(
                playing_heading(game, palette)
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(12.))
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .size(rpx(POPOUT_ART))
                            .rounded(rpx(8.))
                            .map(|this| match art {
                                Some(art) => this.child(art),
                                None => this
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(palette.hover)
                                    .child(theme::icon("gamepad-2", 28., palette.muted)),
                            })
                            // Discord rings the small image in the card colour so it sits on
                            // the large one's corner.
                            .children(badge.map(|badge| {
                                div()
                                    .absolute()
                                    .right(rpx(-4.))
                                    .bottom(rpx(-4.))
                                    .size(rpx(POPOUT_BADGE + 4.))
                                    .p(rpx(2.))
                                    .rounded_full()
                                    .bg(palette.panel)
                                    .child(badge)
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(rpx(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rpx(16.))
                                    .line_height(rpx(20.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(palette.strong))
                                    .child(label(
                                        ("profile-popout-game", card.user_id),
                                        &game.name,
                                    )),
                            )
                            .children(
                                game.details
                                    .as_ref()
                                    .map(|details| line("profile-popout-details", details)),
                            )
                            .children(
                                game.state
                                    .as_ref()
                                    .map(|state| line("profile-popout-state", state)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_x(rpx(8.))
                                    .text_size(rpx(13.))
                                    .line_height(rpx(16.))
                                    .text_color(rgb(palette.text))
                                    .children(elapsed.map(|elapsed| {
                                        activity_badge(timer_icon(game), elapsed, theme::GREEN)
                                            .font_family(cx.theme().mono_font_family.clone())
                                    }))
                                    .children(entry.into_iter().flat_map(activity_traits).map(
                                        |(icon, text)| activity_badge(icon, text, palette.text),
                                    )),
                            ),
                    ),
            )
            .children(actions.buttons("profile-popout-playing-actions", card.user_id, palette))
            .children(play_button(game, palette).map(|play| play.w_full()))
            .children(activity_accessory(app, card, game, palette, cx))
            .into_any_element(),
    )
}

/// Discord's elapsed-time clock: "6:10", "1:02:03".
pub(super) fn clock(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn live(entry: &OutboxEntry, now: DateTime<Utc>) -> bool {
    // A lapsed session keeps its live trait; Discord dates it like any other entry.
    entry.traits.contains(&OutboxTrait::Live)
        && entry
            .expires_at
            .as_deref()
            .and_then(parse)
            .is_none_or(|expires| expires > now)
}

pub(super) fn current_entry<'a>(
    game: &GameActivity,
    entries: &'a [OutboxEntry],
    now: DateTime<Utc>,
) -> Option<&'a OutboxEntry> {
    entries
        .iter()
        .filter_map(|entry| {
            let OutboxKind::PlayedGame {
                name,
                application_id,
            } = &entry.kind
            else {
                return None;
            };
            let exact = match (game.application_id, *application_id) {
                (Some(expected), Some(actual)) if expected == actual => true,
                (Some(_), Some(_)) => return None,
                _ => {
                    if game.name.trim().is_empty()
                        || !game.name.trim().eq_ignore_ascii_case(name.trim())
                    {
                        return None;
                    }
                    false
                }
            };
            Some((entry, exact))
        })
        .max_by_key(|(entry, exact)| {
            (
                *exact,
                live(entry, now),
                entry
                    .ended_at
                    .as_deref()
                    .or(entry.started_at.as_deref())
                    .and_then(parse),
                entry.id,
            )
        })
        .map(|(entry, _)| entry)
}

pub(super) fn activity_traits(entry: &OutboxEntry) -> Vec<(&'static str, String)> {
    let mut meta = Vec::new();
    let started = entry.started_at.as_deref().and_then(parse);
    for activity_trait in &entry.traits {
        match activity_trait {
            OutboxTrait::Resurrected { last_played } => {
                if let (Some(started), Some(last)) = (started, parse(last_played)) {
                    meta.push((
                        "rotate-cw",
                        format!("Returning after {}", span(started - last)),
                    ));
                }
            }
            OutboxTrait::Duration { seconds } if *seconds >= MARATHON_SECONDS => {
                meta.push(("timer", format!("{}h Marathon", seconds / 3600)));
            }
            OutboxTrait::Streak { days } if *days >= 2 => {
                meta.push(("zap", format!("{days}x Streak")));
            }
            OutboxTrait::FirstTime => meta.push(("sprout", "New Player".to_owned())),
            OutboxTrait::Trending => meta.push(("trending-up", "Trending".to_owned())),
            _ => {}
        }
    }
    meta
}

fn activity_badge(icon: &str, text: String, color: u32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(4.))
        .text_color(rgb(color))
        .child(theme::icon(icon, 14., color))
        .child(text)
}

fn activity_row(
    card: &ProfileCard,
    entry: &OutboxEntry,
    now: DateTime<Utc>,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement {
    let live = live(entry, now);
    let art = activity_art(card, entry, cx);
    let (title, subtitle, kind_icon) = match &entry.kind {
        OutboxKind::PlayedGame { name, .. } | OutboxKind::LaunchedActivity { name, .. } => {
            (name.clone(), None, "gamepad-2")
        }
        OutboxKind::Listened { tracks, .. } => {
            let track: Option<&ListenedTrack> = tracks.first();
            (
                track.map_or_else(String::new, |track| track.title.clone()),
                track
                    .map(|track| track.artists.join(", "))
                    .filter(|artists| !artists.is_empty()),
                "music",
            )
        }
        OutboxKind::Unknown => (String::new(), None, "gamepad-2"),
    };
    let last_update = match &entry.kind {
        OutboxKind::Listened { last_update, .. }
        | OutboxKind::LaunchedActivity { last_update, .. } => last_update.as_deref(),
        _ => None,
    };
    let when = if live {
        "Now".to_owned()
    } else {
        entry
            .ended_at
            .as_deref()
            .or(last_update)
            .or(entry.started_at.as_deref())
            .and_then(parse)
            // Launched activities carry no timestamps at all; the entry id is still a snowflake.
            .or_else(|| snowflake_time(entry.id))
            .map_or_else(String::new, |at| format!("{} ago", span(now - at)))
    };
    let mut meta = vec![(kind_icon, when)];
    meta.extend(activity_traits(entry));
    div()
        .flex()
        .items_center()
        .gap(rpx(14.))
        .p(rpx(14.))
        .rounded(rpx(12.))
        .bg(palette.panel)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(ACTIVITY_ART))
                .rounded(rpx(ACTIVITY_RADIUS))
                // Transparent icons (NIKKE's wordmark) sit on the card itself, as in Discord.
                .map(|this| match art {
                    Some(path) => this.child(
                        img(path)
                            .size_full()
                            .rounded(rpx(ACTIVITY_RADIUS))
                            .object_fit(ObjectFit::Cover),
                    ),
                    None => {
                        this.bg(palette.hover)
                            .child(theme::icon(kind_icon, 28., palette.muted))
                    }
                }),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(2.))
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.strong))
                        .child(label(("profile-activity", entry.id), &title)),
                )
                .children(subtitle.map(|subtitle| {
                    div()
                        .truncate()
                        .text_size(rpx(13.))
                        .line_height(rpx(16.))
                        .text_color(rgb(palette.text))
                        .child(label(("profile-activity-artist", entry.id), &subtitle))
                }))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(rpx(10.))
                        .gap_y(rpx(2.))
                        .mt(rpx(2.))
                        .text_size(rpx(13.))
                        .line_height(rpx(16.))
                        .text_color(rgb(palette.text))
                        .children(
                            meta.into_iter()
                                .filter(|(_, text)| !text.is_empty())
                                .map(|(icon, text)| activity_badge(icon, text, palette.text)),
                        ),
                ),
        )
}

pub(super) fn snowflake_time(id: u64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(((id >> 22) + DISCORD_EPOCH_MS) as i64)
}

pub(super) fn parse(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Spans as Discord words them ("23h" reads "1d", 11 days "2w"): each unit rounds to the
/// nearest whole, and a unit takes over once the smaller one would reach its threshold.
pub(super) fn span(delta: chrono::TimeDelta) -> String {
    let seconds = delta.num_seconds().max(0) as f64;
    let minutes = (seconds / 60.).round() as i64;
    let hours = (seconds / 3600.).round() as i64;
    let days = (seconds / 86400.).round() as i64;
    let weeks = (days as f64 / 7.).round() as i64;
    let months = (seconds / (86400. * 30.)).round() as i64;
    match () {
        _ if minutes < 45 => format!("{}m", minutes.max(1)),
        _ if hours < 22 => format!("{}h", hours.max(1)),
        _ if days < 7 => format!("{}d", days.max(1)),
        _ if weeks < 4 => format!("{weeks}w"),
        _ if months < 11 => plural(months.max(1), "month"),
        _ => plural(((seconds / (86400. * 365.)).round() as i64).max(1), "year"),
    }
}

fn plural(count: i64, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}
