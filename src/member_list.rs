use gpui::{prelude::*, *};

use crate::{
    app::DiscordApp,
    data::User,
    theme::{self, rpx},
};

pub fn render(app: &DiscordApp) -> impl IntoElement + use<> {
    let Some(guild) = app.guild() else {
        return div().into_any_element();
    };
    let members = guild
        .member_ids
        .iter()
        .filter_map(|id| app.store.user(*id))
        .collect::<Vec<_>>();
    let hoisted = |user: &User| {
        guild
            .hoisted_roles
            .iter()
            .any(|role| role.member_ids.contains(&user.id))
    };

    let mut sections = guild
        .hoisted_roles
        .iter()
        .map(|role| {
            let users = members
                .iter()
                .copied()
                .filter(|user| user.status.is_online() && role.member_ids.contains(&user.id))
                .collect::<Vec<_>>();
            (role.name.clone(), users)
        })
        .collect::<Vec<_>>();
    sections.push((
        "Online".into(),
        members
            .iter()
            .copied()
            .filter(|user| user.status.is_online() && !hoisted(user))
            .collect(),
    ));
    sections.push((
        "Offline".into(),
        members
            .iter()
            .copied()
            .filter(|user| !user.status.is_online())
            .collect(),
    ));

    div()
        .id("members")
        .w(rpx(264.))
        .h_full()
        .flex_none()
        .overflow_y_scroll()
        .border_l_1()
        .border_color(rgb(theme::BORDER))
        .px_2()
        .pb_4()
        .children(
            sections
                .into_iter()
                .filter(|(_, users)| !users.is_empty())
                .map(|(name, users)| {
                    div()
                        .child(
                            div()
                                .pt_5()
                                .pb_1()
                                .px_2()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(theme::MUTED))
                                .child(format!("{name} — {}", users.len())),
                        )
                        .children(users.into_iter().map(member_row))
                }),
        )
        .into_any_element()
}

fn member_row(user: &User) -> impl IntoElement {
    let online = user.status.is_online();
    let name_color = user.role_color.unwrap_or(theme::TEXT);
    div()
        .id(("member", user.id))
        .flex()
        .items_center()
        .gap_3()
        .h(rpx(44.))
        .px_2()
        .rounded(rpx(8.))
        .cursor_pointer()
        .when(!online, |this| this.opacity(0.35))
        .hover(|style| style.bg(rgb(theme::HOVER)).opacity(1.))
        .child(theme::avatar(
            &user.initials,
            theme::user_color(user.id),
            32.,
            online.then_some((user.status, theme::BG)),
        ))
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
                        .gap_1()
                        .min_w_0()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(rpx(16.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(name_color))
                                .child(user.name.clone()),
                        )
                        .when(user.bot, |this| this.child(theme::app_tag()))
                        .children(user.tag.as_deref().map(theme::guild_tag)),
                )
                .children(user.activity.as_ref().filter(|_| online).map(|activity| {
                    div()
                        .truncate()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::MUTED))
                        .child(activity.clone())
                })),
        )
}
