use super::people::people_row;
use super::*;

fn header_tab(
    id: &'static str,
    text: impl Into<SharedString>,
    selected: bool,
    badge: u32,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(6.))
        .h(rpx(28.))
        .px(rpx(10.))
        .rounded(rpx(8.))
        .text_size(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .text_color(rgb(if selected {
            theme::strong()
        } else {
            theme::muted()
        }))
        .when(selected, |this| this.bg(rgba(theme::selected())))
        .when(!selected, |this| {
            this.hover(|style| {
                style
                    .bg(rgba(theme::hover()))
                    .text_color(rgb(theme::text()))
            })
        })
        .child(text.into())
        .when(badge > 0, |this| this.child(count_pill(badge)))
}

pub(crate) fn requests_header(app: &DiscordApp) -> impl IntoElement + use<> {
    let tab = |id: &'static str, tab: RequestsTab, text: String| {
        let entity = app.entity.clone();
        header_tab(id, text, app.home.requests_tab == tab, 0).on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.select_requests_tab(tab, cx))
                .ok();
        })
    };
    div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .w_full()
        .h(rpx(chat::HEADER_HEIGHT))
        .flex_none()
        .px_4()
        .border_b_1()
        .border_color(rgba(theme::border()))
        .child(theme::icon("mail", 20., theme::faint()))
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Message Requests"),
        )
        .child(
            div()
                .size(rpx(4.))
                .mx(rpx(4.))
                .rounded_full()
                .bg(rgb(theme::faint())),
        )
        // Like Discord: no pill on Requests, and the spam count in the label.
        .child(tab(
            "requests-tab",
            RequestsTab::Requests,
            "Requests".to_owned(),
        ))
        .child(tab(
            "spam-tab",
            RequestsTab::Spam,
            match app.store.spam_request_count() {
                0 => "Spam".to_owned(),
                count => format!("Spam ({count})"),
            },
        ))
}

/// Accepting or ignoring a request can trip Discord's captcha like friend requests do, so both
/// stay in the official client for now. Flip this once captchas are handled.
const REQUEST_ACTIONS: bool = false;
const ACCEPT_FROM_DISCORD: &str = "Accept message requests from Discord for now";
const IGNORE_FROM_DISCORD: &str = "Ignore message requests from Discord for now";
/// Discord's message request avatar (`SIZE_40`).
const REQUEST_AVATAR: f32 = 40.;
const MUTUAL_ICON: f32 = 16.;
/// Height of the request row's second line, which slides between mutual servers and preview.
const REQUEST_LINE: f32 = 18.;
/// How far each stacked server icon sits from the previous one.
const MUTUAL_STEP: f32 = 12.;
/// Discord's `LIST_ROW_HEIGHT`: a 1px top border over a 72px row.
const REQUEST_ROW_HEIGHT: f32 = 73.;

pub(crate) fn requests_page(app: &DiscordApp, _cx: &App) -> AnyElement {
    let spam = app.home.requests_tab == RequestsTab::Spam;
    let count = if spam {
        app.store.spam_request_count()
    } else {
        app.store.message_request_count()
    } as usize;
    let heading = format!(
        "{} — {count}",
        if spam {
            "Spam Requests"
        } else {
            "Pending Requests"
        }
    );
    let title = div()
        .flex_none()
        .ml(rpx(10.))
        .mb(rpx(10.))
        .text_size(rpx(14.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::muted()))
        .child(heading);
    let page = div()
        .flex()
        .flex_col()
        .size_full()
        .px(rpx(24.))
        .pt(rpx(24.))
        .child(title);
    if count == 0 {
        return page
            .child(
                div()
                    .mt(rpx(40.))
                    .text_center()
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::muted()))
                    .child(if spam {
                        "There are no spam requests."
                    } else {
                        "There are no pending message requests."
                    }),
            )
            .into_any_element();
    }
    let entity = app.entity.clone();
    let now_ms = chrono::Utc::now().timestamp_millis();
    // Virtualized like Discord's list: only rows on screen are built, and only they fetch.
    let list = uniform_list("requests-list", count, move |range, _, cx| {
        let Some(app_entity) = entity.upgrade() else {
            return Vec::new();
        };
        let (items, missing, live, app_weak) = {
            let cx: &App = cx;
            let app = app_entity.read(cx);
            let requests = if app.home.requests_tab == RequestsTab::Spam {
                app.store.spam_requests()
            } else {
                app.store.message_requests()
            };
            let hovered = app.home.hovered_row;
            let mut missing = Vec::new();
            let items: Vec<AnyElement> = range
                .filter_map(|index| {
                    let dm = *requests.get(index)?;
                    let id = dm.channel.id;
                    let user_id = (!dm.group)
                        .then(|| dm.recipient_ids.first().copied())
                        .flatten();
                    if !app.home.previews_requested.contains(&id)
                        || user_id.is_some_and(|user| !app.home.mutuals_requested.contains(&user))
                    {
                        missing.push((id, user_id));
                    }
                    // Discord's border sits on top of every row but the first, and hides beside a
                    // hovered row.
                    let above = index.checked_sub(1).and_then(|above| requests.get(above));
                    let border = above.is_some_and(|above| {
                        let lit = |row: u64| hovered == Some(row) || app.thread_pane_shows(row);
                        !lit(id) && !lit(above.channel.id)
                    });
                    Some(
                        // Virtualized items size to their content unless told to fill the row.
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .h(rpx(REQUEST_ROW_HEIGHT))
                            .child(
                                div()
                                    .flex_none()
                                    .h(rpx(1.))
                                    .when(border, |this| this.bg(rgba(theme::border_subtle()))),
                            )
                            .child(
                                request_row(app, dm, now_ms, cx)
                                    .w_full()
                                    .h(rpx(REQUEST_ROW_HEIGHT - 1.))
                                    .py(rpx(0.)),
                            )
                            .into_any_element(),
                    )
                })
                .collect();
            (items, missing, app.live, app.entity.clone())
        };
        // Fetching mutates the app, which can't happen mid-paint, so it waits for the frame.
        if live && !missing.is_empty() {
            cx.defer(move |cx| {
                app_weak
                    .update(cx, |app, cx| app.load_visible_requests(missing, cx))
                    .ok();
            });
        }
        items
    })
    .track_scroll(&app.home.requests_scroll)
    .flex_1()
    .min_h_0()
    .pb(rpx(24.));
    page.child(list).into_any_element()
}

/// Discord's request age: minutes, hours and days up to 30, then ">30d ago" until 360 days,
/// then the full date.
pub(super) fn request_age(sent_ms: i64, now_ms: i64) -> String {
    let seconds = (now_ms - sent_ms).max(0) / 1000;
    match seconds {
        0..60 => "1m ago".to_owned(),
        60..3_600 => format!("{}m ago", seconds / 60),
        3_600..86_400 => format!("{}h ago", seconds / 3_600),
        86_400..2_592_000 => format!("{}d ago", seconds / 86_400),
        2_592_000..31_104_000 => ">30d ago".to_owned(),
        _ => chrono::DateTime::from_timestamp_millis(sent_ms)
            .map(|time| {
                time.with_timezone(&chrono::Local)
                    .format("%B %-d, %Y")
                    .to_string()
            })
            .unwrap_or_default(),
    }
}

/// One line of the latest message, as Discord previews it: line breaks become spaces, and
/// user mentions and custom emoji read as names rather than raw markup.
pub(super) fn request_preview_line(store: &Store, content: &str) -> String {
    let flat = content.replace(['\r', '\n'], " ");
    let mut out = String::with_capacity(flat.len());
    let mut rest = flat.as_str();
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find('>') else {
            out.push_str(tail);
            rest = "";
            break;
        };
        let inner = &tail[1..end];
        let mention = inner
            .strip_prefix("@!")
            .or_else(|| inner.strip_prefix('@'))
            .and_then(|id| id.parse::<u64>().ok());
        let emoji = inner
            .strip_prefix("a:")
            .or_else(|| inner.strip_prefix(':'))
            .and_then(|body| body.split_once(':'))
            .map(|(name, _)| name);
        match (mention, emoji) {
            (Some(id), _) => {
                out.push('@');
                out.push_str(&store.friend_name(id));
            }
            (None, Some(name)) => {
                out.push(':');
                out.push_str(name);
                out.push(':');
            }
            (None, None) => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RequestButton {
    Ignore,
    Accept,
    NotSpam,
    Report,
}

/// Discord's small request buttons. Each is muted with a tooltip while `REQUEST_ACTIONS` is
/// off; Report has no client support yet, so it always is.
pub(super) fn request_button(app: &DiscordApp, id: u64, kind: RequestButton) -> AnyElement {
    let (key, text, tip, background, hover) = match kind {
        RequestButton::Ignore => (
            "request-ignore",
            "Ignore",
            IGNORE_FROM_DISCORD,
            rgba(theme::control()),
            rgba(theme::hover()),
        ),
        RequestButton::Accept => (
            "request-accept",
            "Accept DM",
            ACCEPT_FROM_DISCORD,
            rgb(theme::BLURPLE),
            rgb(theme::BLURPLE_ACTIVE),
        ),
        RequestButton::NotSpam => (
            "request-not-spam",
            "Not Spam",
            "Mark requests as not spam from Discord for now",
            rgba(theme::control()),
            rgba(theme::hover()),
        ),
        RequestButton::Report => (
            "request-report",
            "Report Spam",
            "Report spam from Discord for now",
            rgb(theme::NOTIFICATION),
            rgb(theme::NOTIFICATION),
        ),
    };
    let live = REQUEST_ACTIONS && kind != RequestButton::Report;
    let entity = app.entity.clone();
    let button = div()
        .id((key, id))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(rpx(32.))
        .px(rpx(16.))
        .rounded(rpx(8.))
        .bg(background)
        .text_size(rpx(14.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(
            if kind == RequestButton::Ignore || kind == RequestButton::NotSpam {
                rgb(theme::bright()).into()
            } else {
                white()
            },
        )
        .child(text)
        .when(live, move |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(hover))
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    entity
                        .update(cx, |app, cx| match kind {
                            RequestButton::Ignore => app.ignore_request(id, window, cx),
                            // Discord's "not spam" accepts the request.
                            RequestButton::Accept | RequestButton::NotSpam => {
                                app.accept_request(id, window, cx)
                            }
                            RequestButton::Report => {}
                        })
                        .ok();
                })
        });
    if live {
        return button.into_any_element();
    }
    let tip_key = (tip, id);
    div()
        .id((key, id.wrapping_add(1)))
        .relative()
        .flex_none()
        .child(div().opacity(0.5).child(button))
        .child(theme::hover_tooltip(tip_key, tip))
        // Keeps the row's click (open DM) from firing through the button.
        .on_click(|_, _, cx| cx.stop_propagation())
        .into_any_element()
}

/// Up to three server icons and the count, like Discord's "3 Mutual Servers".
pub(super) fn mutual_servers(app: &DiscordApp, guild_ids: &[u64], cx: &App) -> Div {
    let label = match guild_ids.len() {
        0 => "No Mutual Servers".to_owned(),
        1 => "1 Mutual Server".to_owned(),
        count => format!("{count} Mutual Servers"),
    };
    let shown = guild_ids.len().min(3);
    let size = MUTUAL_ICON + 2.;
    // Absolute positions inside a fixed-width box: the overlap is drawn, not laid out, so the
    // label always starts after the last icon.
    let icons = guild_ids
        .iter()
        .take(shown)
        .enumerate()
        .map(|(index, guild_id)| {
            let image = app
                .store
                .guild_by_id(*guild_id)
                .and_then(|guild| guild.icon_spec())
                .and_then(|spec| images::resolve(&spec, theme::image_px(MUTUAL_ICON), cx));
            div()
                .absolute()
                .top_0()
                .left(rpx(index as f32 * MUTUAL_STEP))
                .size(rpx(size))
                .rounded(rpx(5.))
                .border_1()
                .border_color(rgb(theme::bg()))
                .bg(rgba(theme::selected()))
                .overflow_hidden()
                .children(image.map(|path| img(path).size_full().object_fit(ObjectFit::Cover)))
        });
    let width = (shown > 0).then(|| size + (shown - 1) as f32 * MUTUAL_STEP);
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .min_w_0()
        .children(width.map(|width| {
            div()
                .relative()
                .flex_none()
                .w(rpx(width))
                .h(rpx(size))
                .children(icons)
        }))
        .child(div().min_w_0().truncate().child(label))
}

fn request_row(app: &DiscordApp, dm: &PrivateChannel, now_ms: i64, cx: &App) -> Stateful<Div> {
    let id = dm.channel.id;
    let user_id = dm.recipient_ids.first().copied().unwrap_or(0);
    let user = app.store.user(user_id);
    let name = app.store.private_channel_name(id);
    let username = user
        .filter(|user| !user.username.is_empty() && user.username != name && user.name != name)
        .map(|user| crate::settings::shown_username(&user.username));
    // Discord dates the row by the preview message, falling back to the channel's last message.
    let sent_id = Some(app.store.latest_message_id(id))
        .filter(|id| *id != 0)
        .unwrap_or(0);
    let age = (sent_id != 0).then(|| {
        let sent_ms = (sent_id >> 22).saturating_add(crate::app::DISCORD_EPOCH_MS) as i64;
        request_age(sent_ms, now_ms)
    });
    let hovered = app.home.hovered_row == Some(id);
    let content = app
        .store
        .request_preview(id)
        .map(|preview| request_preview_line(&app.store, &preview.content))
        .filter(|content| !content.trim().is_empty())
        .unwrap_or_else(|| "Sent a message".to_owned());
    let line = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .min_w_0()
            .h(rpx(REQUEST_LINE))
    };
    // Both lines stack in a one-line window; hovering slides the stack up so the latest
    // message replaces the mutual servers, like Discord's row and our account panel.
    let lines = div()
        .absolute()
        .left_0()
        .right_0()
        .flex()
        .flex_col()
        .child(
            line().children(
                app.home
                    .mutuals
                    .get(&user_id)
                    .map(|guild_ids| mutual_servers(app, guild_ids, cx)),
            ),
        )
        .child(
            line().child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(label(("request-preview", id), &content)),
            ),
        );
    // Only the row being entered or left animates, so rows never slide back on first paint.
    let animate = !cx.reduce_motion() && (hovered || app.home.left_row == Some(id));
    let second_line = if animate {
        lines
            .with_animation(
                // A new key per direction restarts the slide; snowflakes fit in 63 bits.
                ("request-line", (id << 1) | u64::from(hovered)),
                Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint()),
                move |this, delta| {
                    let shown = if hovered { delta } else { 1. - delta };
                    this.top(rpx(-REQUEST_LINE * shown))
                },
            )
            .into_any_element()
    } else {
        lines
            .top(rpx(if hovered { -REQUEST_LINE } else { 0. }))
            .into_any_element()
    };
    let entity = app.entity.clone();
    let active = app.thread_pane_shows(id);
    people_row(app, ("request", id).into(), id)
        .ml(rpx(0.))
        .mr(rpx(0.))
        .cursor_pointer()
        // Discord's `.active` row: the one open in the sidebar keeps the hover fill.
        .when(active, |this| this.bg(rgba(theme::hover())))
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| app.open_thread_pane(id, window, cx))
                .ok();
        })
        .child(theme::user_avatar(
            user,
            user_id,
            REQUEST_AVATAR,
            None,
            false,
            cx,
        ))
        .child(
            // Discord's `.userPreview`: 16px clear of the buttons.
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .mr(rpx(16.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .min_w_0()
                        .child(
                            div()
                                .flex_none()
                                .max_w(relative(0.6))
                                .truncate()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::strong()))
                                .child(label(("request-name", id), &name)),
                        )
                        .children(username.map(|username| {
                            div()
                                .min_w_0()
                                .ml(rpx(4.))
                                .truncate()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .text_color(rgb(theme::muted()))
                                .child(username)
                        }))
                        .children(age.map(|age| {
                            // `.timestampWithPreview`: 8px after the name, body-xs muted.
                            div()
                                .flex_none()
                                .ml(rpx(8.))
                                .text_size(rpx(12.))
                                .line_height(rpx(16.))
                                .text_color(rgb(theme::muted()))
                                .child(age)
                        })),
                )
                .child(
                    div()
                        .relative()
                        .min_w_0()
                        .h(rpx(REQUEST_LINE))
                        .mt(rpx(2.))
                        .overflow_hidden()
                        .text_size(rpx(14.))
                        .line_height(rpx(REQUEST_LINE))
                        .text_color(rgb(theme::muted()))
                        .child(second_line),
                ),
        )
        .child(div().flex().flex_none().gap(rpx(16.)).map(|this| {
            if dm.spam {
                this.child(request_button(app, id, RequestButton::NotSpam))
                    .child(request_button(app, id, RequestButton::Report))
            } else {
                this.child(request_button(app, id, RequestButton::Ignore))
                    .child(request_button(app, id, RequestButton::Accept))
            }
        }))
}

/// Replaces the composer while an unaccepted message request is open, in the main view or
/// the side pane.
pub(crate) fn request_bar(app: &DiscordApp, id: u64) -> Option<AnyElement> {
    let dm = app
        .store
        .private_channel(id)
        .filter(|dm| dm.message_request)?;
    let name = app.store.private_channel_name(id);
    let button =
        |key: &'static str, text: &'static str, background: Rgba, hover: Rgba, color: Hsla| {
            div()
                .id(key)
                .flex()
                .items_center()
                .justify_center()
                .h(rpx(32.))
                .px(rpx(16.))
                .rounded(rpx(8.))
                .bg(background)
                .when(REQUEST_ACTIONS, move |this| {
                    this.hover(move |style| style.bg(hover)).cursor_pointer()
                })
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(text)
        };
    let accept = {
        let entity = app.entity.clone();
        button(
            "request-bar-accept",
            "Accept",
            rgb(theme::BLURPLE),
            rgb(theme::BLURPLE),
            white(),
        )
        .when(REQUEST_ACTIONS, move |this| {
            this.on_click(move |_, window, cx| {
                entity
                    .update(cx, |app, cx| app.accept_request(id, window, cx))
                    .ok();
            })
        })
    };
    let ignore = {
        let entity = app.entity.clone();
        button(
            "request-bar-ignore",
            "Ignore",
            rgba(theme::control()),
            rgba(theme::hover()),
            rgb(theme::bright()).into(),
        )
        .when(REQUEST_ACTIONS, move |this| {
            this.on_click(move |_, window, cx| {
                entity
                    .update(cx, |app, cx| app.ignore_request(id, window, cx))
                    .ok();
            })
        })
    };
    // While request actions are off, each button dims, drops its click, and explains itself.
    let gate = |button: Stateful<Div>, key: &'static str, tip: &'static str| -> AnyElement {
        if REQUEST_ACTIONS {
            return button.into_any_element();
        }
        div()
            .id(key)
            .relative()
            .flex_none()
            .child(div().opacity(0.5).child(button))
            .child(theme::hover_tooltip(key, tip))
            .into_any_element()
    };
    let accept = gate(accept, "request-bar-accept-tip", ACCEPT_FROM_DISCORD);
    let ignore = gate(ignore, "request-bar-ignore-tip", IGNORE_FROM_DISCORD);
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(12.))
            .mx_4()
            .mb(rpx(24.))
            .p(rpx(16.))
            .rounded(rpx(8.))
            .bg(rgb(theme::elevated()))
            .border_1()
            .border_color(rgba(theme::border()))
            .child(
                div()
                    .text_size(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .child(format!("{name} wants to message you")),
            )
            .child(
                div()
                    .text_size(rpx(14.))
                    .text_color(rgb(theme::muted()))
                    .child(if dm.spam {
                        "Discord flagged this as possible spam. Ignoring removes it."
                    } else {
                        "Accept to move this conversation into your Direct Messages."
                    }),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(rpx(8.))
                    .child(ignore)
                    .child(accept),
            )
            .into_any_element(),
    )
}

pub(crate) fn blocked_bar(app: &DiscordApp, id: u64) -> Option<AnyElement> {
    let user_id = app
        .store
        .private_channel(id)
        .filter(|dm| !dm.group)?
        .recipient_ids
        .first()
        .copied()?;
    app.store
        .relationship(user_id)
        .filter(|relationship| relationship.kind == RelationshipKind::Blocked)?;
    let entity = app.entity.clone();
    Some(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(rpx(12.))
            .mx_4()
            .mb(rpx(24.))
            .px(rpx(16.))
            .py(rpx(12.))
            .rounded(rpx(8.))
            .bg(rgb(theme::elevated()))
            .border_1()
            .border_color(rgba(theme::border()))
            .child(
                div()
                    .min_w_0()
                    .text_size(rpx(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme::strong()))
                    .child("You cannot send messages to a user you have blocked."),
            )
            .child(
                div()
                    .id("blocked-bar-unblock")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(rpx(32.))
                    .px(rpx(12.))
                    .rounded(rpx(8.))
                    .bg(rgba(theme::control()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .when(app.live, |this| {
                        this.cursor_pointer()
                            .hover(|style| style.bg(rgba(theme::hover())))
                            .on_click(move |_, _, cx| {
                                entity
                                    .update(cx, |app, cx| app.unblock_user(user_id, cx))
                                    .ok();
                            })
                    })
                    .when(!app.live, |this| this.opacity(0.5))
                    .text_size(rpx(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::bright()))
                    .without_text_selection()
                    .child("Unblock"),
            )
            .into_any_element(),
    )
}
