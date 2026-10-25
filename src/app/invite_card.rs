use gpui::{prelude::*, *};

use super::DiscordApp;
use crate::{
    discord::model::invite_codes,
    images,
    store::{Invite, InviteApp, InviteState, Message},
    theme::{self, rpx},
};

const WIDTH: f32 = 300.;
const BANNER: f32 = 72.;
const ICON: f32 = 62.;
const ICON_RING: f32 = 4.;
const GAME_ICON: f32 = 24.;
/// The icon overhangs the banner by half its height, plus its ring.
const ICON_OVERHANG: f32 = ICON / 2. + ICON_RING;
const BUTTON_GREEN: u32 = 0x00863A;
const BUTTON_GREEN_HOVER: u32 = 0x047B37;

/// Server invite cards for the invite links in a message, in link order.
pub(super) fn cards(app: &DiscordApp, message: &Message, cx: &App) -> Vec<AnyElement> {
    invite_codes(&message.content)
        .into_iter()
        .enumerate()
        .map(|(index, code)| {
            let key = message.id.wrapping_mul(4).wrapping_add(index as u64);
            match app.invite(&code) {
                InviteState::Loading => frame(key).child(loading()).into_any_element(),
                InviteState::Invalid => frame(key).child(invalid()).into_any_element(),
                InviteState::Ready(invite) => card(app, key, &invite, cx),
            }
        })
        .collect()
}

// Block layout throughout: the chat list measures rows at min-content height, where gpui's
// flex columns shrink every child to nothing.
fn frame(key: u64) -> Stateful<Div> {
    div()
        .id(("invite-card", key))
        .mt_1()
        // A definite width: inside the message's flex column a percentage width is unresolved
        // while the card's height is measured, so wrapped text would count as one line.
        .w(rpx(WIDTH))
        .max_w_full()
        .rounded(rpx(12.))
        .border_1()
        .border_color(rgb(theme::BORDER))
        .bg(rgb(theme::ELEVATED))
}

fn loading() -> Div {
    div()
        .p(rpx(16.))
        .text_size(rpx(14.))
        .text_color(rgb(theme::MUTED))
        .child("Resolving invite…")
}

fn invalid() -> Div {
    div()
        .p(rpx(16.))
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::RED))
                .child("Invalid Invite"),
        )
        .child(
            div()
                .mt(rpx(4.))
                .text_size(rpx(14.))
                .text_color(rgb(theme::MUTED))
                .child("Try sending a new invite!"),
        )
}

fn card(app: &DiscordApp, key: u64, invite: &Invite, cx: &App) -> AnyElement {
    let joined = app.store.guild_by_id(invite.guild_id).is_some();
    let icon_spec = invite.icon_spec();
    // Without a banner, Discord shows the server icon blurred across the top.
    // Cover-fitted images sample their atlas tile's edge texels, which bleeds a neighbouring
    // tile's colour into a 1px line; the card border is painted over that edge.
    let fill = |image: Img| {
        image
            .absolute()
            .inset_0()
            .size_full()
            .rounded_t(rpx(12.))
            .object_fit(ObjectFit::Cover)
    };
    let banner = match invite.banner_spec() {
        Some(spec) => images::resolve(&spec, theme::image_px(WIDTH), cx)
            .map(|path| fill(img(path)).into_any_element()),
        None => icon_spec
            .as_ref()
            .and_then(|spec| images::blurred(spec, cx))
            .map(|image| fill(img(image)).opacity(0.6).into_any_element()),
    };
    let icon = icon_spec
        .as_ref()
        .and_then(|spec| images::resolve(spec, theme::image_px(ICON), cx))
        .map(|path| img(path).size_full().rounded(rpx(16.)).into_any_element())
        .unwrap_or_else(|| {
            div()
                .size_full()
                .rounded(rpx(16.))
                .bg(rgb(theme::SELECTED))
                .flex()
                .items_center()
                .justify_center()
                .text_size(rpx(20.))
                .text_color(rgb(theme::STRONG))
                .child(initials(&invite.name))
                .into_any_element()
        });
    let games: Vec<_> = invite
        .games
        .iter()
        .filter_map(|id| invite.game_apps.iter().find(|game| game.id == *id))
        .collect();
    let game_icon = |game: &InviteApp| {
        let path = images::resolve(&game.icon_spec()?, theme::image_px(GAME_ICON), cx)?;
        Some(img(path).size(rpx(GAME_ICON)).flex_none().rounded(rpx(6.)))
    };
    // A lone game is listed by name; several collapse to a row of icons.
    let games = match games.as_slice() {
        [] => None,
        [game] => Some(
            div()
                .flex()
                .items_center()
                .gap(rpx(8.))
                .min_w_0()
                .children(game_icon(game))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::STRONG))
                        .child(game.name.clone()),
                ),
        ),
        games => Some(
            div()
                .flex()
                .flex_wrap()
                .gap(rpx(4.))
                .children(games.iter().filter_map(|game| game_icon(game))),
        ),
    };

    let entity = app.entity.clone();
    let guild_id = invite.guild_id;
    let code = invite.code.clone();
    let button = div()
        .id(("invite-join", key))
        .mt(rpx(16.))
        .h(rpx(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rpx(8.))
        .bg(rgb(BUTTON_GREEN))
        .hover(|this| this.bg(rgb(BUTTON_GREEN_HOVER)))
        .cursor_pointer()
        .text_size(rpx(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(0xFFFFFF))
        .child(if joined {
            "Go to Server"
        } else {
            "Join Server"
        })
        .on_click(move |_, window, cx| {
            if joined {
                entity
                    .update(cx, |app, cx| app.select_guild(guild_id, window, cx))
                    .ok();
            } else {
                // Joining needs Discord's captcha flow, so hand it to the browser.
                cx.open_url(&format!("https://discord.gg/{code}"));
            }
        });

    frame(key)
        .relative()
        .border_0()
        .child(
            div()
                .relative()
                .h(rpx(BANNER + ICON_OVERHANG))
                .child(
                    div()
                        .relative()
                        .h(rpx(BANNER))
                        .rounded_t(rpx(12.))
                        .bg(rgb(theme::SELECTED))
                        .children(banner),
                )
                .child(
                    div()
                        .absolute()
                        .left(rpx(16.))
                        .top(rpx(BANNER - ICON_OVERHANG))
                        .size(rpx(ICON + ICON_RING * 2.))
                        .p(rpx(ICON_RING))
                        .rounded(rpx(20.))
                        .bg(rgb(theme::ELEVATED))
                        .child(icon),
                ),
        )
        .child(
            div()
                .px(rpx(16.))
                .pb(rpx(16.))
                .child(
                    div()
                        .mt(rpx(8.))
                        .flex()
                        .items_center()
                        .gap(rpx(6.))
                        .min_w_0()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(rpx(16.))
                                .line_height(rpx(20.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(theme::STRONG))
                                .child(invite.name.clone()),
                        )
                        .children(invite.tag.as_ref().map(|tag| theme::guild_tag(tag, cx))),
                )
                .child(
                    div()
                        .mt(rpx(4.))
                        .flex()
                        .items_center()
                        .gap(rpx(8.))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::MUTED))
                        .child(count(
                            theme::GREEN,
                            format!("{} Online", grouped(invite.online)),
                        ))
                        .child(count(
                            theme::MUTED,
                            format!("{} Members", grouped(invite.members)),
                        )),
                )
                .child(
                    div()
                        .mt(rpx(2.))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::MUTED))
                        .child(format!("Est. {}", invite.created_at.month_year())),
                )
                .children(invite.description.as_ref().map(|description| {
                    div()
                        .mt(rpx(12.))
                        .text_size(rpx(14.))
                        .line_height(rpx(18.))
                        .text_color(rgb(theme::TEXT))
                        .child(description.clone())
                }))
                .children(games.map(|games| games.mt(rpx(12.))))
                .child(button),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .rounded(rpx(12.))
                .border_1()
                .border_color(rgb(theme::BORDER)),
        )
        .into_any_element()
}

fn count(color: u32, label: String) -> Div {
    div()
        .flex()
        .items_center()
        .gap(rpx(4.))
        .child(div().size(rpx(8.)).rounded_full().bg(rgb(color)))
        .child(label)
}

/// 1234 -> "1,234"
fn grouped(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(3)
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_group_thousands() {
        assert_eq!(super::grouped(87), "87");
        assert_eq!(super::grouped(1234), "1,234");
        assert_eq!(super::grouped(1234567), "1,234,567");
    }
}
