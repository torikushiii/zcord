use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

pub struct AppAssets;

pub const LOGO_PATH: &str = "app/zcord.png";
const LOGO: &[u8] = include_bytes!("../assets/zcord.png");

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("app/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*]
    };
}

const ICONS: &[(&str, &[u8])] = icons![
    "activity-game",
    "activity-streak",
    "activity-trophy",
    "paperclip",
    "chart-column",
    "monitor-share",
    "app-window",
    "arrow-up-down",
    "at-sign",
    "accessibility",
    "activity",
    "app-window",
    "arrow-left",
    "arrow-right",
    "arrow-up-right",
    "audio-lines",
    "award",
    "badge-check",
    "bell",
    "calendar",
    "check",
    "chevron-down",
    "chevron-right",
    "chevron-up",
    "circle-plus",
    "circle-plus-filled",
    "circle-question-mark",
    "clock",
    "compass",
    "copy",
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
    "connections/twitch-fill",
    "connections/twitter",
    "connections/xbox",
    "connections/youtube",
    "ellipsis",
    "download",
    "external-link",
    "eye",
    "eye-off",
    "file",
    "flame",
    "folder",
    "forward",
    "gamepad-2",
    "gem",
    "hash",
    "hash-lock",
    "headphone-off",
    "headphones",
    "image",
    "id-badge",
    "image-play",
    "inbox",
    "languages",
    "library",
    "link",
    "lock",
    "log-out",
    "mail",
    "mail-filled",
    "megaphone",
    "message-circle",
    "message-square",
    "maximize-2",
    "messages-square",
    "mic",
    "mic-off",
    "minus",
    "monitor-filled",
    "monitor-up",
    "monitor-x",
    "music",
    "palette",
    "pencil",
    "phone",
    "pin",
    "play",
    "plus",
    "quote",
    "reply",
    "rotate-cw",
    "rss",
    "search",
    "settings",
    "shapes",
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
    "tooltip-arrow-down-edge",
    "tooltip-arrow-edge",
    "tooltip-arrow-up",
    "tooltip-arrow-up-edge",
    "trash-2",
    "upload-fold",
    "upload-glyph-code",
    "upload-glyph-image",
    "upload-glyph-text",
    "upload-sheet",
    "trending-up",
    "user",
    "user-check",
    "user-plus",
    "users",
    "video",
    "video-exit-fullscreen",
    "video-fullscreen",
    "video-muted",
    "video-off",
    "video-pause",
    "video-play",
    "video-volume",
    "volume-1-filled",
    "volume-2",
    "volume-2-lock",
    "thread",
    "x",
    "zap",
    "zoom-in",
    "zoom-out",
];

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == LOGO_PATH {
            return Ok(Some(Cow::Borrowed(LOGO)));
        }
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
