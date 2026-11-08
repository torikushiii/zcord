use super::*;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct GameActions {
    pub(super) steam_url: Option<String>,
    pub(super) play_url: Option<String>,
}

impl GameActions {
    pub(super) fn resolve(
        application_id: u64,
        games: &HashMap<u64, Arc<Game>>,
        applications: &HashMap<u64, Arc<PublicApplication>>,
    ) -> Self {
        let game = games.get(&application_id);
        let mut linked_games: Vec<_> = games
            .values()
            .filter(|game| {
                game.id != application_id
                    && game
                        .linked_applications
                        .iter()
                        .any(|linked| linked.id == application_id)
            })
            .collect();
        linked_games.sort_by_key(|game| game.id);
        let mut linked: Vec<_> = applications
            .values()
            .filter(|application| {
                application.id != application_id
                    && (application
                        .linked_games
                        .iter()
                        .any(|linked| linked.id == application_id)
                        || game.is_some_and(|game| {
                            game.linked_applications
                                .iter()
                                .any(|linked| linked.id == application.id)
                        }))
            })
            .collect();
        linked.sort_by_key(|application| application.id);
        let action_applications: Vec<_> = applications
            .get(&application_id)
            .into_iter()
            .chain(linked)
            .collect();
        let mut action_games: Vec<&Game> = game
            .into_iter()
            .chain(linked_games)
            .map(Arc::as_ref)
            .collect();
        for application in &action_applications {
            for linked in application
                .linked_games
                .iter()
                .filter(|linked| application.id == application_id || linked.id == application_id)
            {
                if let Some(game) = games.get(&linked.id) {
                    action_games.push(game.as_ref());
                }
                if let Some(game) = linked.application.as_ref() {
                    action_games.push(game);
                }
            }
        }
        Self {
            steam_url: action_games
                .iter()
                .find_map(|game| game.steam_app_id.as_deref().and_then(steam_store_url))
                .or_else(|| {
                    action_applications.iter().find_map(|application| {
                        application
                            .steam_app_id
                            .as_deref()
                            .and_then(steam_store_url)
                    })
                }),
            play_url: action_games
                .iter()
                .find_map(|game| official_game_website(game))
                .or_else(|| {
                    action_applications.iter().find_map(|application| {
                        application
                            .deeplink_uri
                            .as_deref()
                            .and_then(game_launch_url)
                            .filter(|url| !matches!(url.scheme(), "http" | "https"))
                            .map(|url| url.to_string())
                    })
                }),
        }
    }

    pub(super) fn buttons(
        self,
        context: &'static str,
        key: u64,
        palette: ProfilePalette,
    ) -> Option<Stateful<Div>> {
        if self.steam_url.is_none() && self.play_url.is_none() {
            return None;
        }
        Some(
            div()
                .id((context, key))
                .flex()
                .flex_col()
                .gap(rpx(8.))
                .children(
                    self.play_url
                        .map(|url| game_action_button("play-now", "Play Now", None, url, palette)),
                )
                .children(self.steam_url.map(|url| {
                    game_action_button(
                        "steam",
                        "Get on Steam",
                        Some("connections/steam"),
                        url,
                        palette,
                    )
                })),
        )
    }
}

fn steam_store_url(id: &str) -> Option<String> {
    let id = id.parse::<u64>().ok().filter(|id| *id > 0)?;
    Some(format!("https://store.steampowered.com/app/{id}"))
}

fn official_game_website(game: &Game) -> Option<String> {
    game.websites
        .iter()
        .filter(|website| website.category == 1)
        .find_map(|website| {
            let url = game_launch_url(&website.url)?;
            matches!(url.scheme(), "http" | "https").then(|| url.to_string())
        })
}

pub(super) fn game_launch_url(raw: &str) -> Option<reqwest::Url> {
    let url = reqwest::Url::parse(raw.trim()).ok()?;
    // Do not hand arbitrary executable/document protocols to the operating system.
    let supported = match url.scheme() {
        "http" | "https" => url.host_str().is_some(),
        "steam" => matches!(url.host_str(), Some("run" | "rungameid")),
        "com.epicgames.launcher" => {
            url.host_str() == Some("apps")
                && url
                    .query_pairs()
                    .any(|(key, value)| key == "action" && value == "launch")
        }
        _ => false,
    };
    if !supported || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    Some(url)
}

fn game_action_button(
    id: &'static str,
    text: &'static str,
    icon: Option<&'static str>,
    url: String,
    palette: ProfilePalette,
) -> Stateful<Div> {
    let hover = rgba(if palette.surface_light {
        0x97979F29
    } else {
        0x97979F33
    });
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(rpx(8.))
        .h(rpx(32.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .bg(palette.button)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(palette.strong))
        .children(icon.map(|icon| theme::icon(icon, 16., palette.strong)))
        .child(div().min_w_0().truncate().child(text))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            cx.open_url(&url);
        })
}

pub(super) fn play_button(game: &GameActivity, palette: ProfilePalette) -> Option<Stateful<Div>> {
    let application_id = game.application_id.filter(|_| game.playable)?;
    Some(
        div()
            .id("profile-current-play")
            .flex()
            .items_center()
            .justify_center()
            .gap(rpx(8.))
            .h(rpx(36.))
            .px(rpx(16.))
            .rounded(rpx(8.))
            .bg(palette.button)
            .cursor_pointer()
            .hover(move |style| style.bg(palette.raised))
            .text_size(rpx(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.strong))
            .child(theme::icon("activity", 18., palette.strong))
            .child("Play")
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                cx.open_url(&format!(
                    "https://discord.com/application-directory/{application_id}"
                ))
            }),
    )
}
