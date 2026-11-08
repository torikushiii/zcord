use super::*;

const MEMBER_AVATAR: f32 = 24.;
const MEMBER_INDENT: f32 = 36.;

pub(super) fn connected_title(details: bool, animate: bool, color: u32) -> AnyElement {
    const LINE: f32 = 20.;
    let lines = div()
        .relative()
        .flex()
        .flex_col()
        .child(div().h(rpx(LINE)).truncate().child("Voice Connected"))
        .child(div().h(rpx(LINE)).truncate().child("Voice Details"));
    let lines = if animate {
        let animation = Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint());
        lines
            .with_animation(
                ("voice-title", u64::from(details)),
                animation,
                move |this, delta| {
                    let shown = if details { delta } else { 1. - delta };
                    this.top(rpx(-LINE * shown))
                },
            )
            .into_any_element()
    } else {
        lines
            .top(rpx(if details { -LINE } else { 0. }))
            .into_any_element()
    };
    div()
        .h(rpx(LINE))
        .min_w_0()
        .overflow_hidden()
        .text_color(rgb(color))
        .child(lines)
        .into_any_element()
}

pub(crate) fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

pub(super) fn remember_channel(channel: Option<SavedChannel>, cx: &mut App) {
    crate::settings::update(cx, |settings| settings.voice_channel = channel);
}

pub(crate) fn live_badge() -> Div {
    div()
        .flex_none()
        .px(rpx(6.))
        .rounded(rpx(8.))
        .bg(rgb(theme::RED))
        .text_size(rpx(11.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::BOLD)
        .text_color(white())
        .child("LIVE")
}

pub(crate) fn member_row(
    app: &DiscordApp,
    guild_id: u64,
    channel_id: u64,
    member: &VoiceMember,
    cx: &App,
) -> impl IntoElement + use<> {
    let user_id = member.user_id;
    let connected_elsewhere = user_id == app.store.current_user_id
        && app
            .voice
            .call
            .as_ref()
            .is_none_or(|call| call.guild_id != guild_id || call.channel_id != channel_id);
    let speaking = !connected_elsewhere && app.voice.speaking.contains(&user_id);
    let name = app
        .store
        .member_name(guild_id, user_id)
        .map(str::to_owned)
        .or_else(|| app.store.user(user_id).map(|user| user.name.clone()))
        .unwrap_or_default();
    let tag = app
        .store
        .user(user_id)
        .and_then(|user| user.tag.as_ref())
        .map(|tag| theme::guild_tag(tag, cx));
    let muted_color = |server: bool| if server { theme::RED } else { theme::muted() };
    let can_watch = user_id != app.store.current_user_id;
    let watch = app.entity.clone();
    let streamer = can_watch && member.streaming;
    let card = app.entity.clone();
    let row_bounds = Rc::new(Cell::new(Bounds::default()));
    let row_paint = row_bounds.clone();
    let row = div()
        .id(("voice-member", user_id))
        .when(streamer, |this| {
            this.on_prepaint(move |bounds, _, _| row_paint.set(bounds))
                .on_hover(move |hovered, window, cx| {
                    let position = stream_card::position(row_bounds.get(), window);
                    card.update(cx, |app, cx| {
                        app.hover_streamer((guild_id, channel_id, user_id), *hovered, position, cx)
                    })
                    .ok();
                })
        })
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(MEMBER_HEIGHT))
        .px_2()
        .rounded(rpx(6.))
        .when(connected_elsewhere, |this| this.opacity(0.5))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::hover())))
        .child(
            div()
                .relative()
                .flex_none()
                .size(rpx(MEMBER_AVATAR))
                .child(theme::undecorated_member_avatar(
                    &app.store,
                    guild_id,
                    user_id,
                    MEMBER_AVATAR,
                    cx,
                ))
                .when(speaking, |this| {
                    this.child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded_full()
                            .border_2()
                            .border_color(rgb(theme::GREEN))
                            .child(
                                div()
                                    .size_full()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(theme::bg())),
                            ),
                    )
                }),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if speaking {
                    theme::strong()
                } else {
                    theme::muted()
                }))
                .child(label(("voice-member-name", user_id), &name)),
        )
        .children(tag)
        .child(div().flex_1())
        .when(member.streaming, |this| {
            this.child(
                live_badge()
                    .id(("voice-member-live", user_id))
                    .when(can_watch, |this| {
                        this.cursor_pointer().on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            watch
                                .update(cx, |app, cx| {
                                    app.watch_or_join((guild_id, channel_id, user_id), window, cx)
                                })
                                .ok();
                        })
                    }),
            )
        })
        .when(member.video, |this| {
            this.child(theme::icon("video", 16., theme::muted()))
        })
        .when(
            member.mute || (member.self_mute && !member.self_deaf),
            |this| this.child(theme::icon("mic-off", 16., muted_color(member.mute))),
        )
        .when(member.deaf || member.self_deaf, |this| {
            this.child(theme::icon("headphone-off", 16., muted_color(member.deaf)))
        });
    // List items are laid out as roots, which ignore their own margins, so indent with padding.
    div()
        .w_full()
        .pl(rpx(MEMBER_INDENT))
        .pr_2()
        .py(rpx(1.))
        .child(row)
}
