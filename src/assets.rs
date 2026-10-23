use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

pub struct AppAssets;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("app/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*]
    };
}

const ICONS: &[(&str, &[u8])] = icons![
    "arrow-left",
    "arrow-right",
    "badge-check",
    "bell",
    "calendar",
    "chevron-down",
    "circle-plus",
    "circle-plus-filled",
    "circle-question-mark",
    "clock",
    "compass",
    "discord",
    "external-link",
    "file",
    "folder",
    "forward",
    "gamepad-2",
    "gem",
    "gift",
    "hash",
    "headphones",
    "image",
    "image-play",
    "inbox",
    "languages",
    "layout-grid",
    "messages-square",
    "mic",
    "pencil",
    "pin",
    "play",
    "plus",
    "reply",
    "search",
    "settings",
    "smile",
    "smile-plus",
    "sticker",
    "tooltip-arrow",
    "tooltip-arrow-down",
    "trash-2",
    "user-plus",
    "users",
    "video-muted",
    "video-pause",
    "video-play",
    "video-volume",
    "volume-2",
    "x",
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
