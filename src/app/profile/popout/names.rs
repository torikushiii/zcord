use super::*;

pub(crate) fn display_names(
    app: &DiscordApp,
    card: &ProfileCard,
    large: bool,
    on_name: Option<ClickHandler>,
    cx: &App,
) -> impl IntoElement + use<> {
    let profile = card.profile.as_deref();
    let palette = card.palette();
    let user = app.store.user(card.user_id);
    let guild_id = card.guild_id.unwrap_or(0);
    // Edits typed into your own profile show on every card of it; the full profile also edits
    // these lines in place.
    let styling = editing_styles(app, card);
    let identity = styling.filter(|_| large);
    let username = profile
        .map(|p| p.user.user.username.clone())
        .filter(|username| !username.is_empty());
    let shown_username = username.clone().map(|username| {
        if crate::settings::streamer().hides_personal() {
            return crate::settings::shown_username(&username);
        }
        match profile
            .and_then(|p| p.user.user.discriminator.as_deref())
            .filter(|discriminator| !matches!(*discriminator, "" | "0"))
        {
            Some(discriminator) => format!("{username}#{discriminator}"),
            None => username,
        }
    });
    let app_tag = user.filter(|user| user.bot).map(|user| {
        div()
            .mt(rpx(if large { 1. } else { -2. }))
            .child(theme::app_tag(user.verified_bot))
    });
    let saved_name = app
        .store
        .member_name(guild_id, card.user_id)
        .map(str::to_owned)
        .or_else(|| user.map(|user| user.name.clone()))
        .unwrap_or_else(|| "Unknown User".to_owned());
    // An emptied display name falls back to the username, as on Discord.
    let name = match styling.and_then(|editor| editor.ui.identity.display_name.clone()) {
        Some(edited) if edited.is_empty() => username.clone().unwrap_or(saved_name.clone()),
        Some(edited) => edited,
        None => saved_name.clone(),
    };
    let pronouns = profile
        .and_then(guild_profile)
        .and_then(|p| p.pronouns.clone())
        .filter(|p| !p.is_empty())
        .or_else(|| profile?.user_profile.as_ref()?.pronouns.clone())
        // Discord renders pronouns as one line of HTML text, so newlines padding them out
        // collapse to spaces there.
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty());
    // Cleared server pronouns show the main profile's, as they will once saved.
    let pronouns = match styling.and_then(|editor| editor.ui.identity.pronouns.clone()) {
        Some(edited) if edited.is_empty() && card.guild_id.is_some() => {
            Some(super::identity_editor::scope_pronouns(profile, false).0)
                .filter(|p| !p.trim().is_empty())
        }
        Some(edited) => Some(edited).filter(|p| !p.is_empty()),
        None => pronouns,
    };
    let tag = match styling {
        Some(editor) => super::identity_editor::shown_tag(app, editor),
        None => user.and_then(|user| user.tag.clone()),
    };
    // The name style dialog's working copy previews before it reaches the draft.
    let edited_style = editing_styles(app, card).map(|editor| match &editor.ui.dialog {
        Some(super::styles_panel::StyleDialog::NameStyle(style)) => {
            Some(super::styles_panel::shown_name_style(style))
        }
        _ => super::styles_panel::shown(app, editor)
            .name_style
            .as_ref()
            .map(super::styles_panel::shown_name_style),
    });
    let name_style = match &edited_style {
        Some(style) => style.as_ref(),
        None => app.store.member_name_style(guild_id, card.user_id),
    };
    let dot = || {
        div()
            .size(rpx(4.))
            .mx(rpx(1.))
            .rounded_full()
            .bg(rgb(palette.text))
    };
    let badges = profile.and_then(|profile| badges(app, card, profile, palette, cx));
    let (inline_badges, badge_line) = if identity.is_some() {
        (None, badges)
    } else {
        (badges, None)
    };
    let name_line = div()
        .id("profile-display-name")
        .map(|this| match on_name {
            Some(handler) => this.cursor_pointer().on_click(handler),
            None => this,
        })
        .text_size(rpx(if large { 24. } else { 20. }))
        .line_height(rpx(if large { 30. } else { 24. }))
        .font_weight(if large {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::BOLD
        })
        .text_color(rgb(palette.strong))
        .child(crate::app::display_name::render(
            ("profile-styled-name", card.user_id),
            &name,
            name_style,
            crate::app::display_name::Surface::Profile,
            cx,
        ));
    let name_line = match app_tag {
        Some(tag) => div()
            .flex()
            .items_center()
            .gap(rpx(4.))
            .child(name_line)
            .child(tag)
            .into_any_element(),
        None => name_line.into_any_element(),
    };
    div()
        .flex()
        .flex_col()
        .gap(rpx(2.))
        .child(match identity {
            Some(editor) => super::identity_editor::name_field(
                app,
                editor,
                name_line,
                name.clone(),
                name_style,
                palette,
                cx,
            ),
            None => name_line,
        })
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(rpx(6.))
                .gap_y(rpx(if large { 4. } else { 2. }))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(palette.text))
                .children(shown_username.map(|username| div().child(username)))
                .map(|this| match identity {
                    Some(editor) => this
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_x(rpx(2.))
                                .child(dot())
                                .child(super::identity_editor::pronouns_field(
                                    app, editor, pronouns, palette, cx,
                                )),
                        )
                        .child(super::identity_editor::tag_pill(app, editor, palette, cx)),
                    None => this
                        .children(pronouns.map(|pronouns| {
                            div()
                                .flex()
                                .items_center()
                                .gap_x(rpx(6.))
                                .child(dot())
                                .child(
                                    div()
                                        .font_weight(FontWeight::NORMAL)
                                        .text_color(rgb(palette.strong))
                                        .child(pronouns),
                                )
                        }))
                        .children(tag.as_ref().map(|tag| {
                            theme::guild_tag(tag, cx)
                                .bg(palette.panel)
                                .text_color(rgb(palette.strong))
                        })),
                })
                .children(inline_badges),
        )
        .children(badge_line.map(|badges| div().flex().mt(rpx(6.)).child(badges)))
        .when(
            !large && card.user_id != app.store.current_user_id,
            |this| {
                this.children(profile.and_then(|profile| mutuals_line(app, profile, palette, cx)))
            },
        )
}
