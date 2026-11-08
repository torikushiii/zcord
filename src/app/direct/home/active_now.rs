use super::*;

const ACTIVE_NOW_WIDTH: f32 = 420.;
const ACTIVE_ICON: f32 = 24.;
pub(super) const ACTIVE_ART: f32 = 32.;
pub(super) const ACTIVE_BADGE: f32 = 16.;
const ACTIVE_CARD_GROUP: &str = "active-card";
const ACTIVE_PARTY_AVATAR: f32 = 24.;
const SPOTIFY_GREEN: u32 = 0x1DB954;
const TWITCH_PURPLE: u32 = 0x9146FF;
const YOUTUBE_RED: u32 = 0xFF0000;
const TWITCH_PREVIEW_WIDTH: f32 = 354.;
const TWITCH_PREVIEW_HEIGHT: f32 = 197.;
const TWITCH_PREVIEW_REFRESH: u64 = 300;

pub(super) fn elapsed(duration: Duration) -> String {
    let minutes = duration.as_secs() / 60;
    if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h", minutes / 60)
    }
}

pub(super) fn activity_elapsed(started: Option<SystemTime>, now: SystemTime) -> Option<Duration> {
    now.duration_since(started?).ok()
}

pub(super) fn shows_elapsed_line(game: &GameActivity) -> bool {
    game.started_at.is_some() && (game.details.is_some() || game.state.is_some())
}

pub(crate) fn needs_active_now_clock(app: &DiscordApp) -> bool {
    app.showing_home_page()
        && app.home.page == HomePage::Friends
        && app.store.active_now().into_iter().any(|id| {
            app.store
                .user(id)
                .and_then(|user| user.game.as_ref())
                .is_some_and(shows_elapsed_line)
        })
}

pub(super) fn active_art(image: AnyElement, badge: Option<AnyElement>) -> Div {
    div()
        .relative()
        .flex_none()
        .size(rpx(ACTIVE_ART))
        .rounded(rpx(ACTIVE_ART / 5.))
        .bg(rgba(theme::selected()))
        .child(image)
        .children(badge.map(|badge| {
            div()
                .absolute()
                .right(rpx(-4.))
                .bottom(rpx(-4.))
                .size(rpx(ACTIVE_BADGE + 4.))
                .p(rpx(2.))
                .rounded_full()
                .bg(rgb(theme::bg()))
                .child(badge)
        }))
}

pub(super) fn active_detail_text(
    id: u64,
    detail: &str,
    state: Option<&str>,
    duration: Option<Duration>,
) -> Div {
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
                .child(label(("active-detail", id), detail)),
        )
        .children(state.map(|state| {
            div()
                .truncate()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child(label(("active-state", id), state))
        }))
        .children(duration.map(|duration| {
            div()
                .debug_selector(|| "active-elapsed".into())
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child(format!("{} elapsed", voice::format_elapsed(duration)))
        }))
}

fn active_header(
    app: &DiscordApp,
    id: u64,
    user: &User,
    headline: &str,
    icon: Div,
    cx: &App,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .p(rpx(16.))
        .bg(rgb(theme::elevated()))
        .group_hover(ACTIVE_CARD_GROUP, |style| {
            style.bg(rgb(theme::composite(theme::hover(), theme::elevated())))
        })
        .child(theme::user_avatar(
            Some(user),
            id,
            DM_AVATAR,
            Some((user.status, theme::elevated())),
            false,
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
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child(display_name::render(
                            ("active-name", id),
                            &app.store.friend_name(id),
                            user.name_style.as_ref(),
                            Surface::FontOnly,
                            cx,
                        )),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::muted()))
                        .child(label(("active-game", id), headline)),
                ),
        )
        .child(icon)
}

fn active_card_frame() -> Div {
    div()
        .group(ACTIVE_CARD_GROUP)
        .flex()
        .flex_col()
        .flex_none()
        .rounded(rpx(12.))
        .border_1()
        .border_color(rgba(theme::border()))
        .overflow_hidden()
        .hover(|style| style.bg(rgba(theme::hover())))
}

fn service_icon(service: &str, size: f32) -> Div {
    if service == "Twitch" {
        // Icons take one color, so the white panel of the logo is its own layer.
        return div()
            .relative()
            .flex_none()
            .size(rpx(size))
            .child(
                theme::icon("connections/twitch-fill", size, 0xFFFFFF)
                    .absolute()
                    .top_0()
                    .left_0(),
            )
            .child(
                theme::icon("connections/twitch", size, TWITCH_PURPLE)
                    .absolute()
                    .top_0()
                    .left_0(),
            );
    }
    let (icon, color) = match service {
        "YouTube" => ("connections/youtube", YOUTUBE_RED),
        _ => ("monitor-up", theme::muted()),
    };
    div()
        .flex_none()
        .size(rpx(size))
        .child(theme::icon(icon, size, color))
}

/// Twitch rebuilds this image every few minutes, so the bucket keeps it from being cached forever.
fn twitch_preview(login: &str, cx: &App) -> Option<Arc<std::path::Path>> {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let bucket = now / TWITCH_PREVIEW_REFRESH;
    let url = format!(
        "https://static-cdn.jtvnw.net/previews-ttv/live_user_{login}-900x500.jpg?t={bucket}"
    );
    images::resolve(
        &ImageSpec::Media {
            url,
            width: 0,
            height: 0,
            animated: false,
        },
        theme::image_px(TWITCH_PREVIEW_WIDTH),
        cx,
    )
}

fn live_stream_preview(id: u64, stream: &LiveStream, cx: &App) -> Div {
    let image = stream
        .twitch_login
        .as_deref()
        .and_then(|login| twitch_preview(login, cx));
    let url = stream.url.clone();
    div().child(
        div()
            .id(("active-stream-preview", id))
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(rpx(TWITCH_PREVIEW_HEIGHT))
            .rounded(rpx(8.))
            .overflow_hidden()
            .bg(rgba(theme::selected()))
            .when_some(url, |this, url| {
                this.cursor_pointer()
                    .on_click(move |_, _, cx| cx.open_url(&url))
            })
            .children(image.map(|path| {
                img(path)
                    .absolute()
                    .inset_0()
                    .size_full()
                    .rounded(rpx(8.))
                    .object_fit(ObjectFit::Cover)
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(rpx(48.))
                    .rounded_full()
                    .bg(rgba(0x00000099))
                    .child(theme::icon("video-play", 24., 0xFFFFFF)),
            ),
    )
}

fn live_stream_card(
    app: &DiscordApp,
    id: u64,
    user: &User,
    stream: &LiveStream,
    cx: &App,
) -> AnyElement {
    let headline = format!("Streaming on {}", stream.service);
    let preview = stream
        .twitch_login
        .is_some()
        .then(|| live_stream_preview(id, stream, cx));
    active_card_frame()
        .child(active_header(
            app,
            id,
            user,
            &headline,
            service_icon(&stream.service, ACTIVE_ICON),
            cx,
        ))
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
                        .gap(rpx(12.))
                        .child(service_icon(&stream.service, ACTIVE_ART))
                        .child(active_detail_text(
                            id,
                            &stream.service,
                            stream.title.as_deref(),
                            None,
                        )),
                )
                .children(preview),
        )
        .into_any_element()
}

fn active_card(
    app: &DiscordApp,
    id: u64,
    user: &User,
    game: &GameActivity,
    cx: &App,
) -> AnyElement {
    let listening = user.game.is_none();
    let duration = activity_elapsed(game.started_at, SystemTime::now());
    let headline = if listening {
        format!("Listening to {}", game.name)
    } else {
        match duration {
            Some(duration) => format!("{} – {}", game.name, elapsed(duration)),
            None => game.name.clone(),
        }
    };
    let app_icon = |size: f32| {
        let spec = app.game_icon_spec(game.application_id?)?;
        let path = images::resolve(&spec, theme::image_px(size), cx)?;
        Some(
            img(path)
                .size_full()
                .rounded(rpx(size / 5.))
                .into_any_element(),
        )
    };
    let presence = |size: f32, key: &'static str| {
        let image = game.large_image.as_ref()?;
        presence_art(&image.url, size, size / 5., (key, id), cx)
    };
    let fallback = |size: f32| {
        div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .rounded(rpx(size / 5.))
            .child(theme::icon("gamepad-2", size * 0.55, theme::muted()))
            .into_any_element()
    };
    // `presence_art` and the app icon fill their parent, so each needs a sized box.
    let icon =
        div()
            .flex_none()
            .size(rpx(ACTIVE_ICON))
            .child(if listening && game.name == "Spotify" {
                theme::icon("connections/spotify", ACTIVE_ICON, SPOTIFY_GREEN).into_any_element()
            } else {
                app_icon(ACTIVE_ICON)
                    .or_else(|| presence(ACTIVE_ICON, "active-icon"))
                    .unwrap_or_else(|| fallback(ACTIVE_ICON))
            });
    let badge = game.small_image.as_ref().and_then(|image| {
        presence_art(
            &image.url,
            ACTIVE_BADGE,
            ACTIVE_BADGE,
            ("active-badge", id),
            cx,
        )
    });
    let art = active_art(
        presence(ACTIVE_ART, "active-art")
            .or_else(|| app_icon(ACTIVE_ART))
            .unwrap_or_else(|| fallback(ACTIVE_ART)),
        badge,
    );
    let detail = game.details.clone().or_else(|| game.state.clone());
    let state = game.details.as_ref().and(game.state.clone());
    active_card_frame()
        .child(active_header(app, id, user, &headline, icon, cx))
        .child(match detail {
            Some(detail) => div()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .p(rpx(16.))
                .child(art)
                .child(active_detail_text(
                    id,
                    &detail,
                    state.as_deref(),
                    duration.filter(|_| !listening),
                )),
            None if listening => div(),
            None => div()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .p(rpx(16.))
                .child(art)
                .child(active_detail_text(id, &game.name, Some("1 Person"), None))
                .child(theme::undecorated_member_avatar(
                    &app.store,
                    0,
                    id,
                    ACTIVE_PARTY_AVATAR,
                    cx,
                )),
        })
        .into_any_element()
}

pub(super) fn voice_title(names: &[String], members: usize) -> String {
    let others = members.saturating_sub(names.len());
    let others_label =
        |count: usize| format!("{count} {}", if count == 1 { "other" } else { "others" });
    match (names, others) {
        ([], _) => others_label(others),
        ([a], 0) => a.clone(),
        ([a], _) => format!("{a} and {}", others_label(others)),
        ([a, b], 0) => format!("{a} and {b}"),
        ([a, b, ..], _) => format!("{a}, {b}, and {}", others_label(others)),
    }
}

fn voice_card(app: &DiscordApp, voice: &ActiveVoice, cx: &App) -> Option<AnyElement> {
    let lead_id = *voice.friend_ids.first()?;
    let lead = app.store.user(lead_id)?;
    let names: Vec<String> = voice
        .friend_ids
        .iter()
        .filter(|id| **id != app.store.current_user_id)
        .take(2)
        .map(|id| app.store.friend_name(*id))
        .collect();
    let guild = app
        .store
        .guilds
        .iter()
        .find(|guild| guild.id == voice.guild_id)?;
    let channel = app
        .store
        .channel_name(voice.channel_id)
        .unwrap_or_default()
        .to_owned();
    let icon = guild
        .icon_spec()
        .and_then(|spec| images::resolve(&spec, theme::image_px(ACTIVE_ART), cx))
        .map(|path| img(path).size_full().rounded_full().into_any_element())
        .unwrap_or_else(|| {
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .rounded_full()
                .bg(rgb(guild.icon_color))
                .text_size(rpx(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(white())
                .child(guild.initials.clone())
                .into_any_element()
        });
    let art = div()
        .relative()
        .flex_none()
        .size(rpx(ACTIVE_ART))
        .child(icon)
        .child(
            div()
                .absolute()
                .left(rpx(18.))
                .top(rpx(18.))
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(16.))
                .rounded_full()
                .bg(rgb(theme::bg()))
                .child(theme::icon("volume-2", 10., theme::text())),
        );
    let shown = voice.member_ids.len().min(3);
    let extra = voice.member_ids.len() - shown;
    let party = div()
        .flex()
        .flex_none()
        .items_center()
        .children(
            voice
                .member_ids
                .iter()
                .take(shown)
                .enumerate()
                .map(|(index, id)| {
                    div().when(index > 0, |this| this.ml(rpx(-2.))).child(
                        theme::undecorated_member_avatar(
                            &app.store,
                            0,
                            *id,
                            ACTIVE_PARTY_AVATAR,
                            cx,
                        ),
                    )
                }),
        )
        .when(extra > 0, |this| {
            this.child(
                div()
                    .ml(rpx(4.))
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(theme::faint()))
                    .child(format!("+{extra}")),
            )
        });
    let header = div()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .p(rpx(16.))
        .bg(rgb(theme::elevated()))
        .group_hover(ACTIVE_CARD_GROUP, |style| {
            style.bg(rgb(theme::composite(theme::hover(), theme::elevated())))
        })
        .child(theme::user_avatar(
            Some(lead),
            lead_id,
            DM_AVATAR,
            Some((lead.status, theme::elevated())),
            false,
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
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::text()))
                        .child(voice_title(&names, voice.member_ids.len())),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::faint()))
                        .child("In a Voice Channel"),
                ),
        );
    let body = div()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .p(rpx(16.))
        .child(art)
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
                        .text_color(rgb(theme::text()))
                        .child(guild.name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .text_color(rgb(theme::faint()))
                        .child(channel),
                ),
        )
        .child(party);
    Some(
        active_card_frame()
            .child(header)
            .child(body)
            .into_any_element(),
    )
}

pub(super) fn active_now(app: &DiscordApp, cx: &App) -> impl IntoElement + use<> {
    let voice_cards = app
        .store
        .active_voice()
        .iter()
        .filter_map(|voice| voice_card(app, voice, cx))
        .collect::<Vec<_>>();
    let cards: Vec<AnyElement> = voice_cards
        .into_iter()
        .chain(app.store.active_now().into_iter().filter_map(|id| {
            let user = app.store.user(id)?;
            if let Some(stream) = &user.live_stream {
                return Some(live_stream_card(app, id, user, stream, cx));
            }
            let activity = user.game.as_ref().or(user.listening.as_ref())?;
            Some(active_card(app, id, user, activity, cx))
        }))
        .collect();
    let empty = cards.is_empty();
    div()
        .id("active-now")
        .flex()
        .flex_col()
        .flex_none()
        .w(rpx(ACTIVE_NOW_WIDTH))
        .h_full()
        .gap(rpx(16.))
        .p(rpx(16.))
        .overflow_y_scrollbar()
        .border_l_1()
        .border_color(rgba(theme::border()))
        .child(
            div()
                .text_size(rpx(20.))
                .line_height(rpx(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Active Now"),
        )
        .children(cards)
        .when(empty, |this| {
            this.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .mt_4()
                    .text_center()
                    .child(
                        div()
                            .text_size(rpx(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme::strong()))
                            .child("It's quiet for now..."),
                    )
                    .child(
                        div()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::muted()))
                            .child(
                                "When a friend starts an activity, like playing a game, we'll show it here!",
                            ),
                    ),
            )
        })
}
