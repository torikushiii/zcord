use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

pub struct AppAssets;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("app/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*]
    };
}

const ICONS: &[(&str, &[u8])] = icons![
    "activity",
    "arrow-left",
    "arrow-right",
    "arrow-up-right",
    "award",
    "badge-check",
    "bell",
    "calendar",
    "check",
    "chevron-down",
    "chevron-up",
    "circle-plus",
    "circle-plus-filled",
    "circle-question-mark",
    "clock",
    "compass",
    "connections/battlenet",
    "connections/bluesky",
    "connections/crunchyroll",
    "connections/domain",
    "connections/ebay",
    "connections/epicgames",
    "connections/facebook",
    "connections/github",
    "connections/instagram",
    "connections/leagueoflegends",
    "connections/paypal",
    "connections/playstation",
    "connections/reddit",
    "connections/riotgames",
    "connections/roblox",
    "connections/spotify",
    "connections/steam",
    "connections/tiktok",
    "connections/twitch",
    "connections/twitter",
    "connections/xbox",
    "connections/youtube",
    "discord",
    "external-link",
    "file",
    "flame",
    "folder",
    "forward",
    "gamepad-2",
    "gem",
    "hash",
    "headphones",
    "image",
    "image-play",
    "inbox",
    "library",
    "lock",
    "mail",
    "megaphone",
    "message-circle",
    "message-square",
    "messages-square",
    "mic",
    "music",
    "pencil",
    "pin",
    "play",
    "plus",
    "quote",
    "reply",
    "rotate-cw",
    "search",
    "settings",
    "smile",
    "smile-plus",
    "sprout",
    "star",
    "star-filled",
    "sticker",
    "store",
    "thumbs-down",
    "thumbs-up",
    "timer",
    "tooltip-arrow",
    "tooltip-arrow-down",
    "trash-2",
    "trending-up",
    "user-plus",
    "users",
    "video-exit-fullscreen",
    "video-fullscreen",
    "video-muted",
    "video-pause",
    "video-play",
    "video-volume",
    "volume-2",
    "x",
    "zap",
];

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match ICONS.iter().find(|(name, _)| *name == path) {
            Some((_, bytes)) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_component_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_component_assets::Assets.list(path)?;
        paths.extend(
            ICONS
                .iter()
                .filter(|(name, _)| name.starts_with(path))
                .map(|(name, _)| SharedString::from(*name)),
        );
        Ok(paths)
    }
}
