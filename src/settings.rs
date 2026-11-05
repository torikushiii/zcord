use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
    time::Duration,
};

use gpui::{App, Bounds, Global, Pixels, Task, Window, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

use crate::discord::{IdentityCache, gifs::GifProvider};

const SAVE_DELAY: Duration = Duration::from_millis(500);

static CLOCK_24H: AtomicBool = AtomicBool::new(false);
static GIF_AUTOPLAY: AtomicBool = AtomicBool::new(true);
static SMOOTH_SCROLL: AtomicBool = AtomicBool::new(true);
static EMOJI_ANIMATION: AtomicBool = AtomicBool::new(true);
static LINK_PREVIEWS: AtomicBool = AtomicBool::new(true);
static NOTIFICATIONS: AtomicBool = AtomicBool::new(true);
static NOTIFICATION_SOUND: AtomicBool = AtomicBool::new(true);
#[cfg(windows)]
static CLOSE_TO_TRAY: AtomicBool = AtomicBool::new(true);
static GAME_ACTIVITY: AtomicBool = AtomicBool::new(false);
static CHAT_FONT_SIZE: AtomicU32 = AtomicU32::new(16.0f32.to_bits());

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub discord_identity: IdentityCache,
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
    #[serde(default)]
    pub voice_noise_suppression_off: bool,
    #[serde(default)]
    pub voice_input_device: Option<String>,
    #[serde(default)]
    pub voice_output_device: Option<String>,
    #[serde(default)]
    pub voice_input_volume: Option<f32>,
    #[serde(default)]
    pub voice_output_volume: Option<f32>,
    #[serde(default)]
    pub voice_sensitivity: Option<f32>,
    #[serde(default)]
    pub gif_autoplay_off: bool,
    #[serde(default)]
    pub smooth_scroll_off: bool,
    #[serde(default)]
    pub emoji_animation_off: bool,
    #[serde(default)]
    pub link_previews_off: bool,
    #[serde(default)]
    pub notifications_off: bool,
    #[serde(default)]
    pub notification_sound_off: bool,
    #[cfg(windows)]
    #[serde(default)]
    pub close_to_tray_off: bool,
    #[serde(default)]
    pub chat_font_size: Option<f32>,
    #[serde(default)]
    pub clock_24h: bool,
    #[serde(default)]
    pub light_theme: bool,
    #[serde(default)]
    pub theme_sync_off: bool,
    #[serde(default)]
    pub reduce_motion: Option<bool>,
    #[serde(default)]
    pub share_game_activity: bool,
    #[serde(default)]
    pub stream_mode: StreamMode,
    #[serde(default)]
    pub stream_custom: Option<CustomStream>,
    #[serde(default)]
    pub stream_preview_hidden: bool,
    #[serde(default)]
    pub stream_audio_muted: bool,
    #[serde(default)]
    pub composer_send_button: bool,
    /// The Input Profile picked in the mic's options; `None` until one is, which shows as
    /// Voice Isolation while noise suppression is on.
    #[serde(default)]
    pub voice_input_profile: Option<InputProfile>,
}

/// Discord's Input Profile presets for the microphone's processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputProfile {
    /// Noise suppression on.
    VoiceIsolation,
    /// No processing: noise suppression off.
    Studio,
    /// Whatever the settings were set to by hand.
    Custom,
}

/// The Go Live picker's Stream Mode, which Discord calls a preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamMode {
    #[default]
    Gaming,
    Screenshare,
    Custom,
}

/// The resolution (output height, 0 for source) and frame rate picked under Custom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomStream {
    pub resolution: u32,
    pub fps: u32,
}

impl Default for CustomStream {
    fn default() -> Self {
        Self {
            resolution: 720,
            fps: 30,
        }
    }
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

pub fn data_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("zcord"))
}

fn path() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("settings.json"))
}

fn load() -> Settings {
    path()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub(crate) fn discord_identity() -> IdentityCache {
    load().discord_identity
}

pub fn init(cx: &mut App) {
    let settings = load();
    let identity = settings.discord_identity.clone();
    cx.set_global(State {
        settings,
        revision: 0,
        saved_revision: 0,
        pending_save: None,
    });
    let refresh = gpui_tokio::Tokio::spawn(cx, crate::discord::refresh_identity(identity));
    cx.spawn(async move |cx| {
        let Ok(identity) = refresh.await else {
            return;
        };
        cx.update(|cx| update(cx, |settings| settings.discord_identity = identity));
    })
    .detach();
}

pub fn get(cx: &App) -> Settings {
    cx.try_global::<State>()
        .map(|state| state.settings.clone())
        .unwrap_or_default()
}

pub fn clock_format(hour12: &'static str, hour24: &'static str) -> &'static str {
    if clock_24h() { hour24 } else { hour12 }
}

pub fn clock_24h() -> bool {
    CLOCK_24H.load(Ordering::Relaxed)
}

pub fn set_clock_24h(cx: &mut App, on: bool) {
    CLOCK_24H.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.clock_24h = on);
    cx.refresh_windows();
}

pub fn set_reduce_motion(cx: &mut App, value: Option<bool>) {
    // gpui does not expose the OS reduce-motion preference, only the current value.
    cx.set_reduce_motion(value.unwrap_or(false));
    update(cx, |settings| settings.reduce_motion = value);
    cx.refresh_windows();
}

pub fn set_gif_autoplay(cx: &mut App, on: bool) {
    GIF_AUTOPLAY.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.gif_autoplay_off = !on);
    cx.refresh_windows();
}

pub fn gif_autoplay() -> bool {
    GIF_AUTOPLAY.load(Ordering::Relaxed)
}

pub fn set_smooth_scroll(cx: &mut App, on: bool) {
    SMOOTH_SCROLL.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.smooth_scroll_off = !on);
    cx.refresh_windows();
}

pub fn smooth_scroll() -> bool {
    SMOOTH_SCROLL.load(Ordering::Relaxed)
}

pub fn set_emoji_animation(cx: &mut App, on: bool) {
    EMOJI_ANIMATION.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.emoji_animation_off = !on);
    cx.refresh_windows();
}

pub fn emoji_animation() -> bool {
    EMOJI_ANIMATION.load(Ordering::Relaxed)
}

pub fn set_link_previews(cx: &mut App, on: bool) {
    LINK_PREVIEWS.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.link_previews_off = !on);
    cx.refresh_windows();
}

pub fn link_previews() -> bool {
    LINK_PREVIEWS.load(Ordering::Relaxed)
}

pub fn set_notifications(cx: &mut App, on: bool) {
    NOTIFICATIONS.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.notifications_off = !on);
    cx.refresh_windows();
}

pub fn notifications() -> bool {
    NOTIFICATIONS.load(Ordering::Relaxed)
}

pub fn set_notification_sound(cx: &mut App, on: bool) {
    NOTIFICATION_SOUND.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.notification_sound_off = !on);
    cx.refresh_windows();
}

pub fn notification_sound() -> bool {
    NOTIFICATION_SOUND.load(Ordering::Relaxed)
}

#[cfg(windows)]
pub fn set_close_to_tray(cx: &mut App, on: bool) {
    CLOSE_TO_TRAY.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.close_to_tray_off = !on);
    cx.refresh_windows();
}

#[cfg(windows)]
pub fn close_to_tray() -> bool {
    CLOSE_TO_TRAY.load(Ordering::Relaxed)
}

pub fn set_game_activity(cx: &mut App, on: bool) {
    GAME_ACTIVITY.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.share_game_activity = on);
    cx.refresh_windows();
}

pub fn game_activity() -> bool {
    GAME_ACTIVITY.load(Ordering::Relaxed)
}

fn clamp_chat_font_size(size: f32) -> f32 {
    size.clamp(12.0, 24.0)
}

fn chat_font_setting(size: f32) -> Option<f32> {
    let size = clamp_chat_font_size(size);
    (size != 16.0).then_some(size)
}

pub fn set_chat_font_size(cx: &mut App, size: f32) {
    let size = clamp_chat_font_size(size);
    CHAT_FONT_SIZE.store(size.to_bits(), Ordering::Relaxed);
    update(cx, |settings| {
        settings.chat_font_size = chat_font_setting(size)
    });
    cx.refresh_windows();
}

pub fn chat_font_size() -> f32 {
    f32::from_bits(CHAT_FONT_SIZE.load(Ordering::Relaxed))
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::json;

    #[test]
    fn clock_preference_switches_time_format() {
        let time = NaiveDate::from_ymd_opt(2024, 1, 2)
            .unwrap()
            .and_hms_opt(13, 5, 9)
            .unwrap();
        let timestamp = crate::store::Timestamp {
            year: 2024,
            month: 1,
            day: 2,
            hour: 13,
            minute: 5,
        };
        CLOCK_24H.store(false, Ordering::Relaxed);
        assert_eq!(timestamp.time(), "1:05 PM");
        assert_eq!(timestamp.full(), "1/2/24, 1:05 PM");
        assert_eq!(
            time.format(clock_format("%-I:%M %p", "%H:%M")).to_string(),
            "1:05 PM"
        );
        assert_eq!(
            time.format(clock_format("%-I:%M:%S %p", "%H:%M:%S"))
                .to_string(),
            "1:05:09 PM"
        );
        assert_eq!(
            time.format(clock_format(
                "%A, %B %-d, %Y %-I:%M %p",
                "%A, %B %-d, %Y %H:%M"
            ))
            .to_string(),
            "Tuesday, January 2, 2024 1:05 PM"
        );
        CLOCK_24H.store(true, Ordering::Relaxed);
        assert_eq!(timestamp.time(), "13:05");
        assert_eq!(timestamp.full(), "1/2/24, 13:05");
        assert_eq!(
            time.format(clock_format("%-I:%M %p", "%H:%M")).to_string(),
            "13:05"
        );
        assert_eq!(
            time.format(clock_format("%-I:%M:%S %p", "%H:%M:%S"))
                .to_string(),
            "13:05:09"
        );
        assert_eq!(
            time.format(clock_format(
                "%A, %B %-d, %Y %-I:%M %p",
                "%A, %B %-d, %Y %H:%M"
            ))
            .to_string(),
            "Tuesday, January 2, 2024 13:05"
        );
        CLOCK_24H.store(false, Ordering::Relaxed);
    }

    #[test]
    fn animation_and_chat_font_atomics_round_trip() {
        GIF_AUTOPLAY.store(false, Ordering::Relaxed);
        assert!(!gif_autoplay());
        GIF_AUTOPLAY.store(true, Ordering::Relaxed);
        assert!(gif_autoplay());

        SMOOTH_SCROLL.store(false, Ordering::Relaxed);
        assert!(!smooth_scroll());
        SMOOTH_SCROLL.store(true, Ordering::Relaxed);
        assert!(smooth_scroll());

        EMOJI_ANIMATION.store(false, Ordering::Relaxed);
        assert!(!emoji_animation());
        EMOJI_ANIMATION.store(true, Ordering::Relaxed);
        assert!(emoji_animation());

        LINK_PREVIEWS.store(false, Ordering::Relaxed);
        assert!(!link_previews());
        LINK_PREVIEWS.store(true, Ordering::Relaxed);
        assert!(link_previews());

        NOTIFICATIONS.store(false, Ordering::Relaxed);
        assert!(!notifications());
        NOTIFICATIONS.store(true, Ordering::Relaxed);
        assert!(notifications());

        NOTIFICATION_SOUND.store(false, Ordering::Relaxed);
        assert!(!notification_sound());
        NOTIFICATION_SOUND.store(true, Ordering::Relaxed);
        assert!(notification_sound());

        GAME_ACTIVITY.store(true, Ordering::Relaxed);
        assert!(game_activity());
        GAME_ACTIVITY.store(false, Ordering::Relaxed);
        assert!(!game_activity());

        CHAT_FONT_SIZE.store(20.0f32.to_bits(), Ordering::Relaxed);
        assert_eq!(chat_font_size(), 20.0);
        CHAT_FONT_SIZE.store(16.0f32.to_bits(), Ordering::Relaxed);
        assert_eq!(chat_font_size(), 16.0);
    }

    #[test]
    fn chat_font_size_clamps_and_sixteen_persists_as_none() {
        assert_eq!(clamp_chat_font_size(8.0), 12.0);
        assert_eq!(clamp_chat_font_size(30.0), 24.0);
        assert_eq!(clamp_chat_font_size(18.0), 18.0);
        assert_eq!(chat_font_setting(16.0), None);
        assert_eq!(chat_font_setting(12.0), Some(12.0));
        assert_eq!(chat_font_setting(30.0), Some(24.0));
    }

    #[test]
    fn old_settings_json_uses_new_defaults() {
        let settings: Settings = serde_json::from_value(json!({
            "zoom": 5,
            "voice_mute": true,
            "clock_24h": true
        }))
        .unwrap();
        assert!(settings.voice_input_device.is_none());
        assert!(settings.voice_output_device.is_none());
        assert!(settings.voice_input_volume.is_none());
        assert!(settings.voice_output_volume.is_none());
        assert!(settings.voice_sensitivity.is_none());
        assert!(!settings.gif_autoplay_off);
        assert!(!settings.smooth_scroll_off);
        assert!(!settings.emoji_animation_off);
        assert!(!settings.link_previews_off);
        assert!(!settings.notifications_off);
        assert!(!settings.notification_sound_off);
        assert!(settings.chat_font_size.is_none());
        assert!(settings.clock_24h);
        assert!(!settings.light_theme);
        assert!(!settings.theme_sync_off);
        assert!(!settings.share_game_activity);
    }
}
