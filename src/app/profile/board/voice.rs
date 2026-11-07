use super::*;

const PARTY_BOX: f32 = 60.;
const RING: f32 = 1.6;

#[derive(Clone, Copy)]
struct Spot {
    top: f32,
    left: f32,
    size: f32,
}

const fn spot(top: f32, left: f32, size: f32) -> Spot {
    Spot { top, left, size }
}

const SPOTS: [&[Spot]; 4] = [
    &[spot(15., 15., 28.)],
    &[spot(7., 7., 20.), spot(25., 25., 26.)],
    &[spot(5., 10., 18.), spot(15., 31., 22.), spot(29., 5., 24.)],
    &[
        spot(5., 5., 22.),
        spot(7., 33., 18.),
        spot(33., 7., 18.),
        spot(31., 31., 22.),
    ],
];

pub(in crate::app::profile) fn popout_voice(
    app: &DiscordApp,
    card: &ProfileCard,
    cx: &App,
) -> Option<AnyElement> {
    let palette = card.palette();
    let (guild_id, channel_id) = app.store.user_voice(card.user_id)?;
    let guild = app.store.guilds.iter().find(|guild| guild.id == guild_id);
    let channel = app
        .store
        .channel_name(channel_id)
        .unwrap_or_default()
        .to_owned();
    let members: Vec<u64> = app
        .store
        .voice_members(guild_id, channel_id)
        .into_iter()
        .map(|member| member.user_id)
        .collect();
    let joined = app
        .voice
        .joined_call()
        .is_some_and(|(_, joined)| joined == channel_id);
    let can_join = app.store.can_connect_voice(guild_id, channel_id);
    let entity = app.entity.clone();
    let join = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        cx.stop_propagation();
        entity
            .update(cx, |app, cx| {
                app.open_voice_channel(guild_id, channel_id, cx);
                app.close_profile(cx);
            })
            .ok();
    };
    let guild_line = guild.map(|guild| {
        let entity = app.entity.clone();
        div()
            .id(("profile-voice-guild", card.user_id))
            .max_w_full()
            .truncate()
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .text_color(rgb(palette.strong))
            .cursor_pointer()
            .hover(|style| style.underline())
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| {
                        app.close_profile(cx);
                        app.select_guild(guild_id, window, cx);
                    })
                    .ok();
            })
            .child(label(
                ("profile-voice-guild-name", card.user_id),
                &format!("in {}", guild.name),
            ))
    });
    Some(
        super::hover_card(
            app,
            ("profile-popout-voice", card.user_id),
            Tab::Activity,
            palette,
        )
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .p(rpx(8.))
        .rounded(rpx(8.))
        .bg(palette.panel)
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.strong))
                .child("In voice"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(8.))
                .child(party(app, card, guild_id, &members, palette, cx))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(rpx(2.))
                                .min_w_0()
                                .child(theme::icon("volume-2", 12., palette.strong))
                                .child(
                                    div()
                                        .id(("profile-voice-channel", card.user_id))
                                        .min_w_0()
                                        .truncate()
                                        .text_size(rpx(14.))
                                        .line_height(rpx(18.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.strong))
                                        .when(can_join, |this| {
                                            this.cursor_pointer()
                                                .hover(|style| style.underline())
                                                .on_click(join.clone())
                                        })
                                        .child(label(
                                            ("profile-voice-channel-name", card.user_id),
                                            &channel,
                                        )),
                                ),
                        )
                        .children(guild_line),
                ),
        )
        .when(can_join, |this| {
            this.child(
                div()
                    .id(("profile-voice-join", card.user_id))
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(rpx(32.))
                    .mt(rpx(4.))
                    .px(rpx(12.))
                    .rounded(rpx(8.))
                    .bg(palette.button)
                    .cursor_pointer()
                    .hover(move |style| style.bg(palette.raised))
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(palette.strong))
                    .child(if joined { "Open Voice" } else { "Join Voice" })
                    .on_click(join),
            )
        })
        .into_any_element(),
    )
}

fn party(
    app: &DiscordApp,
    card: &ProfileCard,
    guild_id: u64,
    members: &[u64],
    palette: ProfilePalette,
    cx: &App,
) -> Div {
    let spots = SPOTS[members.len().clamp(1, SPOTS.len()) - 1];
    let more = members.len().saturating_sub(SPOTS.len() - 1);
    let count_spot = match more {
        0 | 1 => None,
        10.. => Some(3),
        _ => Some(1),
    };
    let ring = if palette.surface_light {
        hsla(0., 0., 1., 0.72)
    } else {
        hsla(0., 0., 1., 0.24)
    };
    div()
        .relative()
        .flex_none()
        .size(rpx(PARTY_BOX))
        .rounded(rpx(8.))
        .bg(palette.button)
        .border_1()
        .border_color(palette.border)
        .children(
            spots
                .iter()
                .zip(members)
                .enumerate()
                .map(|(index, (place, &user_id))| {
                    let Spot { top, left, size } = *place;
                    let spot = div()
                        .id(("profile-voice-member", user_id))
                        .absolute()
                        .top(rpx(top - RING))
                        .left(rpx(left - RING))
                        .size(rpx(size + RING * 2.))
                        .p(rpx(RING))
                        .rounded_full()
                        .bg(ring);
                    if count_spot == Some(index) {
                        let text = if more > 99 {
                            ">99".to_owned()
                        } else {
                            format!("+{more}")
                        };
                        return spot
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size_full()
                                    .rounded_full()
                                    .bg(ring)
                                    .text_size(rpx(10.))
                                    .line_height(rpx(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.strong))
                                    .child(text),
                            )
                            .into_any_element();
                    }
                    let name = app
                        .store
                        .member_name(guild_id, user_id)
                        .map(str::to_owned)
                        .unwrap_or_else(|| app.store.friend_name(user_id));
                    spot.child(theme::undecorated_member_avatar(
                        &app.store, guild_id, user_id, size, cx,
                    ))
                    .child(theme::hover_tooltip(
                        ("profile-voice-member-tip", card.user_id ^ user_id),
                        name,
                    ))
                    .into_any_element()
                }),
        )
}
