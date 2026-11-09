use super::rows::{CHANNEL_GAP, CHANNEL_HEIGHT};
use super::*;

const BOOST_PINK: u32 = 0xFF4CD2;
const BOOST_GOAL_GROUP: &str = "boost-goal";

fn boost_fill(right: f32, left: f32) -> Background {
    let pink = Hsla::from(rgb(BOOST_PINK));
    linear_gradient(
        270.,
        linear_color_stop(pink.opacity(right), 0.),
        linear_color_stop(pink.opacity(left), 1.),
    )
}

pub(super) fn spacer(banner: bool) -> impl IntoElement {
    div().h(rpx(if banner { 12. } else { 4. }))
}

pub(super) fn boost_goal(app: &DiscordApp, spaced: bool) -> AnyElement {
    let Some(goal) = app.guild().and_then(|guild| guild.boost_goal) else {
        return div().into_any_element();
    };
    let guild_id = app.selected_guild_id;
    let fraction = if goal.goal == 0 {
        1.
    } else {
        (goal.boosts as f32 / goal.goal as f32).min(1.)
    };
    let reached = goal.boosts >= goal.goal;
    let count = if reached {
        format!("{} Boosts", goal.boosts)
    } else {
        format!("{}/{} Boosts", goal.boosts, goal.goal)
    };
    let progress = div()
        .size_full()
        .map(|this| {
            if fraction <= 0.05 {
                this.rounded_l_full()
            } else {
                this.rounded_full()
            }
        })
        .bg(boost_fill(0.4, 0.1))
        .group_hover(BOOST_GOAL_GROUP, |style| style.bg(boost_fill(0.7, 0.25)));
    let text = div()
        .absolute()
        .inset_0()
        .px(rpx(8.))
        .flex()
        .items_center()
        .justify_between()
        .gap(rpx(8.))
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(theme::text()))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(2.))
                .min_w_0()
                .child(div().truncate().child("Boost Goal"))
                .when(reached, |this| {
                    this.child(label(("boost-goal-reached", guild_id), "\u{1F389}"))
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(2.))
                .min_w_0()
                .opacity(0.7)
                .group_hover(BOOST_GOAL_GROUP, |style| style.opacity(1.))
                .child(div().truncate().child(count))
                .child(theme::icon("chevron-right", 12., theme::text()).flex_none()),
        );
    div()
        .w_full()
        .px_2()
        .child(
            div()
                .id(("boost-goal", guild_id))
                .group(BOOST_GOAL_GROUP)
                .relative()
                .pt(rpx(4.))
                .px(rpx(4.))
                .pb(rpx(12.))
                .when(spaced, |this| this.mb(rpx(12.)))
                .border_b_1()
                .border_color(rgba(theme::border_subtle()))
                .cursor_pointer()
                .child(
                    div()
                        .relative()
                        .h(rpx(26.))
                        .child(div().absolute().inset_0().rounded_full().bg(theme::surface(
                            theme::Layer::Higher,
                            Some(theme::Layer::Lower),
                            theme::base_low(),
                        )))
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left_0()
                                .w(relative(fraction))
                                .p(rpx(2.))
                                .child(progress),
                        )
                        .child(text),
                )
                .child(theme::hover_tooltip_detail(
                    ("boost-goal-tip", guild_id),
                    "Server Boosts",
                    "Boost servers from Discord for now",
                )),
        )
        .into_any_element()
}

pub(super) fn link(app: &DiscordApp, row: Row) -> AnyElement {
    let guild_id = app.selected_guild_id;
    let (key, tip_key, icon, label, detail): (_, _, _, SharedString, _) = match row {
        Row::ServerGuide => (
            "server-guide",
            "server-guide-tip",
            "signpost",
            "Server Guide".into(),
            "Open the Server Guide from Discord for now",
        ),
        Row::Events => (
            "events",
            "events-tip",
            "calendar",
            match app.guild().and_then(|guild| guild.events).unwrap_or(0) {
                0 => "Events".into(),
                1 => "1 Event".into(),
                count => format!("{count} Events").into(),
            },
            "Browse events from Discord for now",
        ),
        Row::ChannelsAndRoles => (
            "channels-and-roles",
            "channels-and-roles-tip",
            "text-search",
            "Channels & Roles".into(),
            "Browse channels and roles from Discord for now",
        ),
        _ => return div().into_any_element(),
    };
    div()
        .w_full()
        .px_2()
        .child(
            div()
                .id((key, guild_id))
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
                .text_color(rgb(theme::muted()))
                .cursor_pointer()
                .hover(|style| {
                    style
                        .bg(rgba(theme::hover()))
                        .text_color(rgb(theme::text()))
                })
                .child(theme::icon(icon, 20., theme::faint()).flex_none())
                .child(div().flex_1().min_w_0().truncate().child(label.clone()))
                .child(theme::hover_tooltip_detail(
                    (tip_key, guild_id),
                    label,
                    detail,
                )),
        )
        .into_any_element()
}

pub(super) fn divider() -> impl IntoElement {
    div()
        .w_full()
        .px(rpx(16.))
        .pt(rpx(11.))
        .child(div().h(px(1.)).bg(rgba(theme::border_subtle())))
}
