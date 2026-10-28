use std::{borrow::Cow, fs, path::PathBuf};

use gpui::{App, SharedString};
use gpui_component::Theme;

mod display;

pub(crate) use display::{DisplayFont, display_font};

const DISCORD_FAMILY: &str = "gg sans";
const BUNDLED_FAMILY: &str = "Noto Sans";

const BUNDLED: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/NotoSans-Regular.ttf"),
    include_bytes!("../assets/fonts/NotoSans-Medium.ttf"),
    include_bytes!("../assets/fonts/NotoSans-SemiBold.ttf"),
    include_bytes!("../assets/fonts/NotoSans-Bold.ttf"),
];

fn user_font_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("zcord").join("fonts"))
}

pub fn init(cx: &mut App) {
    let text_system = cx.text_system().clone();
    if let Err(error) = text_system.add_fonts(BUNDLED.map(Cow::Borrowed).to_vec()) {
        log::error!("Could not load bundled fonts: {error:#}");
    }

    let user_fonts = read_user_fonts();
    if !user_fonts.is_empty() {
        let count = user_fonts.len();
        match text_system.add_fonts(user_fonts) {
            Ok(()) => log::info!("Loaded {count} user font files"),
            Err(error) => log::warn!("Could not load user fonts: {error:#}"),
        }
    }

    if let Err(error) = text_system.add_fonts(display::FONTS.map(Cow::Borrowed).to_vec()) {
        log::error!("Could not load bundled display fonts: {error:#}");
    }

    let available = text_system.all_font_names();
    display::cache_availability(cx, &available);
    let family = pick_family(&available);
    log::info!("Using {family} as UI font");
    Theme::global_mut(cx).font_family = family;
}

fn read_user_fonts() -> Vec<Cow<'static, [u8]>> {
    let Some(entries) = user_font_dir().and_then(|dir| fs::read_dir(dir).ok()) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    ["ttf", "otf", "ttc"].contains(&extension.to_ascii_lowercase().as_str())
                })
        })
        .filter_map(|path| match fs::read(&path) {
            Ok(bytes) => Some(Cow::Owned(bytes)),
            Err(error) => {
                log::warn!("Could not read font {}: {error}", path.display());
                None
            }
        })
        .collect()
}

fn pick_family(available: &[String]) -> SharedString {
    if available
        .iter()
        .any(|name| name.eq_ignore_ascii_case(DISCORD_FAMILY))
    {
        DISCORD_FAMILY.into()
    } else {
        BUNDLED_FAMILY.into()
    }
}
