use super::popout::section;
use super::*;

pub(super) fn connections(profile: &model::Profile, palette: ProfilePalette) -> Option<AnyElement> {
    if profile.connected_accounts.is_empty() {
        return None;
    }
    Some(section(
        "Connections",
        div()
            .flex()
            .flex_col()
            .gap(rpx(16.))
            .children(
                profile
                    .connected_accounts
                    .iter()
                    .enumerate()
                    .map(|(index, account)| {
                        let url = connection_url(account);
                        div()
                            .id(("profile-connection", index))
                            .flex()
                            .items_center()
                            .gap(rpx(12.))
                            .min_w_0()
                            .child(
                                div()
                                    .size(rpx(28.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(rpx(8.))
                                    .bg(rgb(theme::connection_tile_bg()))
                                    .map(|this| match connection_icon(&account.kind) {
                                        Some((icon, color)) => {
                                            this.child(theme::icon(icon, 18., color))
                                        }
                                        None => this
                                            .text_size(rpx(13.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(rgb(theme::connection_tile_text()))
                                            .child(connection_initial(&account.kind)),
                                    }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(rpx(16.))
                                    .text_color(rgb(palette.strong))
                                    .child(account.name.clone()),
                            )
                            .when_some(url, |this, url| {
                                this.cursor_pointer()
                                    .child(theme::icon("arrow-up-right", 16., palette.strong))
                                    .on_click(move |_, _, cx| cx.open_url(&url))
                            })
                    }),
            )
            .into_any_element(),
        palette,
    ))
}

/// Icon and brand color, drawn on the white connection tile.
fn connection_icon(kind: &str) -> Option<(&'static str, u32)> {
    Some(match kind {
        "github" => ("connections/github", 0x181717),
        "twitch" => ("connections/twitch", 0x9146FF),
        "youtube" => ("connections/youtube", 0xFF0000),
        "steam" => ("connections/steam", 0x000000),
        "spotify" => ("connections/spotify", 0x1ED760),
        "xbox" => ("connections/xbox", 0x107C10),
        "playstation" => ("connections/playstation", 0x0070D1),
        "reddit" => ("connections/reddit", 0xFF4500),
        "twitter" => ("connections/twitter", 0x000000),
        "domain" => ("connections/domain", 0x000000),
        "battlenet" => ("connections/battlenet", 0x148EFF),
        "leagueoflegends" => ("connections/leagueoflegends", 0xC28F2C),
        "tiktok" => ("connections/tiktok", 0x000000),
        "bluesky" => ("connections/bluesky", 0x0285FF),
        "instagram" => ("connections/instagram", 0xFF0069),
        "facebook" => ("connections/facebook", 0x0866FF),
        "epicgames" => ("connections/epicgames", 0x313131),
        "riotgames" => ("connections/riotgames", 0xEB0029),
        "roblox" => ("connections/roblox", 0x000000),
        "paypal" => ("connections/paypal", 0x002991),
        "ebay" => ("connections/ebay", 0xE53238),
        "crunchyroll" => ("connections/crunchyroll", 0xFF5E00),
        _ => return None,
    })
}

/// Public profile link for platforms Discord links out to; the rest render without the arrow.
fn connection_url(account: &model::ConnectedAccount) -> Option<String> {
    let (name, id) = (&account.name, &account.id);
    Some(match account.kind.as_str() {
        "github" => format!("https://github.com/{name}"),
        "twitch" => format!("https://www.twitch.tv/{name}"),
        "youtube" => format!("https://www.youtube.com/channel/{id}"),
        "steam" => format!("https://steamcommunity.com/profiles/{id}"),
        "spotify" => format!("https://open.spotify.com/user/{id}"),
        "reddit" => format!("https://www.reddit.com/u/{name}"),
        "twitter" => format!("https://x.com/{name}"),
        "domain" => format!("https://{name}"),
        "tiktok" => format!("https://www.tiktok.com/@{name}"),
        "bluesky" => format!("https://bsky.app/profile/{id}"),
        "instagram" => format!("https://www.instagram.com/{name}"),
        "roblox" => format!("https://www.roblox.com/users/{id}/profile"),
        "ebay" => format!("https://www.ebay.com/usr/{name}"),
        _ => return None,
    })
}

fn connection_initial(kind: &str) -> String {
    connection_name(kind)
        .chars()
        .next()
        .unwrap_or('?')
        .to_string()
}

fn connection_name(kind: &str) -> String {
    match kind {
        "github" => "GitHub".into(),
        "twitch" => "Twitch".into(),
        "youtube" => "YouTube".into(),
        "steam" => "Steam".into(),
        "spotify" => "Spotify".into(),
        "xbox" => "Xbox".into(),
        "playstation" => "PlayStation".into(),
        "reddit" => "Reddit".into(),
        "twitter" => "X".into(),
        "domain" => "Domain".into(),
        "battlenet" => "Battle.net".into(),
        "leagueoflegends" => "League".into(),
        "tiktok" => "TikTok".into(),
        "bluesky" => "Bluesky".into(),
        "instagram" => "Instagram".into(),
        "facebook" => "Facebook".into(),
        "epicgames" => "Epic Games".into(),
        "riotgames" => "Riot".into(),
        "roblox" => "Roblox".into(),
        "paypal" => "PayPal".into(),
        "ebay" => "eBay".into(),
        "crunchyroll" => "Crunchyroll".into(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}
