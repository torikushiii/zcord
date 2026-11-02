use gpui_component::scroll::ScrollableElement as _;
use std::time::SystemTime;

use crate::app::{
    message_body::emoji_pieces,
    rich_text::{Content, EmojiSource, Piece, PieceStyle, rich_text},
};
use crate::store::{CustomStatus, StatusEmoji};

use super::badges::badges;
use super::frame::{frame_overflow, framed};
use super::loading::guild_profile;
use super::mutuals::mutuals_line;
use super::*;

mod menu;
pub(super) mod status;

pub(super) use menu::profile_actions;

pub(super) fn popout(
    app: &DiscordApp,
    card: &ProfileCard,
    window: &Window,
    cx: &App,
) -> AnyElement {
    let theme = card.theme();
    let palette = card.palette();
    let layout = Layout::popout(theme.is_some());
    let corner = if theme.is_some() { 6. } else { 8. };
    let surface = theme.map_or(theme::ELEVATED, ProfileTheme::surface);
    let outer_width = POPOUT_WIDTH + theme.map_or(0., |_| THEME_BORDER * 2.);
    let overflow = frame_overflow(card, outer_width).unwrap_or_default();
    let position = card.popout_position(window, &overflow);
    let entity = app.entity.clone();
    let open_full = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        entity.update(cx, |app, cx| app.open_full_profile(cx)).ok();
    };
    let inner = div()
        .relative()
        .flex()
        .flex_col()
        .rounded(rpx(corner))
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
        .child(popout_body(app, card, &layout, window, cx))
        .children(effect_layers(card, layout.width, corner, false, cx));
    let inner = match card.account_max_height(window, &overflow) {
        Some(height) => {
            let inset = match theme {
                Some(_) => rpx(THEME_BORDER * 2.).to_pixels(window.rem_size()),
                None => px(2.),
            };
            // Scroll the account content instead of letting window snapping cover its anchor.
            div()
                .id("account-popout-scroll")
                .flex()
                .flex_col()
                .max_h((height - inset).max(px(0.)))
                .rounded(rpx(corner))
                .overflow_hidden()
                .overflow_y_scrollbar()
                .child(inner.flex_none())
                .into_any_element()
        }
        None => inner.into_any_element(),
    };
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
                .border_color(rgba(theme::BORDER)),
        })
        .child(inner)
        .into_any_element();
    let framed = framed(card, outer_width, outer, false, cx);
    let close = app.entity.clone();
    let in_status_menu = card.status_menu.hit_test();
    // Keep the frame art that hangs past the card on screen, not just the card itself.
    let margin = |overflow: f32| rpx(overflow).to_pixels(window.rem_size()) + px(12.);
    deferred(
        anchored()
            .position(position)
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
                    .on_mouse_down_out(move |event, _, cx| {
                        if in_status_menu(event.position) {
                            return;
                        }
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
    let status = active_custom_status(
        user.and_then(|user| user.custom_status.as_ref()),
        SystemTime::now(),
    )
    .cloned();
    // Your own profile shows an "add status" prompt in place of an empty bubble.
    let prompt = status.is_none() && card.user_id == app.store.current_user_id;
    let status = status.or_else(|| {
        prompt.then(|| CustomStatus {
            text: Some(STATUS_PROMPTS[card.status_prompt].to_owned()),
            emoji: None,
            expires_at: None,
        })
    });
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
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .w_full()
        .min_h(rpx(layout.header_min))
        .child(banner(card, layout, cx))
        .child(avatar)
        .children(
            status.map(|status| status_bubble(card.user_id, &status, prompt, layout, palette, cx)),
        )
        .into_any_element()
}

// An emoji alone is a complete status, even while its image is still loading.
fn active_custom_status(status: Option<&CustomStatus>, now: SystemTime) -> Option<&CustomStatus> {
    status.filter(|status| {
        !status.is_expired(now)
            && (status.emoji.is_some()
                || status
                    .text
                    .as_deref()
                    .is_some_and(|text| !text.trim().is_empty()))
    })
}

pub(super) fn banner_spec(card: &ProfileCard) -> Option<ImageSpec> {
    card.profile.as_deref().and_then(|p| {
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
    })
}

fn banner(card: &ProfileCard, layout: &Layout, cx: &App) -> AnyElement {
    let profile = card.profile.as_deref();
    let spec = banner_spec(card);
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
        .unwrap_or(theme::SURFACE_HIGHEST);
    let banner = div()
        .flex_none()
        .w_full()
        .h(rpx(layout.banner))
        .rounded_t(corner)
        .bg(rgb(color))
        .children(image);
    if card.loading() {
        pulse(banner, "profile-banner-pulse", card.user_id).into_any_element()
    } else {
        banner.into_any_element()
    }
}

fn status_bubble(
    user_id: u64,
    status: &CustomStatus,
    prompt: bool,
    layout: &Layout,
    palette: ProfilePalette,
    cx: &App,
) -> impl IntoElement + use<> {
    let shadow = || {
        vec![
            BoxShadow::new(
                px(0.),
                px(1.),
                hsla(0., 0., 0., if palette.light { 0.08 } else { 0.14 }),
            )
            .blur_radius(px(4.)),
        ]
    };
    let bubble = |size: f32| {
        div()
            .absolute()
            .size(rpx(size))
            .rounded_full()
            .bg(rgb(palette.status_surface))
            .border_1()
            .border_color(palette.status_border)
            .shadow(shadow())
    };
    div()
        .relative()
        .flex_none()
        .ml(rpx(layout.status_left))
        .mr(rpx(layout.status_right))
        .mt(rpx(-layout.status_overlap))
        .flex()
        .child(
            div()
                .relative()
                .max_w(rpx(181.))
                .min_w(rpx(42.))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(rpx(16.))
                        .bg(rgb(palette.status_surface))
                        .border_1()
                        .border_color(palette.status_border)
                        .shadow(shadow()),
                )
                .child(bubble(20.).left(rpx(10.)).top(rpx(-8.)))
                .child(bubble(10.).left(rpx(-3.)).top(rpx(-15.)))
                // The inner fill covers the circles inside the outline without clipping
                // the portions that extend above it, just like Discord's nested span.
                .child(
                    div()
                        .relative()
                        .rounded(rpx(16.))
                        .overflow_hidden()
                        .border_1()
                        .border_color(hsla(0., 0., 0., 0.))
                        .child(status_body(user_id, status, prompt, palette, cx)),
                ),
        )
}

fn status_emoji_source(emoji: &StatusEmoji, size: f32, cx: &App) -> Option<ImageSource> {
    match emoji {
        StatusEmoji::Unicode(sequence) => crate::text::emoji::image(sequence).map(Into::into),
        StatusEmoji::Custom { id, animated, .. } => images::resolve(
            &ImageSpec::Emoji {
                id: *id,
                animated: *animated && crate::settings::emoji_animation(),
            },
            theme::image_px(size),
            cx,
        )
        .map(Into::into),
    }
}

fn status_body(
    user_id: u64,
    status: &CustomStatus,
    prompt: bool,
    palette: ProfilePalette,
    cx: &App,
) -> Div {
    let text = status.text.as_deref().unwrap_or_default();
    let body = div()
        .min_w_0()
        .px(rpx(12.))
        .py(rpx(8.))
        .rounded(rpx(15.))
        .bg(rgb(palette.status_surface))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::NORMAL)
        .text_color(palette.status_text);
    if prompt {
        // Will open a set-status dialog once one exists.
        return body
            .cursor_pointer()
            .flex()
            .items_start()
            .gap(rpx(4.))
            .child(theme::icon("circle-plus-filled", 16., palette.muted).mt(rpx(1.)))
            .child(
                div()
                    .min_w_0()
                    .line_clamp(2)
                    .italic()
                    .text_color(rgb(palette.muted))
                    .child(text.to_owned()),
            );
    }
    if text.trim().is_empty() {
        return body.child(
            div().size(rpx(32.)).children(
                status
                    .emoji
                    .as_ref()
                    .and_then(|emoji| status_emoji_source(emoji, 32., cx))
                    .map(|source| {
                        img(source)
                            .id(("profile-status-emoji", user_id))
                            .size_full()
                            .object_fit(ObjectFit::Contain)
                    }),
            ),
        );
    }
    let mut pieces = emoji_pieces(text, &PieceStyle::default());
    // As an inline piece, the emoji only indents the first line, not every wrapped line.
    if let Some(emoji) = status.emoji.as_ref() {
        let fallback = match emoji {
            StatusEmoji::Unicode(sequence) => sequence.clone(),
            StatusEmoji::Custom { name, .. } => format!(":{name}:"),
        };
        pieces.insert(
            0,
            Piece {
                content: Content::Emoji(
                    status_emoji_source(emoji, 16., cx).map(EmojiSource::Still),
                    fallback.into(),
                ),
                style: PieceStyle::default(),
            },
        );
    }
    let content = rich_text(("profile-status", user_id), pieces)
        .emoji_size(rpx(16.))
        .emoji_gap(rpx(4.))
        .line_height(rpx(18.))
        .line_clamp(2)
        .selectable();
    body.cursor_text().child(
        div()
            .min_w_0()
            .max_h(rpx(36.))
            .overflow_hidden()
            .child(content),
    )
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
        // Discord renders pronouns as one line of HTML text, so newlines padding them out
        // collapse to spaces there.
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
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
                .text_color(rgb(palette.strong))
                .child(crate::app::display_name::render(
                    ("profile-styled-name", card.user_id),
                    &name,
                    name_style,
                    crate::app::display_name::Surface::Profile,
                    cx,
                )),
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
        .when(
            !large && card.user_id != app.store.current_user_id,
            |this| {
                this.children(profile.and_then(|profile| mutuals_line(app, profile, palette, cx)))
            },
        )
}

fn popout_body(
    app: &DiscordApp,
    card: &ProfileCard,
    layout: &Layout,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
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
        .map(|this| {
            if card.loading() {
                return this.child(skeleton(card.user_id));
            }
            this.children(bio(app, card, true, cx))
                .children(board_sections(app, card, cx))
                .when(!card.is_account(), |this| this.children(roles(app, card)))
        })
        .when(card.is_account(), |this| {
            this.child(account_menu(app, card, window, cx))
        })
}

pub(super) fn board_sections(
    app: &DiscordApp,
    card: &ProfileCard,
    cx: &App,
) -> impl Iterator<Item = AnyElement> {
    let user = app.store.user(card.user_id);
    let playing = board::popout_playing(app, card, cx)
        .or_else(|| user.and_then(|user| playing(user, card.palette())));
    [
        board::popout_widget(app, card, cx),
        playing,
        board::listening(app, card, false, cx),
    ]
    .into_iter()
    .flatten()
}

fn account_menu(app: &DiscordApp, card: &ProfileCard, window: &Window, cx: &App) -> AnyElement {
    let palette = card.palette();
    let row = |id: &'static str, icon: &'static str, text: &'static str, action: AccountAction| {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(rpx(12.))
            .min_h(rpx(48.))
            .px(rpx(12.))
            .py(rpx(12.))
            .rounded(rpx(8.))
            .cursor_pointer()
            .text_size(rpx(14.))
            .line_height(rpx(20.))
            .text_color(rgb(palette.text))
            .hover(|style| style.bg(palette.hover).text_color(rgb(palette.strong)))
            .without_text_selection()
            .on_click(account_action(app, action))
            .child(theme::icon(icon, 18., palette.muted))
            .child(div().flex_1().min_w_0().truncate().child(text))
    };
    div()
        .flex()
        .flex_col()
        .gap(rpx(12.))
        .child(
            div()
                .rounded(rpx(8.))
                .bg(palette.panel)
                .child(row(
                    "account-view-profile",
                    "user",
                    "View Profile",
                    AccountAction::ViewProfile,
                ))
                .child(div().h(rpx(1.)).mx(rpx(12.)).bg(palette.border))
                .child(status::trigger(app, card, window, cx))
                .child(div().h(rpx(1.)).mx(rpx(12.)).bg(palette.border))
                .child(row(
                    "account-settings",
                    "settings",
                    "User Settings",
                    AccountAction::Settings,
                )),
        )
        .child(div().rounded(rpx(8.)).bg(palette.panel).child(row(
            "account-copy-user-id",
            "copy",
            "Copy User ID",
            AccountAction::CopyUserId,
        )))
        .into_any_element()
}

/// Stands in for the bio and roles while a profile nobody has opened before is fetched.
pub(super) fn skeleton(user_id: u64) -> impl IntoElement {
    let bar = |width: DefiniteLength, height: f32| {
        div()
            .w(width)
            .h(rpx(height))
            .rounded(rpx(height / 2.))
            .bg(rgba(theme::SELECTED))
    };
    let pill = |width: f32| bar(rpx(width).into(), 20.);
    let content = div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .child(bar(relative(1.), 12.))
        .child(bar(relative(0.85), 12.))
        .child(bar(relative(0.55), 12.))
        .child(div().h(rpx(4.)))
        .child(bar(rpx(48.).into(), 12.))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(rpx(4.))
                .child(pill(72.))
                .child(pill(56.))
                .child(pill(88.)),
        );
    pulse(content, "profile-skeleton-pulse", user_id)
}

fn pulse<E: IntoElement + Styled + 'static>(
    element: E,
    key: &'static str,
    user_id: u64,
) -> AnimationElement<E> {
    element.with_animation(
        (key, user_id),
        Animation::new(Duration::from_millis(1200))
            .repeat()
            .with_easing(pulsating_between(0.45, 1.)),
        |this, delta| this.opacity(delta),
    )
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
pub(super) fn markdown(
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
        entity: app.entity.clone(),
        colors: BodyColors {
            strong: palette.strong,
            muted: palette.muted,
            faint: palette.faint,
            link: palette.light.then_some(0x154C8A),
        },
    };
    render_body(&parsed.blocks, false, false, &context, cx).into_any_element()
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
            .into_any_element(),
        palette,
    )
}

#[cfg(test)]
mod status_tests {
    use super::active_custom_status;
    use crate::store::{CustomStatus, StatusEmoji};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn custom_status_accepts_text_and_emoji_only() {
        let now = UNIX_EPOCH + Duration::from_secs(60);
        let mut status = CustomStatus {
            text: Some("My custom status".to_owned()),
            emoji: None,
            expires_at: None,
        };
        assert_eq!(active_custom_status(Some(&status), now), Some(&status));

        status.text = None;
        status.emoji = Some(StatusEmoji::Custom {
            id: 123,
            name: "status_emoji".to_owned(),
            animated: true,
        });
        assert_eq!(active_custom_status(Some(&status), now), Some(&status));

        status.emoji = Some(StatusEmoji::Unicode("\u{1F680}".to_owned()));
        assert_eq!(active_custom_status(Some(&status), now), Some(&status));
    }

    #[test]
    fn custom_status_omits_missing_empty_and_expired_values() {
        let now = UNIX_EPOCH + Duration::from_secs(60);
        assert!(active_custom_status(None, now).is_none());

        let status = CustomStatus {
            text: Some(" \n ".to_owned()),
            emoji: None,
            expires_at: None,
        };
        assert!(active_custom_status(Some(&status), now).is_none());

        let status = CustomStatus {
            text: Some("Expires now".to_owned()),
            expires_at: Some(now),
            ..status
        };
        assert!(active_custom_status(Some(&status), now).is_none());
        assert_eq!(
            active_custom_status(Some(&status), now - Duration::from_secs(1)),
            Some(&status)
        );
    }
}
