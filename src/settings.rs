use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    time::Duration,
};

use gpui::{App, Bounds, Global, Pixels, Task, Window, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

use crate::discord::gifs::GifProvider;

const SAVE_DELAY: Duration = Duration::from_millis(500);

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub zoom: Option<usize>,
    #[serde(default)]
    pub window: Option<SavedWindow>,
    #[serde(default)]
    pub last_channel: Option<SavedChannel>,
    #[serde(default)]
    pub guild_channels: HashMap<u64, u64>,
    #[serde(default)]
    pub recent_emojis: Vec<String>,
    #[serde(default)]
    pub collapsed_categories: HashSet<u64>,
    #[serde(default)]
    pub video_audio: Option<VideoAudio>,
    #[serde(default)]
    pub gif_provider: GifProvider,
    #[serde(default)]
    pub voice_channel: Option<SavedChannel>,
    #[serde(default)]
    pub voice_mute: bool,
    #[serde(default)]
    pub voice_deaf: bool,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VideoAudio {
    pub volume: f64,
    pub muted: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedChannel {
    pub guild_id: u64,
    pub channel_id: u64,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

struct State {
    settings: Settings,
    revision: u64,
    saved_revision: u64,
    pending_save: Option<Task<()>>,
}

impl Global for State {}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("zcord").join("settings.json"))
}

pub fn init(cx: &mut App) {
    let settings = path()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    cx.set_global(State {
        settings,
        revision: 0,
        saved_revision: 0,
        pending_save: None,
    });
}

pub fn get(cx: &App) -> Settings {
    cx.try_global::<State>()
        .map(|state| state.settings.clone())
        .unwrap_or_default()
}

pub fn update(cx: &mut App, change: impl FnOnce(&mut Settings)) {
    let Some(state) = cx.try_global::<State>() else {
        return;
    };
    let mut settings = state.settings.clone();
    change(&mut settings);
    if settings == state.settings {
        return;
    }
    let revision = state.revision + 1;
    let snapshot = settings.clone();
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(SAVE_DELAY).await;
        cx.background_executor()
            .spawn(async move { write(&snapshot) })
            .await;
        cx.update(|cx| {
            let state = cx.global_mut::<State>();
            state.saved_revision = state.saved_revision.max(revision);
        });
    });
    let state = cx.global_mut::<State>();
    state.settings = settings;
    state.revision = revision;
    state.pending_save = Some(task);
}

pub fn flush(cx: &mut App) {
    let Some(state) = cx.try_global::<State>() else {
        return;
    };
    if state.saved_revision != state.revision {
        let settings = state.settings.clone();
        let state = cx.global_mut::<State>();
        state.saved_revision = state.revision;
        state.pending_save = None;
        write(&settings);
    }
}

fn write(settings: &Settings) {
    let Some(path) = path() else {
        return;
    };
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, bytes)?;
        fs::rename(&temp, &path)
    })();
    if let Err(error) = result {
        log::warn!("Could not save settings: {error}");
    }
}

pub fn remember_window(window: &Window, cx: &mut App) {
    let (bounds, maximized) = match window.window_bounds() {
        WindowBounds::Windowed(bounds) => (bounds, false),
        WindowBounds::Maximized(bounds) | WindowBounds::Fullscreen(bounds) => (bounds, true),
    };
    let saved = SavedWindow {
        x: bounds.origin.x.as_f32(),
        y: bounds.origin.y.as_f32(),
        width: bounds.size.width.as_f32(),
        height: bounds.size.height.as_f32(),
        maximized,
    };
    update(cx, |settings| settings.window = Some(saved));
}

pub fn initial_window_bounds(min_size: gpui::Size<Pixels>, cx: &App) -> Option<WindowBounds> {
    let saved = get(cx).window?;
    let bounds = Bounds::new(
        point(px(saved.x), px(saved.y)),
        size(
            px(saved.width).max(min_size.width),
            px(saved.height).max(min_size.height),
        ),
    );
    let visible = cx.displays().iter().any(|display| {
        let screen = display.bounds();
        let overlap = screen.intersect(&bounds);
        overlap.size.width >= px(100.) && overlap.size.height >= px(50.)
    });
    if !visible {
        return None;
    }
    Some(if saved.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    })
}
