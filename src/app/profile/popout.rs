mod body;
mod names;
mod sections;
mod status_bubble;

pub(super) use body::{board_sections, skeleton};
use body::{popout_body, pulse};
pub(super) use names::display_names;
pub(super) use sections::{
    add_app_button, app_section, bio, command_chips, is_app, markdown, member_since,
    member_since_dates, playing, role_chips, roles, section,
};
use status_bubble::status_bubble;

use gpui_component::scroll::ScrollableElement as _;
use std::time::SystemTime;

use crate::app::{
    messages::message_body::emoji_pieces,
    messages::rich_text::{Content, EmojiSource, Piece, PieceStyle, rich_text},
};
use crate::store::{CustomStatus, StatusEmoji};

use super::badges::badges;
use super::frame::{frame_overflow, framed, framed_with};
use super::loading::guild_profile;
use super::mutuals::mutuals_line;
use super::*;

mod menu;
pub(super) mod status;

pub(super) use menu::{message_button, more_button, profile_actions};

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
    let surface = theme.map_or(theme::elevated(), ProfileTheme::surface);
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
            true,
            cx,
        ))
        .child(popout_body(app, card, &layout, window, cx))
        .children(effect_layers(card, layout.width, corner, false, cx))
        .when(app.profile_gated(card.user_id), |this| {
            this.min_h(rpx(gate::MIN_HEIGHT))
                .child(gate::gate(app, card.user_id, corner, cx))
        });
    let inner = match card.account_max_height(window, &overflow) {
        Some(height) => {
            let inset = match theme {
                Some(_) => rpx(THEME_BORDER * 2.).to_pixels(window.rem_size()),
                None => px(2.),
            };
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
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgba(theme::border())),
        })
        .child(inner)
        .into_any_element();
    let framed = framed(card, outer_width, outer, false, cx);
    let close = app.entity.clone();
    let in_status_menu = card.status_menu.hit_test();
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

pub(super) fn style_preview(
    app: &DiscordApp,
    card: &ProfileCard,
    effect: Option<&model::CollectibleItem>,
    frame: Option<(u64, &model::CollectibleItem)>,
    clock: &RefCell<Option<Instant>>,
    compact: bool,
    cx: &App,
) -> AnyElement {
    let theme = card.theme();
    let palette = card.palette();
    let layout = Layout::popout(theme.is_some());
    let corner = if theme.is_some() { 6. } else { 8. };
    let surface = theme.map_or(theme::elevated(), ProfileTheme::surface);
    let outer_width = POPOUT_WIDTH + theme.map_or(0., |_| THEME_BORDER * 2.);
    let button = div()
        .mx(rpx(16.))
        .mb(rpx(12.))
        .h(rpx(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rpx(8.))
        .bg(rgb(theme.map_or(theme::BLURPLE, ProfileTheme::button)))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(gpui::white())
        .child("Example Button");
    let inner = div()
        .relative()
        .flex()
        .flex_col()
        .rounded(rpx(corner))
        .overflow_hidden()
        .text_color(rgb(palette.text))
        .map(|this| match theme {
            Some(theme) => this.bg(theme.overlay()),
            None => this,
        })
        .child(header(app, card, &layout, surface, None, !compact, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(rpx(12.))
                .px(rpx(layout.padding))
                .pt(rpx(4.))
                .pb(rpx(8.))
                .child(display_names(app, card, false, None, cx))
                .when(!compact, |this| this.children(bio(app, card, true, cx))),
        )
        .when(!compact, |this| this.child(button))
        .children(
            effect
                .into_iter()
                .flat_map(|item| effect_item_layers(item, clock, layout.width, corner, false, cx)),
        );
    let outer = div()
        .relative()
        .w(rpx(outer_width))
        .rounded(rpx(8.))
        .map(|this| match theme {
            Some(theme) => this.p(rpx(THEME_BORDER)).bg(theme.outer()),
            None => this
                .bg(rgb(theme::elevated()))
                .border_1()
                .border_color(rgba(theme::border())),
        })
        .child(inner)
        .into_any_element();
    framed_with(frame, outer_width, outer, false, cx)
}

pub(super) type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub(super) fn header(
    app: &DiscordApp,
    card: &ProfileCard,
    layout: &Layout,
    surface: u32,
    on_avatar: Option<ClickHandler>,
    show_status: bool,
    cx: &App,
) -> AnyElement {
    let user = app.store.user(card.user_id);
    let palette = card.palette();
    let status = active_custom_status(
        user.and_then(|user| user.custom_status.as_ref()),
        SystemTime::now(),
    )
    .cloned();
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
        .child(match editing_styles(app, card) {
            Some(editor) => {
                let shown = super::styles_panel::shown(app, editor);
                let decoration = shown
                    .decoration
                    .and_then(|sku| super::styles_panel::decoration_of(app, editor, sku));
                theme::preview_avatar(
                    user,
                    card.user_id,
                    layout.avatar,
                    user.map(|user| (user.status, surface)),
                    shown.avatar,
                    shown.avatar_file,
                    decoration.as_ref(),
                    cx,
                )
                .into_any_element()
            }
            None => theme::member_avatar(
                &app.store,
                card.guild_id.unwrap_or(0),
                card.user_id,
                layout.avatar,
                user.map(|user| (user.status, surface)),
                true,
                cx,
            )
            .into_any_element(),
        })
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
        .child(banner(app, card, layout, cx))
        .child(avatar)
        .children(
            status
                .filter(|_| show_status)
                .map(|status| status_bubble(card.user_id, &status, prompt, layout, palette, cx)),
        )
        .into_any_element()
}

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

fn editing_styles<'a>(
    app: &'a DiscordApp,
    card: &ProfileCard,
) -> Option<&'a super::styles::StylesEditor> {
    app.profile_styles
        .as_ref()
        .filter(|_| card.modal && card.user_id == app.store.current_user_id)
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

pub(super) fn banner(
    app: &DiscordApp,
    card: &ProfileCard,
    layout: &Layout,
    cx: &App,
) -> AnyElement {
    let profile = card.profile.as_deref();
    let pixels = BANNER_SOURCE;
    let corner = rpx(layout.corner);
    let (spec, file) = match editing_styles(app, card) {
        Some(editor) => {
            let shown = super::styles_panel::shown(app, editor);
            (shown.banner, shown.banner_file)
        }
        None => (banner_spec(card), None),
    };
    let file = file.map(|path| {
        img(path)
            .size_full()
            .rounded_t(corner)
            .object_fit(ObjectFit::Cover)
            .into_any_element()
    });
    let image = file.or_else(|| {
        spec.and_then(|spec| {
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
        })
    });
    let color = profile
        .and_then(|p| p.user.banner_color.or(p.user.accent_color))
        .or_else(|| profile?.user_profile.as_ref()?.accent_color)
        .or_else(|| card.theme().map(|theme| theme.primary))
        .unwrap_or(theme::surface_highest());
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
