use super::super::badges::badges;
use super::super::popout::{
    add_app_button, app_section, banner, bio, command_chips, member_since_dates, message_button,
    more_button, role_chips,
};
use super::super::*;
use super::servers_panel;

const WIDTH: f32 = 600.;
const HEIGHT: f32 = 758.;
const RADIUS: f32 = 12.;
const BANNER: f32 = 210.;
const AVATAR: f32 = 120.;
const AVATAR_LEFT: f32 = 24.;
const AVATAR_TOP: f32 = 144.;
const RING: f32 = 6.;
const VIEWPORT_MARGIN: f32 = 16.;
const HELP_CENTER: &str = "https://support.discord.com/hc/articles/7933951485975";

pub(super) fn modal(app: &DiscordApp, card: &ProfileCard, window: &Window, cx: &App) -> AnyElement {
    let palette = card.palette();
    let css = |pixels: Pixels| pixels / window.rem_size() * theme::BASE_FONT_SIZE;
    let viewport = window.viewport_size();
    let width = WIDTH.min(css(viewport.width) - VIEWPORT_MARGIN * 2.);
    let height = HEIGHT.min(css(viewport.height) - VIEWPORT_MARGIN * 2.);
    let surface = theme::surface_high();
    let layout = Layout {
        width,
        banner: BANNER,
        avatar: AVATAR,
        avatar_left: AVATAR_LEFT,
        ring: RING,
        padding: 16.,
        status_left: 0.,
        status_right: 0.,
        status_overlap: 0.,
        header_min: 0.,
        corner: RADIUS,
    };
    let user = app.store.user(card.user_id);
    let avatar = div()
        .absolute()
        .left(rpx(AVATAR_LEFT - RING))
        .top(rpx(AVATAR_TOP - RING))
        .p(rpx(RING))
        .rounded_full()
        .bg(rgb(surface))
        .child(theme::member_avatar(
            &app.store,
            card.guild_id.unwrap_or(0),
            card.user_id,
            AVATAR,
            user.map(|user| (user.status, surface)),
            true,
            cx,
        ));
    let own = card.user_id == app.store.current_user_id;
    let header = div()
        .relative()
        .flex()
        .flex_col()
        .flex_none()
        .child(banner(app, card, &layout, cx))
        .when(!own, |this| {
            this.child(
                div()
                    .absolute()
                    .top(rpx(12.))
                    .right(rpx(16.))
                    .child(more_button(app, card, window, true)),
            )
        })
        .child(avatar)
        .child(
            div()
                .flex()
                .justify_end()
                .gap(rpx(8.))
                .h(rpx(22. + 32.))
                .pt(rpx(22.))
                .px(rpx(16.))
                .when(!own, |this| this.child(message_button(app, card, false)))
                .children(add_app_button(app, card, false)),
        )
        .child(names(app, card, cx));
    let close = app.entity.clone();
    let dialog = div()
        .id("app-profile-modal")
        .flex()
        .flex_col()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(RADIUS))
        .overflow_hidden()
        .bg(rgb(surface))
        .text_color(rgb(palette.text))
        .occlude()
        .without_text_selection()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(header)
        .child(tabs(app, card, cx));
    div()
        .id("profile-modal-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .occlude()
        .on_click(move |_, _, cx| {
            close.update(cx, |app, cx| app.close_profile(cx)).ok();
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

fn names(app: &DiscordApp, card: &ProfileCard, cx: &App) -> impl IntoElement {
    let palette = card.palette();
    let profile = card.profile.as_deref();
    let user = app.store.user(card.user_id);
    let name = app
        .store
        .member_name(card.guild_id.unwrap_or(0), card.user_id)
        .map(str::to_owned)
        .or_else(|| user.map(|user| user.name.clone()))
        .unwrap_or_else(|| "Unknown User".to_owned());
    let username = profile.map(|profile| {
        let user = &profile.user.user;
        match user
            .discriminator
            .as_deref()
            .filter(|discriminator| !matches!(*discriminator, "" | "0"))
        {
            Some(discriminator) => format!("{}#{discriminator}", user.username),
            None => user.username.clone(),
        }
    });
    div()
        .flex()
        .flex_col()
        .gap(rpx(10.))
        .pt(rpx(18.))
        .px(rpx(16.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(4.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(24.))
                        .line_height(rpx(32.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.text))
                        .child(label(("app-profile-name", card.user_id), &name)),
                )
                .children(
                    user.filter(|user| user.bot)
                        .map(|user| div().mt(rpx(1.)).child(theme::app_tag(user.verified_bot))),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(5.))
                .min_h(rpx(20.))
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(palette.strong))
                .children(username.map(|username| div().child(username)))
                .children(profile.and_then(|profile| badges(app, card, profile, palette, cx))),
        )
}

fn tabs(app: &DiscordApp, card: &ProfileCard, cx: &App) -> impl IntoElement {
    let palette = card.palette();
    let profile = card.profile.as_deref();
    let mutual_guilds: Vec<&model::MutualGuild> = profile
        .and_then(|profile| profile.mutual_guilds.as_ref())
        .into_iter()
        .flatten()
        .collect();
    let application = profile.and_then(|profile| profile.application.as_ref());
    let current = match card.tab {
        Tab::Servers if !mutual_guilds.is_empty() => Tab::Servers,
        Tab::DataAccess if application.is_some() => Tab::DataAccess,
        _ => Tab::Bio,
    };
    let tab = |id: Tab, title: String| {
        let entity = app.entity.clone();
        div()
            .id(SharedString::from(format!("app-profile-tab-{}", id as u8)))
            .pb(rpx(5.))
            .border_b_2()
            .border_color(if current == id {
                rgb(palette.strong)
            } else {
                rgba(0)
            })
            .cursor_pointer()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .text_color(rgb(palette.text))
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.select_profile_tab(id, cx))
                    .ok();
            })
            .child(title)
    };
    let panel = if card.loading() {
        super::super::popout::skeleton(card.user_id).into_any_element()
    } else {
        match current {
            Tab::Servers => servers_panel(app, &mutual_guilds, palette, cx),
            Tab::DataAccess => data_access(application, palette),
            _ => bio_panel(app, card, cx),
        }
    };
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .mt(rpx(28.))
        .child(
            div()
                .flex()
                .gap(rpx(56.))
                .px(rpx(16.))
                .border_b_1()
                .border_color(palette.border)
                .child(tab(Tab::Bio, "Bio".to_owned()))
                .when(!mutual_guilds.is_empty(), |this| {
                    this.child(tab(
                        Tab::Servers,
                        plural(mutual_guilds.len(), "Mutual Server"),
                    ))
                })
                .when(application.is_some(), |this| {
                    this.child(tab(Tab::DataAccess, "Data Access".to_owned()))
                }),
        )
        .child(
            div()
                .id("app-profile-panel")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .p(rpx(16.))
                .overflow_y_scroll()
                .child(panel),
        )
}

fn bio_panel(app: &DiscordApp, card: &ProfileCard, cx: &App) -> AnyElement {
    let palette = card.palette();
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(rpx(20.))
        .children(bio(app, card, false, cx))
        .children(command_chips(app, card).map(|chips| app_section("Commands", chips, palette)))
        .children(role_chips(app, card).map(|chips| app_section("Roles", chips, palette)))
        .child(app_section(
            "Created On",
            member_since_dates(app, card, cx)
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(palette.text))
                .into_any_element(),
            palette,
        ))
        .children(card.note.as_ref().map(|note| {
            app_section(
                "Note (only visible to you)",
                div()
                    .text_size(rpx(14.))
                    .line_height(rpx(18.))
                    .text_color(rgb(palette.text))
                    .child(note.clone())
                    .into_any_element(),
                palette,
            )
        }))
        .into_any_element()
}

fn data_access(
    application: Option<&model::ProfileApplication>,
    palette: ProfilePalette,
) -> AnyElement {
    let rows = application.map(model::ProfileApplication::data_access);
    div()
        .flex()
        .flex_col()
        .flex_none()
        .gap(rpx(32.))
        .children(rows.into_iter().flatten().map(|access| {
            let (icon, title, text) = match access {
                model::DataAccess::Messages => (
                    "message-circle",
                    "Read Messages",
                    "Can access messages in this server and read their contents.",
                ),
                model::DataAccess::Presence => (
                    "gamepad-2",
                    "Presence",
                    "Can see online status of server members and what they are playing, watching, or listening to.",
                ),
                model::DataAccess::Members => (
                    "users",
                    "Members",
                    "Can see who's in the server and their roles, nicknames, and permission changes.",
                ),
            };
            div()
                .flex()
                .items_center()
                .gap(rpx(20.))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(rpx(40.))
                        .rounded_full()
                        .bg(rgba(0x97979F14))
                        .child(theme::icon(icon, 20., palette.text)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rpx(4.))
                        .min_w_0()
                        .child(
                            div()
                                .text_size(rpx(12.))
                                .line_height(rpx(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.strong))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .text_color(rgb(palette.text))
                                .child(text),
                        ),
                )
        }))
        .child(
            div()
                .flex()
                .flex_wrap()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(palette.text))
                .child("All apps can access a set of baseline data. Please visit the\u{a0}")
                .child(
                    div()
                        .id("app-profile-help-center")
                        .cursor_pointer()
                        .text_color(rgb(palette.link.unwrap_or_else(theme::link)))
                        .hover(|style| style.underline())
                        .on_click(|_, _, cx| cx.open_url(HELP_CENTER))
                        .child("Help Center"),
                )
                .child("\u{a0}to learn more."),
        )
        .into_any_element()
}
