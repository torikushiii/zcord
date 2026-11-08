use super::*;

pub(crate) fn bio(
    app: &DiscordApp,
    card: &ProfileCard,
    clamped: bool,
    cx: &App,
) -> Option<AnyElement> {
    if crate::settings::streamer().hides_personal() {
        return None;
    }
    let profile = card.profile.as_deref()?;
    let palette = card.palette();
    let text = guild_profile(profile)
        .and_then(|p| p.bio.clone())
        .filter(|bio| !bio.trim().is_empty())
        .or_else(|| profile.user_profile.as_ref()?.bio.clone())
        .filter(|bio| !bio.trim().is_empty())?;
    Some(
        div()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .text_color(rgb(palette.strong))
            .when(clamped, |this| {
                this.max_h(rpx(18. * BIO_LINES as f32)).overflow_hidden()
            })
            .child(markdown(app, card, card.user_id, &text, cx))
            .into_any_element(),
    )
}

/// Profile text Discord renders as markdown (the bio, some widget fields), in the card's colours.
/// `id` keys the parse cache, so give each field its own.
pub(crate) fn markdown(
    app: &DiscordApp,
    card: &ProfileCard,
    id: u64,
    text: &str,
    cx: &App,
) -> AnyElement {
    let palette = card.palette();
    let parsed = app.parsed(id, text);
    let context = BodyContext {
        store: &app.store,
        guild_id: card.guild_id.unwrap_or(0),
        message_id: id,
        revealed: &app.revealed_spoilers,
        reveal_all: false,
        entity: app.entity.clone(),
        colors: BodyColors {
            strong: palette.strong,
            muted: palette.muted,
            faint: palette.faint,
            link: palette.link,
            quote: None,
        },
    };
    render_body(&parsed.blocks, false, false, &context, cx).into_any_element()
}

pub(crate) fn playing(user: &User, palette: ProfilePalette) -> Option<AnyElement> {
    let activity = user.activity.as_ref().filter(|_| user.playing)?;
    if user.bot {
        return Some(
            div()
                .flex()
                .flex_col()
                .gap(rpx(8.))
                .p(rpx(8.))
                .rounded(rpx(8.))
                .bg(palette.panel)
                .child(
                    div()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(palette.strong))
                        .child("Playing"),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.text))
                        .child(label(("profile-playing", user.id), activity)),
                )
                .into_any_element(),
        );
    }
    Some(section(
        "Playing",
        div()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .p(rpx(12.))
            .rounded(rpx(8.))
            .bg(palette.panel)
            .border_1()
            .border_color(palette.border)
            .child(theme::icon("gamepad-2", 20., theme::GREEN))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.strong))
                    .child(label(("profile-playing", user.id), activity)),
            )
            .into_any_element(),
        palette,
    ))
}

pub(crate) fn add_app_button(
    app: &DiscordApp,
    card: &ProfileCard,
    full_width: bool,
) -> Option<AnyElement> {
    card.profile
        .as_deref()?
        .application
        .as_ref()
        .filter(|application| application.installable())?;
    let palette = card.palette();
    // Dimmed through an inner layer so the tooltip, a child of the button, stays opaque.
    let face = div()
        .flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(rpx(4.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .bg(palette.button)
        .opacity(0.5)
        .child(theme::icon("plus", 16., palette.strong))
        .child("Add App");
    let button = div()
        .id("profile-add-app")
        .flex_none()
        .h(rpx(32.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(palette.strong))
        .without_text_selection()
        .when(full_width, |this| this.w_full().mt(rpx(4.)))
        .child(face);
    Some(
        menu::action_tooltip(button, app, card, "Add apps from Discord for now", true)
            .into_any_element(),
    )
}

pub(crate) fn is_app(app: &DiscordApp, card: &ProfileCard) -> bool {
    app.store.user(card.user_id).is_some_and(|user| user.bot)
        || card
            .profile
            .as_deref()
            .is_some_and(|profile| profile.application.is_some())
}

pub(crate) fn app_section(
    title: &'static str,
    body: AnyElement,
    palette: ProfilePalette,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .child(
            div()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.text))
                .child(title),
        )
        .child(body)
        .into_any_element()
}

pub(crate) fn command_chips(app: &DiscordApp, card: &ProfileCard) -> Option<AnyElement> {
    if card.commands.is_empty() {
        return None;
    }
    let palette = card.palette();
    Some(
        div()
            .flex()
            .flex_wrap()
            .gap(rpx(8.))
            .children(card.commands.iter().map(|command| {
                let (id, description) = (command.id, command.description.clone());
                let entity = app.entity.clone();
                div()
                    .id(("profile-command", id))
                    .relative()
                    .flex()
                    .items_center()
                    .h(rpx(32.))
                    .px(rpx(14.))
                    .rounded(rpx(8.))
                    .bg(palette.button)
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(palette.strong))
                    .without_text_selection()
                    .on_hover(move |hovered, _, cx| {
                        entity
                            .update(cx, |app, cx| {
                                if let Some(card) = &mut app.profile {
                                    if *hovered {
                                        card.hovered_command = Some(id);
                                    } else if card.hovered_command == Some(id) {
                                        card.hovered_command = None;
                                    }
                                    cx.notify();
                                }
                            })
                            .ok();
                    })
                    .child(format!("/{}", command.name))
                    .when(
                        card.hovered_command == Some(id) && !description.is_empty(),
                        |this| this.child(theme::tooltip_above(description)),
                    )
            }))
            .into_any_element(),
    )
}

pub(crate) fn roles(app: &DiscordApp, card: &ProfileCard) -> Option<AnyElement> {
    Some(section("Roles", role_chips(app, card)?, card.palette()))
}

pub(crate) fn role_chips(app: &DiscordApp, card: &ProfileCard) -> Option<AnyElement> {
    let palette = card.palette();
    let roles = app.store.member_roles(card.guild_id?, card.user_id);
    if roles.is_empty() {
        return None;
    }
    Some(
        div()
            .flex()
            .flex_wrap()
            .gap(rpx(4.))
            .children(roles.into_iter().map(|role| {
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .min_h(rpx(24.))
                    .pl(rpx(6.))
                    .pr(rpx(8.))
                    .rounded(rpx(8.))
                    .border_1()
                    .border_color(palette.border)
                    .text_size(rpx(12.))
                    .line_height(rpx(16.))
                    .text_color(rgb(palette.text))
                    .child(
                        div()
                            .size(rpx(12.))
                            .flex_none()
                            .rounded_full()
                            .bg(rgb(role.color.unwrap_or(palette.muted))),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(label(("profile-role", role.id), &role.name)),
                    )
            }))
            .into_any_element(),
    )
}

pub(crate) fn section(
    title: &'static str,
    body: AnyElement,
    palette: ProfilePalette,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .child(
            div()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.muted))
                .child(title),
        )
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(palette.text))
                .child(body),
        )
        .into_any_element()
}

pub(crate) fn member_since(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    section(
        "Member Since",
        member_since_dates(app, card, cx).into_any_element(),
        card.palette(),
    )
}

pub(crate) fn member_since_dates(app: &DiscordApp, card: &ProfileCard, cx: &App) -> Div {
    let palette = card.palette();
    let format = |date: chrono::NaiveDate| date.format("%b %-d, %Y").to_string();
    let created = (card.user_id >> 22) as i64 + DISCORD_EPOCH_MS as i64;
    let created = chrono::DateTime::from_timestamp_millis(created).map(|date| date.date_naive());
    let joined = card
        .profile
        .as_deref()
        .and_then(|p| p.guild_member.as_ref()?.joined_at.as_deref())
        .and_then(|joined| chrono::DateTime::parse_from_rfc3339(joined).ok())
        .map(|date| date.date_naive());
    let guild = card.guild_id.and_then(|id| app.store.guild_by_id(id));
    div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .children(created.map(|date| {
            div()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .child(theme::logo(16.))
                .child(format(date))
        }))
        .children(joined.zip(guild).map(|(date, guild)| {
            let icon = guild
                .icon_spec()
                .and_then(|spec| images::resolve(&spec, theme::image_px(16.), cx));
            div()
                .flex()
                .items_center()
                .gap(rpx(8.))
                .child(div().size(rpx(4.)).rounded_full().bg(rgb(palette.muted)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(4.))
                        .child(match icon {
                            Some(path) => {
                                img(path).size(rpx(16.)).rounded(rpx(4.)).into_any_element()
                            }
                            None => div()
                                .size(rpx(16.))
                                .rounded(rpx(4.))
                                .bg(rgb(guild.icon_color))
                                .into_any_element(),
                        })
                        .child(format(date)),
                )
        }))
}
