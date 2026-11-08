use super::active_now::active_now;
use super::*;

const ACTION_SIZE: f32 = 36.;

pub(crate) fn friends_header(app: &DiscordApp) -> impl IntoElement + use<> {
    let tab = |tab: FriendsTab, badge: u32| {
        let selected = app.home.tab == tab;
        let entity = app.entity.clone();
        div()
            .id(tab.label())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .min_w(rpx(40.))
            .gap(rpx(8.))
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
            .child(tab.label())
            .when(badge > 0, |this| this.child(count_pill(badge)))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.select_friends_tab(tab, cx))
                    .ok();
            })
    };
    let suggestions = app.store.friend_suggestion_count();
    let adding = app.home.tab == FriendsTab::Add;
    let open_add = app.entity.clone();
    let add_tab = div()
        .id("friends-add-tab")
        .flex()
        .flex_none()
        .items_center()
        .h(rpx(28.))
        .px(rpx(10.))
        .rounded(rpx(8.))
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .text_color(rgb(if adding { theme::BLURPLE } else { 0xFFFFFF }))
        .when(adding, |this| this.bg(rgba(theme::selected())))
        .when(!adding, |this| {
            this.bg(rgb(theme::BLURPLE))
                .hover(|style| style.bg(rgb(theme::BLURPLE_ACTIVE)))
        })
        .child(FriendsTab::Add.label())
        .on_click(move |_, _, cx| {
            open_add
                .update(cx, |app, cx| app.select_friends_tab(FriendsTab::Add, cx))
                .ok();
        });
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
        .child(theme::icon("users", 20., theme::faint()))
        .child(
            div()
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Friends"),
        )
        .child(
            div()
                .size(rpx(4.))
                .mx(rpx(4.))
                .rounded_full()
                .bg(rgb(theme::faint())),
        )
        .child(tab(FriendsTab::Online, 0))
        .child(tab(FriendsTab::All, 0))
        .child(tab(FriendsTab::Pending, app.store.pending_count()))
        .when(
            suggestions > 0 || app.home.tab == FriendsTab::Suggestions,
            |this| this.child(tab(FriendsTab::Suggestions, suggestions)),
        )
        .child(add_tab)
}

fn search_query(app: &DiscordApp, cx: &App) -> String {
    app.home
        .search
        .as_ref()
        .map(|search| search.read(cx).value().trim().to_lowercase())
        .unwrap_or_default()
}

fn status_text(user: Option<&User>, kind: RelationshipKind) -> String {
    match kind {
        RelationshipKind::Incoming => return "Incoming Friend Request".into(),
        RelationshipKind::Outgoing => return "Outgoing Friend Request".into(),
        _ => {}
    }
    let Some(user) = user else {
        return "Offline".into();
    };
    if let Some(activity) = user.activity.as_ref().filter(|_| user.status.is_online()) {
        return activity.clone();
    }
    match user.status {
        Status::Online => "Online",
        Status::Idle => "Idle",
        Status::DoNotDisturb => "Do Not Disturb",
        Status::Offline => "Offline",
    }
    .into()
}

pub(crate) fn friends_page(app: &DiscordApp, cx: &App) -> AnyElement {
    let query = search_query(app, cx);
    let tab = app.home.tab;
    if tab == FriendsTab::Add {
        return div()
            .flex()
            .size_full()
            .child(friends::add_friend_page(app))
            .child(active_now(app, cx))
            .into_any_element();
    }
    let ids = match tab {
        FriendsTab::Online => app.store.online_friends(),
        FriendsTab::All => app.store.friends(),
        FriendsTab::Pending => app.store.pending_requests(),
        FriendsTab::Suggestions => app
            .store
            .friend_suggestions()
            .into_iter()
            .map(|suggestion| suggestion.user_id)
            .collect(),
        FriendsTab::Add => Vec::new(),
    };
    let ids: Vec<u64> = ids
        .into_iter()
        .filter(|id| {
            query.is_empty()
                || app.store.friend_name(*id).to_lowercase().contains(&query)
                || app
                    .store
                    .user(*id)
                    .is_some_and(|user| user.username.to_lowercase().contains(&query))
        })
        .collect();
    let heading = format!(
        "{} — {}",
        match tab {
            FriendsTab::Online => "Online",
            FriendsTab::All => "All friends",
            FriendsTab::Pending => "Pending",
            FriendsTab::Suggestions => "Friend Suggestions",
            FriendsTab::Add => "",
        },
        ids.len()
    );
    let empty = ids.is_empty();
    let mut rows: Vec<AnyElement> = Vec::with_capacity(ids.len() * 2);
    for (index, id) in ids.iter().copied().enumerate() {
        if index > 0 {
            rows.push(row_divider(app, ids[index - 1], id).into_any_element());
        }
        rows.push(match tab {
            FriendsTab::Suggestions => suggestion_row(app, id, cx).into_any_element(),
            _ => friend_row(app, id, cx).into_any_element(),
        });
    }
    div()
        .flex()
        .size_full()
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .h_full()
                .children(app.home.search.as_ref().map(|search| {
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .mt(rpx(12.))
                        .mb(rpx(8.))
                        .ml(rpx(24.))
                        .mr(rpx(16.))
                        .h(rpx(36.))
                        .px(rpx(10.))
                        .rounded(rpx(8.))
                        .bg(rgb(theme::elevated()))
                        .border_1()
                        .border_color(rgba(theme::border()))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(search)
                                    .appearance(false)
                                    .px_0()
                                    .text_size(rpx(16.)),
                            ),
                        )
                        .child(theme::icon("search", 18., theme::muted()))
                }))
                .child(
                    div()
                        .flex_none()
                        .pt(rpx(16.))
                        .px(rpx(30.))
                        .pb(rpx(8.))
                        .border_b_1()
                        .border_color(rgba(theme::border_subtle()))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::text()))
                        .child(heading),
                )
                .children(
                    matches!(tab, FriendsTab::Pending | FriendsTab::Suggestions)
                        .then(|| app.pending_error())
                        .flatten()
                        .map(|text| {
                            div()
                                .flex_none()
                                .px(rpx(30.))
                                .pt(rpx(8.))
                                .text_size(rpx(14.))
                                .text_color(rgb(theme::RED))
                                .child(text)
                        }),
                )
                .child(
                    div()
                        .id("friends-list")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .pb(rpx(8.))
                        .children(rows)
                        .when(empty, |this| {
                            this.child(
                                div()
                                    .mt(rpx(40.))
                                    .text_center()
                                    .text_size(rpx(14.))
                                    .text_color(rgb(theme::muted()))
                                    .child(match tab {
                                        FriendsTab::Pending => {
                                            "There are no pending friend requests."
                                        }
                                        FriendsTab::Suggestions
                                            if !app.store.friend_suggestions_loaded() =>
                                        {
                                            "Loading suggestions…"
                                        }
                                        FriendsTab::Suggestions => {
                                            "No friend suggestions right now."
                                        }
                                        _ => "No one's around to play with Wumpus.",
                                    }),
                            )
                        }),
                ),
        )
        .child(active_now(app, cx))
        .into_any_element()
}

fn round_action(
    id: impl Into<ElementId>,
    icon: &'static str,
    row_group: impl Into<SharedString>,
) -> Stateful<Div> {
    let row_group = row_group.into();
    let group = SharedString::from(format!("{row_group}-{icon}"));
    div()
        .id(id)
        .group(group.clone())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(ACTION_SIZE))
        .rounded_full()
        .bg(rgb(theme::base_lower()))
        .group_hover(row_group, |style| style.bg(rgb(theme::base_lowest())))
        .cursor_pointer()
        .active(|style| style.bg(rgba(theme::active())))
        .child(
            theme::icon(icon, 20., theme::text())
                .group_hover(group, |style| style.text_color(rgb(theme::strong()))),
        )
}

fn muted_action(
    id: (&'static str, u64),
    icon: &'static str,
    tip: &'static str,
    row_group: SharedString,
) -> Stateful<Div> {
    let tip_key = (tip, id.1);
    div()
        .id(id)
        .relative()
        .flex_none()
        .size(rpx(ACTION_SIZE))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size_full()
                .rounded_full()
                .opacity(0.3)
                .bg(rgb(theme::base_lower()))
                .group_hover(row_group, |style| style.bg(rgb(theme::base_lowest())))
                .child(theme::icon(icon, 20., theme::text())),
        )
        .child(theme::hover_tooltip(tip_key, tip))
        .on_click(|_, _, cx| cx.stop_propagation())
}

fn row_group(user_id: u64) -> SharedString {
    format!("people-row-{user_id}").into()
}

fn row_divider(app: &DiscordApp, above: u64, below: u64) -> Div {
    let hidden = matches!(app.home.hovered_row, Some(id) if id == above || id == below);
    div()
        .flex_none()
        .h(rpx(1.))
        .ml(rpx(30.))
        .mr(rpx(20.))
        .when(!hidden, |this| this.bg(rgba(theme::border_subtle())))
}

pub(super) fn people_row(app: &DiscordApp, id: ElementId, user_id: u64) -> Stateful<Div> {
    let hover = app.entity.clone();
    div()
        .id(id)
        .group(row_group(user_id))
        .flex()
        .flex_none()
        .items_center()
        .gap(rpx(12.))
        .ml(rpx(14.))
        .mr(rpx(10.))
        .py(rpx(16.))
        .px(rpx(10.))
        .rounded(rpx(8.))
        .hover(|style| style.bg(rgba(theme::hover())))
        .without_text_selection()
        .on_hover(move |hovered, _, cx| {
            hover
                .update(cx, |app, cx| app.hover_people_row(user_id, *hovered, cx))
                .ok();
        })
}

fn name_line(user_id: u64, name: &str, username: Option<String>, tag: Option<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .min_w_0()
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(label(("people-name", user_id), name)),
        )
        .children(tag)
        .children(username.map(|username| {
            div()
                .invisible()
                .group_hover(row_group(user_id), |style| style.visible())
                .ml(rpx(4.))
                .min_w_0()
                .truncate()
                .text_size(rpx(14.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::text()))
                .child(username)
        }))
}

fn subtext_line(id: (&'static str, u64), text: &str, icon: Option<&'static str>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .min_w_0()
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .text_color(rgb(theme::muted()))
        .children(icon.map(|icon| theme::icon(icon, 12., theme::GREEN)))
        .child(div().flex_1().min_w_0().truncate().child(label(id, text)))
}

fn friend_row(app: &DiscordApp, user_id: u64, cx: &App) -> impl IntoElement + use<> {
    let user = app.store.user(user_id);
    let kind = app
        .store
        .relationship(user_id)
        .map_or(RelationshipKind::Friend, |relationship| relationship.kind);
    let name = app.store.friend_name(user_id);
    let username = user.map(|user| crate::settings::shown_username(&user.username));
    let status = user.map_or(Status::Offline, |user| user.status);
    let icon = user
        .filter(|user| user.status.is_online())
        .and_then(theme::activity_icon);
    let line = status_text(user, kind);
    let tag = user
        .and_then(|user| user.tag.as_ref())
        .map(|tag| theme::guild_tag(tag, cx).into_any_element());
    let entity = app.entity.clone();
    let group = row_group(user_id);
    let pending = matches!(
        kind,
        RelationshipKind::Incoming | RelationshipKind::Outgoing
    );
    people_row(app, ("friend", user_id).into(), user_id)
        .cursor_pointer()
        .on_click(move |_, window, cx| {
            entity
                .update(cx, |app, cx| {
                    if pending {
                        app.open_full_profile_for(user_id, cx);
                    } else {
                        app.open_dm(user_id, window, cx);
                    }
                })
                .ok();
        })
        .child(theme::user_avatar(
            user,
            user_id,
            DM_AVATAR,
            Some((status, theme::bg())),
            false,
            cx,
        ))
        .child(div().flex().flex_col().flex_1().min_w_0().map(|this| {
            if pending {
                let username = username.unwrap_or(line);
                this.child(name_line(user_id, &name, None, tag))
                    .child(subtext_line(("friend-status", user_id), &username, None))
            } else {
                this.child(name_line(user_id, &name, username, tag))
                    .child(subtext_line(("friend-status", user_id), &line, icon))
            }
        }))
        .child(
            div()
                .flex()
                .flex_none()
                .gap(rpx(10.))
                .ml(rpx(8.))
                .when(kind == RelationshipKind::Incoming, |this| {
                    this.child(muted_action(
                        ("friend-accept", user_id),
                        "check",
                        "Accept friend requests from Discord for now",
                        group.clone(),
                    ))
                })
                .when(pending, |this| {
                    let tip = if kind == RelationshipKind::Incoming {
                        "Ignore friend requests from Discord for now"
                    } else {
                        "Cancel friend requests from Discord for now"
                    };
                    this.child(muted_action(
                        ("friend-drop", user_id),
                        "x",
                        tip,
                        group.clone(),
                    ))
                })
                .when(!pending, |this| {
                    this.child(round_action(
                        ("friend-message", user_id),
                        "message-circle",
                        group.clone(),
                    ))
                }),
        )
}

fn suggestion_row(app: &DiscordApp, user_id: u64, cx: &App) -> impl IntoElement + use<> {
    let user = app.store.user(user_id);
    let name = app.store.friend_name(user_id);
    let username = user.map(|user| user.username.clone()).unwrap_or_default();
    let reason = app
        .store
        .friend_suggestions()
        .into_iter()
        .find(|suggestion| suggestion.user_id == user_id)
        .and_then(|suggestion| suggestion.reason.clone())
        .unwrap_or_else(|| username.clone());
    let group = row_group(user_id);
    let entity = app.entity.clone();
    people_row(app, ("suggestion", user_id).into(), user_id)
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.open_full_profile_for(user_id, cx))
                .ok();
        })
        .child(theme::user_avatar(
            user, user_id, DM_AVATAR, None, false, cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(name_line(user_id, &name, None, None))
                .child(subtext_line(("suggestion-reason", user_id), &reason, None)),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap(rpx(10.))
                .ml(rpx(8.))
                .child(muted_action(
                    ("suggestion-add", user_id),
                    "check",
                    "Send friend requests from Discord for now",
                    group.clone(),
                ))
                .child(muted_action(
                    ("suggestion-dismiss", user_id),
                    "x",
                    "Dismiss suggestions from Discord for now",
                    group,
                )),
        )
}
