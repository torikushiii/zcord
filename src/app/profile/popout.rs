use super::badges::badges;
use super::frame::{frame_overflow, framed};
use super::loading::guild_profile;
use super::mutuals::mutuals_line;
use super::*;

pub(super) fn popout(
    app: &DiscordApp,
    card: &ProfileCard,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let theme = card.theme();
    let palette = card.palette();
    let layout = Layout::popout(theme.is_some());
    let surface = theme.map_or(theme::ELEVATED, ProfileTheme::surface);
    let entity = app.entity.clone();
    let open_full = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
    };
    let inner = div()
        .relative()
        .flex()
        .flex_col()
        .rounded(rpx(if theme.is_some() { 6. } else { 8. }))
        .overflow_hidden()
        .pb(rpx(4.))
        .text_color(rgb(palette.text))
        .map(|this| match theme {
            Some(theme) => this.bg(theme.overlay()),
            None => this,
        })
        .child(header(
            app,
            card,
            &layout,
            surface,
            Some(Box::new(open_full)),
            cx,
        ))
        .child(popout_body(app, card, &layout, cx))
        .children(effect_layers(card, layout.width, cx));
    let outer_width = POPOUT_WIDTH + theme.map_or(0., |_| THEME_BORDER * 2.);
    let outer = div()
        .relative()
        .w(rpx(outer_width))
        .rounded(rpx(8.))
        .shadow_lg()
        .map(|this| match theme {
            Some(theme) => this.p(rpx(THEME_BORDER)).bg(theme.outer()),
            None => this
                .bg(rgb(theme::ELEVATED))
                .border_1()
                .border_color(rgb(theme::BORDER)),
        })
        .child(inner)
        .into_any_element();
    let framed = framed(card, outer_width, outer, false, cx);
    let close = app.entity.clone();
    // Keep the frame art that hangs past the card on screen, not just the card itself.
    let overflow = frame_overflow(card, outer_width).unwrap_or_default();
    let margin = |overflow: f32| rpx(overflow).to_pixels(window.rem_size()) + px(12.);
    deferred(
        anchored()
            .position(card.position)
            .anchor(card.anchor)
            .snap_to_window_with_margin(Edges {
                top: margin(overflow.top),
                bottom: margin(overflow.bottom),
                left: margin(overflow.left),
                right: margin(overflow.right),
            })
            .child(
                div()
                    .id("profile-popout")
                    .relative()
                    .occlude()
                    .on_mouse_down_out(move |_, _, cx| {
                        close.update(cx, |app, cx| app.close_profile(cx)).ok();
                    })
                    .child(framed)
                    .with_animation(
                        ("profile-popout-enter", card.user_id),
                        Animation::new(ENTER).with_easing(ease_out_quint()),
                        |this, delta| this.opacity(delta),
                    ),
            ),
    )
    .with_priority(1)
    .into_any_element()
}

pub(super) type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub(super) fn header(
    app: &DiscordApp,
    card: &ProfileCard,
    layout: &Layout,
    surface: u32,
    on_avatar: Option<ClickHandler>,
    cx: &App,
) -> AnyElement {
    let user = app.store.user(card.user_id);
    let palette = card.palette();
    let status = user
        .filter(|user| !user.playing)
        .and_then(|user| user.activity.clone());
    // Your own profile shows an "add status" prompt in place of an empty bubble.
    let prompt = status.is_none() && card.user_id == app.store.current_user_id;
    let status = status
        .map(|text| (text, false))
        .or_else(|| prompt.then(|| (STATUS_PROMPTS[card.status_prompt].to_owned(), true)));
    let avatar = div()
        .id("profile-avatar")
        .absolute()
        .left(rpx(layout.avatar_left - layout.ring))
        .top(rpx(layout.banner - layout.avatar / 2. - layout.ring))
        .p(rpx(layout.ring))
        .rounded_full()
        .bg(rgb(surface))
        .child(theme::member_avatar(
            &app.store,
            card.guild_id.unwrap_or(0),
            card.user_id,
            layout.avatar,
            user.map(|user| (user.status, surface)),
            true,
            cx,
        ))
        .map(|this| match on_avatar {
            Some(handler) => this.cursor_pointer().on_click(handler),
            None => this,
        });
    let status_height = status.as_ref().map_or(0., |(text, prompt)| {
        if text.chars().count() > if *prompt { 18 } else { 22 } {
            54.
        } else {
            36.
        }
    });
    let height = layout
        .header_min
        .max(layout.banner - STATUS_OVERLAP + status_height);
    div()
        .relative()
        .w_full()
        .h(rpx(height))
        .child(banner(card, layout, cx))
        .child(avatar)
        .children(
            status
                .map(|(text, prompt)| status_bubble(card.user_id, &text, prompt, layout, palette)),
        )
        .into_any_element()
}

fn banner(card: &ProfileCard, layout: &Layout, cx: &App) -> impl IntoElement {
    let profile = card.profile.as_deref();
    let spec = profile.and_then(|p| {
        let member_banner = p.guild_member.as_ref().and_then(|m| m.banner.clone());
        match (member_banner, card.guild_id) {
            (Some(hash), Some(guild_id)) => Some(ImageSpec::GuildMemberBanner {
                guild_id,
                user_id: card.user_id,
                hash,
            }),
            _ => p.user.banner.clone().map(|hash| ImageSpec::Banner {
                user_id: card.user_id,
                hash,
            }),
        }
    });
    let pixels = BANNER_SOURCE;
    let corner = rpx(layout.corner);
    let image = spec.and_then(|spec| {
        let animated = spec
            .animated()
            .and_then(|animated| animated::resolve(&animated, pixels, cx));
        Some(match animated {
            Some(image) => animated::image(image)
                .size_full()
                .rounded_t(corner)
                .object_fit(ObjectFit::Cover)
                .id("profile-banner")
                .into_any_element(),
            None => img(images::resolve(&spec, pixels, cx)?)
                .size_full()
                .rounded_t(corner)
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
        })
    });
    let color = profile
        .and_then(|p| p.user.banner_color.or(p.user.accent_color))
        .or_else(|| profile?.user_profile.as_ref()?.accent_color)
        .or_else(|| card.theme().map(|theme| theme.primary))
        .unwrap_or(theme::SELECTED);
    div()
        .w_full()
        .h(rpx(layout.banner))
        .rounded_t(corner)
        .bg(rgb(color))
        .children(image)
}

fn status_bubble(
    user_id: u64,
    text: &str,
    prompt: bool,
    layout: &Layout,
    palette: ProfilePalette,
) -> impl IntoElement + use<> {
    let surface = if palette.light {
        0xF1EDF5
    } else {
        SURFACE_HIGHEST
    };
    let bubble = |size: f32| {
        div()
            .absolute()
            .size(rpx(size))
            .rounded_full()
            .bg(rgb(surface))
            .border_1()
            .border_color(palette.border)
    };
    div()
        .absolute()
        .left(rpx(layout.status_left))
        .top(rpx(layout.banner - 7.))
        .right(rpx(12.))
        .flex()
        .child(
            // Thought circles come first so the bubble paints over them,
            // hiding their lower half behind its edge.
            div()
                .relative()
                .child(bubble(20.).left(rpx(10.)).top(rpx(-8.)))
                .child(bubble(10.).left(rpx(-3.)).top(rpx(-15.)))
                .child(status_body(user_id, text, prompt, surface, palette)),
        )
}

fn status_body(
    user_id: u64,
    text: &str,
    prompt: bool,
    surface: u32,
    palette: ProfilePalette,
) -> Div {
    div()
        .max_w(rpx(181.))
        .min_w(rpx(42.))
        .px(rpx(12.))
        .py(rpx(8.))
        .rounded(rpx(12.))
        .bg(rgb(surface))
        .border_1()
        .border_color(palette.border)
        .shadow_sm()
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(rgb(palette.text))
        .map(|this| {
            if prompt {
                // Will open a set-status dialog once one exists.
                this.cursor_pointer()
                    .flex()
                    .items_start()
                    .gap(rpx(4.))
                    .child(theme::icon("circle-plus-filled", 16., palette.muted).mt(rpx(1.)))
                    .child(
                        div()
                            .line_clamp(2)
                            .italic()
                            .text_color(rgb(palette.muted))
                            .child(text.to_owned()),
                    )
            } else {
                this.child(
                    div()
                        .line_clamp(2)
                        .child(wrapped_label(("profile-status", user_id), text)),
                )
            }
        })
}

pub(super) fn display_names(
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
    let name = app
        .store
        .member_name(guild_id, card.user_id)
        .map(str::to_owned)
        .or_else(|| user.map(|user| user.name.clone()))
        .unwrap_or_else(|| "Unknown User".to_owned());
    let username = profile
        .map(|p| p.user.user.username.clone())
        .filter(|username| !username.is_empty());
    let pronouns = profile
        .and_then(guild_profile)
        .and_then(|p| p.pronouns.clone())
        .filter(|p| !p.is_empty())
        .or_else(|| profile?.user_profile.as_ref()?.pronouns.clone())
        .filter(|p| !p.is_empty());
    let name_style = app.store.member_name_style(guild_id, card.user_id);
    let dot = || {
        div()
            .size(rpx(4.))
            .mx(rpx(1.))
            .rounded_full()
            .bg(rgb(palette.text))
    };
    div()
        .flex()
        .flex_col()
        .gap(rpx(2.))
        .child(
            div()
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
                .child(theme::display_name(&name, name_style, palette.strong)),
        )
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
                .children(username.map(|username| div().child(username)))
                .when(user.is_some_and(|user| user.bot), |this| {
                    this.child(theme::app_tag())
                })
                .children(pronouns.map(|pronouns| {
                    div()
                        .flex()
                        .items_center()
                        .gap_x(rpx(6.))
                        .child(dot())
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.strong))
                                .child(pronouns),
                        )
                }))
                .children(user.and_then(|user| user.tag.as_ref()).map(|tag| {
                    theme::guild_tag(tag, cx)
                        .bg(palette.panel)
                        .text_color(rgb(palette.strong))
                }))
                .children(profile.and_then(|profile| badges(app, card, profile, palette, cx))),
        )
        .when(!large && card.user_id != app.store.current_user_id, |this| {
            this.children(profile.and_then(|profile| mutuals_line(app, profile, palette, cx)))
        })
}

fn popout_body(
    app: &DiscordApp,
    card: &ProfileCard,
    layout: &Layout,
    cx: &App,
) -> impl IntoElement {
    let user = app.store.user(card.user_id);
    let palette = card.palette();
    let entity = app.entity.clone();
    div()
        .flex()
        .flex_col()
        .gap(rpx(12.))
        .px(rpx(layout.padding))
        .pt(rpx(4.))
        .pb(rpx(8.))
        .child(display_names(
            app,
            card,
            false,
            Some(Box::new(move |_, _, cx| {
                entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
            })),
            cx,
        ))
        .children(bio(app, card, true, cx))
        .children(board::popout_widget(app, card, cx))
        .children(
            board::popout_playing(app, card, cx)
                .or_else(|| user.and_then(|user| playing(user, palette))),
        )
        .children(roles(app, card))
}

pub(super) fn bio(
    app: &DiscordApp,
    card: &ProfileCard,
    clamped: bool,
    cx: &App,
) -> Option<AnyElement> {
    let profile = card.profile.as_deref()?;
    let palette = card.palette();
    let text = guild_profile(profile)
        .and_then(|p| p.bio.clone())
        .filter(|bio| !bio.trim().is_empty())
        .or_else(|| profile.user_profile.as_ref()?.bio.clone())
        .filter(|bio| !bio.trim().is_empty())?;
    let parsed = app.parsed(card.user_id, &text);
    let context = BodyContext {
        store: &app.store,
        guild_id: card.guild_id.unwrap_or(0),
        message_id: card.user_id,
        revealed: &app.revealed_spoilers,
        entity: app.entity.clone(),
        colors: BodyColors {
            strong: palette.strong,
            muted: palette.muted,
            faint: palette.faint,
            link: palette.light.then_some(0x154C8A),
        },
    };
    Some(
        div()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .text_color(rgb(palette.strong))
            .when(clamped, |this| {
                this.max_h(rpx(18. * BIO_LINES as f32)).overflow_hidden()
            })
            .child(render_body(&parsed.blocks, false, false, &context, cx))
            .into_any_element(),
    )
}

pub(super) fn playing(user: &User, palette: ProfilePalette) -> Option<AnyElement> {
    let activity = user.activity.as_ref().filter(|_| user.playing)?;
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

pub(super) fn roles(app: &DiscordApp, card: &ProfileCard) -> Option<AnyElement> {
    let palette = card.palette();
    let roles = app.store.member_roles(card.guild_id?, card.user_id);
    if roles.is_empty() {
        return None;
    }
    Some(section(
        "Roles",
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
        palette,
    ))
}

pub(super) fn section(
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

pub(super) fn member_since(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
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
    section(
        "Member Since",
        div()
            .flex()
            .items_center()
            .gap(rpx(8.))
            .children(created.map(|date| {
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(4.))
                    .child(theme::icon("discord", 16., palette.muted))
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
            .into_any_element(),
        palette,
    )
}
