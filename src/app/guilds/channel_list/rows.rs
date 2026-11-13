use super::*;

pub(super) const CHANNEL_HEIGHT: f32 = 32.;
pub(super) const CHANNEL_GAP: f32 = 2.;

pub(super) fn row_content(
    guild: Option<&Guild>,
    row: Row,
    selected: u64,
    openness: f32,
    voice: Option<voice::Joined>,
    app: Option<(&DiscordApp, bool)>,
) -> Div {
    let content = match row {
        Row::Banner
        | Row::ActionSpacer
        | Row::BoostGoal { .. }
        | Row::ServerGuide
        | Row::Events
        | Row::ChannelsAndRoles
        | Row::ActionDivider
        | Row::VoiceMember { .. }
        | Row::Thread { .. } => div().into_any_element(),
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
                        .children(
                            app.filter(|(app, _)| app.store.can_manage_channel(category.id))
                                .map(|_| create_channel_action(category.id)),
                        ),
                )
                .into_any_element(),
            None => div().into_any_element(),
        },
        Row::Channel { category, channel } => match guild.and_then(|guild| {
            let channel = guild.categories.get(category)?.channels.get(channel)?;
            let muted = app.is_some_and(|(app, _)| {
                let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
                app.store
                    .channel_muted_with_parent(guild.id, channel, now_ms)
            });
            Some((channel, muted))
        }) {
            Some((channel, muted)) if openness < 1. => div()
                .h(rpx((CHANNEL_HEIGHT + CHANNEL_GAP) * openness))
                .overflow_hidden()
                .opacity(openness)
                .child(
                    div()
                        .flex_none()
                        .h(rpx(CHANNEL_HEIGHT + CHANNEL_GAP))
                        .child(channel_row(
                            channel,
                            selected == channel.id,
                            muted,
                            voice,
                            actions(app, channel, selected),
                        )),
                )
                .into_any_element(),
            Some((channel, muted)) => channel_row(
                channel,
                selected == channel.id,
                muted,
                voice,
                actions(app, channel, selected),
            )
            .into_any_element(),
            None => div().into_any_element(),
        },
    };
    div().w_full().px_2().child(content)
}

pub(super) fn thread_row(
    app: &DiscordApp,
    thread: &ThreadRow,
    row: usize,
) -> impl IntoElement + use<> {
    let id = thread.id;
    let selected = app.selected_channel_id == id;
    let mentions = app.store.thread_mentions(id);
    let muted = thread.muted
        || app.guild().is_some_and(|guild| {
            let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
            guild.channel(thread.parent_id).is_some_and(|parent| {
                app.store
                    .channel_muted_with_parent(guild.id, parent, now_ms)
            })
        });
    let unread = (app.store.thread_unread(id) || mentions > 0) && !selected && !muted;
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
            } else if muted {
                theme::interactive_muted()
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
    muted: bool,
    voice: Option<voice::Joined>,
    actions: Option<Div>,
) -> impl IntoElement + use<> {
    let id = channel.id;
    let joined = voice.filter(|voice| voice.channel_id == id);
    let mentions = channel.mentions;
    let new_posts = if muted { 0 } else { channel.new_posts };
    let dim = muted && !selected && joined.is_none();
    let unread = (channel.unread || mentions > 0 || new_posts > 0) && !selected && !muted;
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
        } else if dim {
            theme::interactive_muted()
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
            } else if dim {
                theme::interactive_muted()
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
        .children(actions)
}

fn actions(app: Option<(&DiscordApp, bool)>, channel: &Channel, selected: u64) -> Option<Div> {
    let (app, hovered) = app?;
    (hovered || selected == channel.id)
        .then(|| row_actions(app, channel))
        .flatten()
}

fn row_actions(app: &DiscordApp, channel: &Channel) -> Option<Div> {
    let id = channel.id;
    let voice = channel.kind == ChannelKind::Voice;
    let open_chat = voice.then(|| {
        let entity = app.entity.clone();
        let guild_id = app.selected_guild_id;
        row_action(("channel-open-chat", id), "message-circle")
            .child(theme::hover_tooltip(
                ("channel-open-chat-tip", id),
                "Open Chat",
            ))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| app.open_voice_chat(guild_id, id, window, cx))
                    .ok();
            })
    });
    let invite = app.store.can_create_invite(id).then(|| {
        row_action(("channel-invite", id), "user-plus")
            .child(theme::hover_tooltip_detail(
                ("channel-invite-tip", id),
                if voice {
                    "Invite to Voice"
                } else {
                    "Invite to Channel"
                },
                "Create invites from Discord for now",
            ))
            .on_click(|_, _, cx| cx.stop_propagation())
    });
    let settings = app.store.can_manage_channel(id).then(|| {
        row_action(("channel-settings", id), "settings")
            .child(theme::hover_tooltip_detail(
                ("channel-settings-tip", id),
                "Edit Channel",
                "Edit channels from Discord for now",
            ))
            .on_click(|_, _, cx| cx.stop_propagation())
    });
    let favorite = app.favorites_visible().then(|| favorite_action(app, id));
    if open_chat.is_none() && invite.is_none() && settings.is_none() && favorite.is_none() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(rpx(4.))
            .children(favorite)
            .children(open_chat)
            .children(invite)
            .children(settings),
    )
}

fn create_channel_action(category_id: u64) -> Stateful<Div> {
    row_action(("category-create-channel", category_id), "plus")
        .child(theme::hover_tooltip_detail(
            ("category-create-channel-tip", category_id),
            "Create Channel",
            "Create channels from Discord for now",
        ))
        .on_click(|_, _, cx| cx.stop_propagation())
}

fn favorite_action(app: &DiscordApp, id: u64) -> Stateful<Div> {
    let favorite = app.is_favorite_channel(id);
    let entity = app.entity.clone();
    row_action(
        ("channel-favorite", id),
        if favorite { "star-filled" } else { "star" },
    )
    .child(theme::hover_tooltip(
        ("channel-favorite-tip", id),
        if favorite {
            "Remove from Favorites"
        } else {
            "Add to Favorites"
        },
    ))
    .on_click(move |_, _, cx| {
        cx.stop_propagation();
        entity
            .update(cx, |app, cx| app.toggle_favorite_channel(id, cx))
            .ok();
    })
}

pub(super) fn favorites_sidebar(app: &DiscordApp) -> impl IntoElement {
    let selected = app.selected_channel_id;
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let rows: Vec<AnyElement> = app
        .favorite_channels()
        .into_iter()
        .filter_map(|id| {
            let guild = app.store.guild_by_id(app.store.channel_guild_id(id)?)?;
            let channel = guild.channel(id)?;
            let muted = app
                .store
                .channel_muted_with_parent(guild.id, channel, now_ms);
            let entity = app.entity.clone();
            let actions = Some(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(favorite_action(app, id)),
            );
            Some(
                div()
                    .id(("favorite-channel", id))
                    .flex()
                    .flex_col()
                    .on_click(move |_, window, cx| {
                        entity
                            .update(cx, |app, cx| app.open_favorite_channel(id, window, cx))
                            .ok();
                    })
                    .child(channel_row(channel, selected == id, muted, None, actions))
                    .child(
                        div()
                            .pl(rpx(36.))
                            .mt(rpx(-6.))
                            .mb(rpx(2.))
                            .truncate()
                            .text_size(rpx(12.))
                            .line_height(rpx(14.))
                            .text_color(rgb(theme::faint()))
                            .child(guild.name.clone()),
                    )
                    .into_any_element(),
            )
        })
        .collect();
    let empty = rows.is_empty();
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .h(rpx(HEADER_HEIGHT))
                .px(rpx(16.))
                .border_b_1()
                .border_color(rgba(theme::border()))
                .text_size(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child(theme::icon("star-filled", 16., theme::strong()))
                .child("Favorites"),
        )
        .child(
            div()
                .id("favorites-list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_2()
                .py_2()
                .children(rows)
                .when(empty, |this| {
                    this.child(
                        div()
                            .p(rpx(8.))
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .text_color(rgb(theme::muted()))
                            .child(
                                "No favorites yet. Hover a channel in a server and click its star \
                                 to add it here.",
                            ),
                    )
                }),
        )
}

fn row_action(id: impl Into<ElementId>, icon: &str) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(16.))
        .cursor_pointer()
        .child(
            theme::icon(icon, 16., theme::muted())
                .hover(|style| style.text_color(rgb(theme::strong()))),
        )
}
