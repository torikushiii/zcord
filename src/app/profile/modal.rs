use super::connections::connections;
use super::frame::framed;
use super::popout::{
    banner_spec, bio, display_names, header, member_since, playing, profile_actions, roles, section,
};
use super::*;
use gpui_component::scroll::ScrollableElement as _;

/// The Wishlist tab is hidden for now; its panel and loading stay in place.
const WISHLIST_TAB: bool = false;

pub(super) fn profile_close_button(palette: ProfilePalette) -> Stateful<Div> {
    let (background, hover) = if palette.light {
        (hsla(0., 0., 1., 0.6), hsla(0., 0., 1., 0.85))
    } else {
        (rgba(0x00000066).into(), palette.hover)
    };
    theme::icon_button_with_hover("profile-close", "x", 18., palette.text, hover).bg(background)
}

pub(super) fn modal_backdrop_corners(rem_size: Pixels) -> Corners<Pixels> {
    let radius = rpx(MODAL_RADIUS).to_pixels(rem_size);
    Corners {
        top_left: radius,
        top_right: radius,
        ..Corners::default()
    }
}

pub(super) fn modal(app: &DiscordApp, card: &ProfileCard, window: &Window, cx: &App) -> AnyElement {
    let theme = card.theme();
    let palette = card.palette();
    let css = |pixels: Pixels| pixels / window.rem_size() * theme::BASE_FONT_SIZE;
    let viewport = window.viewport_size();
    let (viewport, viewport_height) = (css(viewport.width), css(viewport.height));
    let windowed = viewport >= MODAL_MIN_VIEWPORT_WIDTH;
    let modal_width = if windowed {
        MODAL_WIDTH.min(viewport - MODAL_MARGIN * 2.)
    } else {
        viewport
    };
    let modal_height = if viewport_height >= MODAL_MIN_VIEWPORT_HEIGHT {
        MODAL_HEIGHT.min(viewport_height - MODAL_MARGIN * 2.)
    } else {
        viewport_height
    };
    let card_inset = if windowed { MODAL_MARGIN * 0.91 } else { 0. };
    let card_width = (viewport * 0.455 - card_inset).clamp(300., 400.);
    let layout = Layout::modal(card_width);
    let surface = theme.map_or(theme::elevated(), ProfileTheme::surface);
    let background: Background =
        theme.map_or(rgb(theme::bg()).into(), ProfileTheme::modal_background);
    let profile_card = div()
        .relative()
        .w(rpx(card_width))
        .h_full()
        .flex()
        .flex_col()
        .rounded_t(rpx(MODAL_CARD_RADIUS))
        .shadow(vec![
            BoxShadow::new(
                px(0.),
                px(2.),
                hsla(0., 0., 0., if palette.light { 0.16 } else { 0.3 }),
            )
            .blur_radius(px(8.)),
        ])
        .overflow_hidden()
        .map(|this| match theme {
            Some(theme) => this.bg(theme.outer()),
            None => this.bg(rgb(theme::elevated())),
        })
        .child(
            div()
                .id("profile-modal-card")
                .flex()
                .flex_col()
                .size_full()
                .rounded_t(rpx(layout.corner))
                .text_color(rgb(palette.text))
                .map(|this| match theme {
                    Some(theme) => this.bg(theme.overlay()),
                    None => this,
                })
                .child(header(app, card, &layout, surface, None, cx))
                .child(
                    div()
                        .id("profile-modal-body")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .px(rpx(layout.padding))
                        .pb(rpx(layout.padding))
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_none()
                                .gap(rpx(24.))
                                .child(display_names(app, card, true, None, cx))
                                .children(profile_actions(app, card, window))
                                .children(
                                    bio(app, card, false, cx)
                                        .map(|bio| section("About Me", bio, palette)),
                                )
                                .child(member_since(app, card, cx))
                                .children(roles(app, card))
                                .children(
                                    card.profile
                                        .as_deref()
                                        .and_then(|profile| connections(profile, palette)),
                                )
                                .children(card.note.as_ref().map(|note| {
                                    section(
                                        "Note",
                                        div()
                                            .text_size(rpx(14.))
                                            .line_height(rpx(18.))
                                            .text_color(rgb(palette.text))
                                            .child(note.clone())
                                            .into_any_element(),
                                        palette,
                                    )
                                })),
                        ),
                ),
        )
        .children(effect_layers(card, card_width, layout.corner, true, cx))
        .into_any_element();
    let framed = framed(card, card_width, profile_card, true, cx);
    let banner_backdrop = banner_spec(card)
        .and_then(|spec| {
            let sigma = BACKDROP_BLUR * BACKDROP_SOURCE as f32 / modal_width;
            images::blurred_with(&spec, BACKDROP_SOURCE, sigma, BACKDROP_SATURATION, cx)
        })
        .map(|image| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(rpx(BACKDROP_HEIGHT))
                .overflow_hidden()
                .opacity(BACKDROP_OPACITY)
                .child(
                    canvas(
                        move |_, _, _| image,
                        |bounds, image, window, _| {
                            if bounds.is_empty() {
                                return;
                            }
                            let fitted = ObjectFit::Cover.get_bounds(bounds, image.size(0));
                            // Fade the image itself, leaving the profile theme visible underneath.
                            let fade = EdgeFade {
                                bounds,
                                band: bounds.size.height,
                                band_top: None,
                                band_bottom: None,
                                band_left: None,
                                band_right: None,
                                top: false,
                                bottom: true,
                                left: false,
                                right: false,
                            };
                            window.with_edge_fade(Some(fade), |window| {
                                if let Err(error) = window.paint_image_fitted(
                                    bounds,
                                    fitted,
                                    // Parent overflow clipping is rectangular, so round the image too.
                                    modal_backdrop_corners(window.rem_size()),
                                    image,
                                    0,
                                    false,
                                ) {
                                    log::warn!("Could not paint profile backdrop: {error}");
                                }
                            });
                        },
                    )
                    .size_full(),
                )
        });
    let close = app.entity.clone();
    let dismiss = app.entity.clone();
    let dialog = div()
        .id("profile-modal")
        .relative()
        .w(rpx(modal_width))
        .h(rpx(modal_height))
        .rounded(rpx(MODAL_RADIUS))
        .text_color(rgb(palette.text))
        .occlude()
        .without_text_selection()
        .on_click(|_, _, cx| cx.stop_propagation())
        // Clip the dialog surface, not the frame layers that extend beyond it.
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(rpx(MODAL_RADIUS))
                .overflow_hidden()
                .bg(background)
                .children(banner_backdrop),
        )
        .child(div().absolute().top(rpx(8.)).right(rpx(8.)).child(
            profile_close_button(palette).on_click(move |_, _, cx| {
                close.update(cx, |app, cx| app.close_profile(cx)).ok();
            }),
        ))
        .child(
            div()
                .size_full()
                .flex()
                .pt(rpx(32.))
                .pl(rpx(32.))
                .pr(rpx(16.))
                .child(framed)
                .child(side_panel(app, card, cx)),
        );
    div()
        .id("profile-modal-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_click(move |_, _, cx| {
            dismiss.update(cx, |app, cx| app.close_profile(cx)).ok();
        })
        .with_animation(
            ("profile-modal-backdrop", card.user_id),
            Animation::new(ENTER).with_easing(ease_out_quint()),
            |this, delta| this.bg(rgba(theme::SCRIM).opacity(delta)),
        )
        .child(dialog.with_animation(
            ("profile-modal-enter", card.user_id),
            Animation::new(ENTER).with_easing(ease_out_quint()),
            |this, delta| this.opacity(delta).top(rpx(16. * (1. - delta))),
        ))
        .into_any_element()
}

fn side_panel(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    let own_profile = card.user_id == app.store.current_user_id;
    let profile = card.profile.as_deref().filter(|_| !own_profile);
    let palette = card.palette();
    let mutual_guilds: Vec<&model::MutualGuild> = profile
        .and_then(|p| p.mutual_guilds.as_ref())
        .into_iter()
        .flatten()
        .collect();
    let mutual_friends: Vec<&model::User> = profile
        .and_then(|p| p.mutual_friends.as_ref())
        .into_iter()
        .flatten()
        .collect();
    let friend_count = profile
        .and_then(|p| p.mutual_friends_count)
        .filter(|count| *count > 0)
        .map(|count| count as usize)
        .or_else(|| (!mutual_friends.is_empty()).then_some(mutual_friends.len()));
    let has_board = board::has_board(card);
    let has_wishlist = WISHLIST_TAB && board::has_wishlist(card);
    let current = match card.tab {
        Tab::Board if !has_board => Tab::Activity,
        Tab::Wishlist if !has_wishlist => Tab::Activity,
        Tab::Servers | Tab::Friends if own_profile => Tab::Activity,
        tab => tab,
    };
    let tab = |id: Tab, title: String| {
        let entity = app.entity.clone();
        let selected = current == id;
        div()
            .id(SharedString::from(format!("profile-tab-{}", id as u8)))
            .pb(rpx(12.))
            .border_b_2()
            .border_color(if selected {
                rgb(palette.strong)
            } else {
                rgba(0)
            })
            .cursor_pointer()
            .text_size(rpx(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(if selected {
                palette.strong
            } else {
                palette.muted
            }))
            .hover(move |style| style.text_color(rgb(palette.strong)))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.select_profile_tab(id, cx))
                    .ok();
            })
            .child(title)
    };
    let panel = match current {
        Tab::Board => board::board_panel(app, card, cx),
        Tab::Activity => activity_panel(app, card, cx),
        Tab::Wishlist => board::wishlist_panel(app, card, cx),
        Tab::Servers => servers_panel(app, &mutual_guilds, palette, cx),
        Tab::Friends => friends_panel(app, &mutual_friends, palette, cx),
    };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .pl(rpx(32.))
        .pt(rpx(20.))
        .child(
            div()
                .flex()
                .items_end()
                .gap(rpx(24.))
                .border_b_1()
                .border_color(palette.border)
                .when(has_board, |this| {
                    this.child(tab(Tab::Board, "Board".to_owned()))
                })
                .child(tab(Tab::Activity, "Activity".to_owned()))
                .when(has_wishlist, |this| {
                    this.child(tab(Tab::Wishlist, "Wishlist".to_owned()))
                })
                .when(!mutual_guilds.is_empty(), |this| {
                    this.child(tab(
                        Tab::Servers,
                        plural(mutual_guilds.len(), "Mutual Server"),
                    ))
                })
                .children(friend_count.map(|count| {
                    let title = plural(count, "Mutual Friend");
                    if mutual_friends.is_empty() {
                        div()
                            .pb(rpx(12.))
                            .text_size(rpx(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.faint))
                            .child(title)
                            .into_any_element()
                    } else {
                        tab(Tab::Friends, title).into_any_element()
                    }
                })),
        )
        .child(tab_panel(panel))
        .into_any_element()
}

pub(super) fn tab_panel(panel: AnyElement) -> Stateful<Div> {
    div()
        .id("profile-tab-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .pt(rpx(20.))
        .pb(rpx(32.))
        .pr(rpx(16.))
        .overflow_y_scroll()
        .child(panel)
}

fn activity_panel(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    if let Some(activity) = board::activity(app, card, cx) {
        return activity;
    }
    let palette = card.palette();
    let user = app.store.user(card.user_id);
    if let Some(playing) = user.and_then(|user| playing(user, palette)) {
        return playing;
    }
    let name = user.map_or("This user", |user| user.name.as_str());
    activity_empty(name, &ACTIVITY_PROMPTS[card.activity_prompt], palette).into_any_element()
}

pub(super) fn activity_empty(name: &str, prompt: &ActivityPrompt, palette: ProfilePalette) -> Div {
    div()
        .debug_selector(|| "profile-activity-empty".into())
        .flex()
        .flex_col()
        .flex_1()
        .w_full()
        .items_center()
        .justify_center()
        .px(rpx(16.))
        .py(rpx(24.))
        .child(
            div()
                .debug_selector(|| "profile-activity-copy".into())
                .flex()
                .flex_col()
                .w_full()
                .max_w(rpx(360.))
                .gap(rpx(8.))
                .text_center()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::NORMAL)
                .text_color(rgb(palette.text))
                .when(prompt.heading, |this| {
                    this.child(
                        div()
                            .debug_selector(|| "profile-activity-heading".into())
                            .w_full()
                            .font_weight(FontWeight::MEDIUM)
                            .child(format!("{name} doesn't have any activity to share here")),
                    )
                })
                .child(
                    div()
                        .debug_selector(|| "profile-activity-flavor".into())
                        .w_full()
                        .child(prompt.text),
                ),
        )
}

fn servers_panel(
    app: &DiscordApp,
    mutuals: &[&model::MutualGuild],
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .children(mutuals.iter().filter_map(|mutual| {
            let guild = app.store.guild_by_id(mutual.id)?;
            let icon = guild
                .icon_spec()
                .and_then(|spec| images::resolve(&spec, theme::image_px(40.), cx));
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(rpx(12.))
                    .p(rpx(8.))
                    .rounded(rpx(8.))
                    .hover(move |style| style.bg(palette.hover))
                    .child(match icon {
                        Some(path) => img(path)
                            .size(rpx(40.))
                            .rounded(rpx(12.))
                            .into_any_element(),
                        None => div()
                            .size(rpx(40.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(rpx(12.))
                            .bg(rgb(guild.icon_color))
                            .text_size(rpx(14.))
                            .text_color(gpui::black())
                            .child(guild.initials.clone())
                            .into_any_element(),
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rpx(16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(palette.strong))
                                    .child(label(("mutual-guild", guild.id), &guild.name)),
                            )
                            .children(mutual.nick.as_ref().map(|nick| {
                                div()
                                    .truncate()
                                    .text_size(rpx(12.))
                                    .text_color(rgb(palette.muted))
                                    .child(label(("mutual-nick", guild.id), nick))
                            })),
                    ),
            )
        }))
        .into_any_element()
}

fn friends_panel(
    app: &DiscordApp,
    friends: &[&model::User],
    palette: ProfilePalette,
    cx: &App,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .children(friends.iter().map(|friend| {
            let name = friend
                .global_name
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or(&friend.username);
            div()
                .flex()
                .items_center()
                .gap(rpx(12.))
                .p(rpx(8.))
                .rounded(rpx(8.))
                .hover(move |style| style.bg(palette.hover))
                .child(theme::user_avatar(
                    app.store.user(friend.id),
                    friend.id,
                    40.,
                    None,
                    false,
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(16.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.strong))
                                .child(label(("mutual-friend", friend.id), name)),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(12.))
                                .text_color(rgb(palette.muted))
                                .child(label(
                                    ("mutual-friend-username", friend.id),
                                    &friend.username,
                                )),
                        ),
                )
        }))
        .into_any_element()
}
