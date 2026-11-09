use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{
        LazyLock, RwLock,
        atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
    },
    time::Duration,
};

use gpui::{App, Bounds, Global, Pixels, Task, Window, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

use crate::discord::{IdentityCache, gifs::GifProvider, settings_proto::TextImages};

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
static USER_PANEL_EXTENDED: AtomicBool = AtomicBool::new(true);
static TYPING_HIDDEN: AtomicBool = AtomicBool::new(false);
static ALWAYS_ANIMATE: AtomicBool = AtomicBool::new(false);
static ACTIVITY_TOGGLE: AtomicBool = AtomicBool::new(false);
static CHAT_FONT_SIZE: AtomicU32 = AtomicU32::new(16.0f32.to_bits());
static PROFILE_THEME_SYNC: AtomicBool = AtomicBool::new(false);
static UNDERLINE_LINKS: AtomicBool = AtomicBool::new(false);
static QUIET_REPLIES: AtomicBool = AtomicBool::new(false);
static MENTION_AVATARS: AtomicBool = AtomicBool::new(false);
static WHO_REACTED: AtomicBool = AtomicBool::new(false);
static RICH_TYPING: AtomicBool = AtomicBool::new(false);
static WINDOW_FOCUSED: AtomicBool = AtomicBool::new(true);
static DISPLAY_NAME_STYLES: AtomicBool = AtomicBool::new(true);
static SWITCH_INDICATORS: AtomicBool = AtomicBool::new(false);
static GROUP_SPACING: AtomicU32 = AtomicU32::new(u32::MAX);
static ROLE_COLORS: AtomicU8 = AtomicU8::new(0);
static NOTIFICATION_DURATION: AtomicU32 = AtomicU32::new(DEFAULT_NOTIFICATION_SECONDS);

pub const GROUP_SPACINGS: [u32; 5] = [0, 4, 8, 16, 24];
pub const NOTIFICATION_SECONDS: std::ops::RangeInclusive<u32> = 3..=20;
pub const DEFAULT_NOTIFICATION_SECONDS: u32 = 3;
static STREAMER: RwLock<StreamerMode> = RwLock::new(StreamerMode {
    enabled: false,
    auto: true,
    hide_personal: true,
    hide_invites: true,
    mute_sounds: true,
    mute_notifications: true,
});
static DISPLAY: LazyLock<RwLock<TextImages>> = LazyLock::new(Default::default);

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub discord_identity: IdentityCache,
    #[serde(default)]
    pub accounts: Vec<SavedAccount>,
    #[serde(default)]
    pub active_account: Option<u64>,
    #[serde(default)]
    pub stashed_accounts: HashMap<u64, AccountScoped>,
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
    pub inbox_unreads_tip_dismissed: bool,
    #[serde(default)]
    pub mentions_everyone_off: bool,
    #[serde(default)]
    pub mentions_roles_off: bool,
    #[serde(default)]
    pub light_theme: bool,
    #[serde(default)]
    pub base_theme: Option<crate::theme::BaseTheme>,
    #[serde(default)]
    pub client_theme: Option<SavedClientTheme>,
    #[serde(default)]
    pub theme_follows_device: bool,
    #[serde(default)]
    pub theme_sync_off: bool,
    #[serde(default)]
    pub profile_theme_sync: bool,
    #[serde(default)]
    pub streamer: StreamerMode,
    #[serde(default)]
    pub reduce_motion: Option<bool>,
    #[serde(default)]
    pub reduce_motion_sync: Option<bool>,
    #[serde(default)]
    pub underline_links: bool,
    #[serde(default)]
    pub quiet_replies: bool,
    #[serde(default)]
    pub mention_avatars: bool,
    #[serde(default)]
    pub who_reacted: bool,
    #[serde(default)]
    pub rich_typing: bool,
    #[serde(default)]
    pub display_name_styles_off: bool,
    #[serde(default)]
    pub switch_indicators: bool,
    #[serde(default)]
    pub message_group_spacing: Option<u32>,
    #[serde(default)]
    pub role_colors: RoleColors,
    #[serde(default)]
    pub notification_seconds: Option<u32>,
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
    #[serde(default)]
    pub user_panel_compact: bool,
    #[serde(default)]
    pub typing_hidden: bool,
    #[serde(default)]
    pub always_animate: bool,
    #[serde(default)]
    pub activity_toggle: bool,
    #[serde(default)]
    pub voice_input_profile: Option<InputProfile>,
}

pub const MAX_ACCOUNTS: usize = 5;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedAccount {
    pub id: u64,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub discriminator: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub invalid: bool,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountScoped {
    #[serde(default)]
    pub last_channel: Option<SavedChannel>,
    #[serde(default)]
    pub guild_channels: HashMap<u64, u64>,
    #[serde(default)]
    pub recent_emojis: Vec<String>,
    #[serde(default)]
    pub collapsed_categories: HashSet<u64>,
    #[serde(default)]
    pub voice_channel: Option<SavedChannel>,
}

impl Settings {
    fn take_scoped(&mut self) -> AccountScoped {
        AccountScoped {
            last_channel: self.last_channel.take(),
            guild_channels: std::mem::take(&mut self.guild_channels),
            recent_emojis: std::mem::take(&mut self.recent_emojis),
            collapsed_categories: std::mem::take(&mut self.collapsed_categories),
            voice_channel: self.voice_channel.take(),
        }
    }

    fn put_scoped(&mut self, scoped: AccountScoped) {
        self.last_channel = scoped.last_channel;
        self.guild_channels = scoped.guild_channels;
        self.recent_emojis = scoped.recent_emojis;
        self.collapsed_categories = scoped.collapsed_categories;
        self.voice_channel = scoped.voice_channel;
    }

    fn stash(&mut self, user_id: u64, scoped: AccountScoped) {
        if scoped == AccountScoped::default() {
            self.stashed_accounts.remove(&user_id);
        } else {
            self.stashed_accounts.insert(user_id, scoped);
        }
    }

    pub fn activate_account(&mut self, user_id: u64) -> bool {
        if self.active_account == Some(user_id) {
            return false;
        }
        let current = self.take_scoped();
        let next = match self.active_account {
            Some(active) => {
                self.stash(active, current);
                self.stashed_accounts.remove(&user_id).unwrap_or_default()
            }
            None => self.stashed_accounts.remove(&user_id).unwrap_or(current),
        };
        self.put_scoped(next);
        self.active_account = Some(user_id);
        true
    }

    pub fn remember_account(&mut self, account: SavedAccount) -> Vec<u64> {
        let kept = account.id;
        match self.accounts.iter_mut().find(|saved| saved.id == kept) {
            Some(saved) => *saved = account,
            None => self.accounts.push(account),
        }
        let mut dropped = Vec::new();
        while self.accounts.len() > MAX_ACCOUNTS
            && let Some(index) = self.accounts.iter().rposition(|saved| saved.id != kept)
        {
            dropped.push(self.accounts.remove(index).id);
        }
        for user_id in &dropped {
            self.stashed_accounts.remove(user_id);
        }
        dropped
    }

    pub fn forget_account(&mut self, user_id: u64) {
        self.accounts.retain(|account| account.id != user_id);
        self.stashed_accounts.remove(&user_id);
        if self.active_account == Some(user_id) {
            self.take_scoped();
            self.active_account = None;
        }
    }

    pub fn set_account_invalid(&mut self, user_id: u64, invalid: bool) {
        if let Some(account) = self
            .accounts
            .iter_mut()
            .find(|account| account.id == user_id)
        {
            account.invalid = invalid;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoleColors {
    #[default]
    InNames,
    NextToNames,
    Hidden,
}

impl RoleColors {
    pub const ALL: [Self; 3] = [Self::InNames, Self::NextToNames, Self::Hidden];

    pub fn label(self) -> &'static str {
        match self {
            Self::InNames => "In names",
            Self::NextToNames => "Next to names",
            Self::Hidden => "Don't show role colors",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputProfile {
    VoiceIsolation,
    Studio,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamMode {
    #[default]
    Gaming,
    Screenshare,
    Custom,
}

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SavedClientTheme {
    Preset(u32),
    Custom {
        colors: Vec<u32>,
        angle: i32,
        base_mix: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamerMode {
    pub enabled: bool,
    pub auto: bool,
    pub hide_personal: bool,
    pub hide_invites: bool,
    pub mute_sounds: bool,
    pub mute_notifications: bool,
}

impl Default for StreamerMode {
    fn default() -> Self {
        Self {
            enabled: false,
            auto: true,
            hide_personal: true,
            hide_invites: true,
            mute_sounds: true,
            mute_notifications: true,
        }
    }
}

impl StreamerMode {
    pub fn hides_personal(&self) -> bool {
        self.enabled && self.hide_personal
    }

    pub fn mutes_sounds(&self) -> bool {
        self.enabled && self.mute_sounds
    }

    pub fn mutes_notifications(&self) -> bool {
        self.enabled && self.mute_notifications
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

pub fn accounts(cx: &App) -> Vec<SavedAccount> {
    cx.try_global::<State>()
        .map(|state| state.settings.accounts.clone())
        .unwrap_or_default()
}

pub fn active_account(cx: &App) -> Option<u64> {
    cx.try_global::<State>()
        .and_then(|state| state.settings.active_account)
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
    update(cx, |settings| settings.reduce_motion = value);
    apply_reduce_motion(cx);
    cx.refresh_windows();
}

pub fn reduce_motion_sync(cx: &App) -> bool {
    let settings = get(cx);
    settings
        .reduce_motion_sync
        .unwrap_or(settings.reduce_motion.is_none())
}

pub fn set_reduce_motion_sync(cx: &mut App, on: bool) {
    update(cx, |settings| settings.reduce_motion_sync = Some(on));
    apply_reduce_motion(cx);
    cx.refresh_windows();
}

pub fn apply_reduce_motion(cx: &mut App) {
    let on = if reduce_motion_sync(cx) {
        system_reduce_motion()
    } else {
        get(cx).reduce_motion == Some(true)
    };
    if cx.reduce_motion() != on {
        cx.set_reduce_motion(on);
        cx.refresh_windows();
    }
}

fn system_reduce_motion() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
        };
        use windows_core::BOOL;
        let mut animate = BOOL(1);
        let read = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                Some((&raw mut animate).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        read.is_ok() && !animate.as_bool()
    }
    #[cfg(not(windows))]
    false
}

pub fn set_window_focused(cx: &mut App, focused: bool) {
    if WINDOW_FOCUSED.swap(focused, Ordering::Relaxed) != focused {
        cx.refresh_windows();
    }
}

pub fn window_focused() -> bool {
    WINDOW_FOCUSED.load(Ordering::Relaxed)
}

pub fn set_quiet_replies(cx: &mut App, on: bool) {
    QUIET_REPLIES.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.quiet_replies = on);
    cx.refresh_windows();
}

pub fn quiet_replies() -> bool {
    QUIET_REPLIES.load(Ordering::Relaxed)
}

pub fn set_mention_avatars(cx: &mut App, on: bool) {
    MENTION_AVATARS.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.mention_avatars = on);
    cx.refresh_windows();
}

pub fn mention_avatars() -> bool {
    MENTION_AVATARS.load(Ordering::Relaxed)
}

pub fn set_who_reacted(cx: &mut App, on: bool) {
    WHO_REACTED.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.who_reacted = on);
    cx.refresh_windows();
}

pub fn who_reacted() -> bool {
    WHO_REACTED.load(Ordering::Relaxed)
}

pub fn set_rich_typing(cx: &mut App, on: bool) {
    RICH_TYPING.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.rich_typing = on);
    cx.refresh_windows();
}

pub fn rich_typing() -> bool {
    RICH_TYPING.load(Ordering::Relaxed)
}

pub fn set_underline_links(cx: &mut App, on: bool) {
    UNDERLINE_LINKS.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.underline_links = on);
    cx.refresh_windows();
}

pub fn underline_links() -> bool {
    UNDERLINE_LINKS.load(Ordering::Relaxed)
}

pub fn set_display_name_styles(cx: &mut App, on: bool) {
    DISPLAY_NAME_STYLES.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.display_name_styles_off = !on);
    cx.refresh_windows();
}

pub fn display_name_styles() -> bool {
    DISPLAY_NAME_STYLES.load(Ordering::Relaxed)
}

pub fn set_switch_indicators(cx: &mut App, on: bool) {
    SWITCH_INDICATORS.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.switch_indicators = on);
    cx.refresh_windows();
}

pub fn switch_indicators() -> bool {
    SWITCH_INDICATORS.load(Ordering::Relaxed)
}

fn nearest(steps: &[u32], value: u32) -> u32 {
    steps
        .iter()
        .copied()
        .min_by_key(|step| step.abs_diff(value))
        .unwrap_or(value)
}

pub fn set_message_group_spacing(cx: &mut App, spacing: Option<u32>) {
    let spacing = spacing.map(|spacing| nearest(&GROUP_SPACINGS, spacing));
    GROUP_SPACING.store(spacing.unwrap_or(u32::MAX), Ordering::Relaxed);
    update(cx, |settings| settings.message_group_spacing = spacing);
    cx.refresh_windows();
}

pub fn message_group_spacing(compact: bool) -> u32 {
    match GROUP_SPACING.load(Ordering::Relaxed) {
        u32::MAX if compact => 0,
        u32::MAX => 16,
        spacing => spacing,
    }
}

pub fn set_role_colors(cx: &mut App, mode: RoleColors) {
    ROLE_COLORS.store(mode as u8, Ordering::Relaxed);
    update(cx, |settings| settings.role_colors = mode);
    cx.refresh_windows();
}

pub fn role_colors() -> RoleColors {
    match ROLE_COLORS.load(Ordering::Relaxed) {
        1 => RoleColors::NextToNames,
        2 => RoleColors::Hidden,
        _ => RoleColors::InNames,
    }
}

pub fn set_notification_seconds(cx: &mut App, seconds: u32) {
    let seconds = seconds.clamp(*NOTIFICATION_SECONDS.start(), *NOTIFICATION_SECONDS.end());
    NOTIFICATION_DURATION.store(seconds, Ordering::Relaxed);
    update(cx, |settings| {
        settings.notification_seconds = (seconds != DEFAULT_NOTIFICATION_SECONDS).then_some(seconds)
    });
    cx.refresh_windows();
}

pub fn notification_seconds() -> u32 {
    NOTIFICATION_DURATION.load(Ordering::Relaxed)
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

pub fn set_user_panel_extended(cx: &mut App, on: bool) {
    USER_PANEL_EXTENDED.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.user_panel_compact = !on);
    cx.refresh_windows();
}

pub fn user_panel_extended() -> bool {
    USER_PANEL_EXTENDED.load(Ordering::Relaxed)
}

pub fn set_typing_hidden(cx: &mut App, on: bool) {
    TYPING_HIDDEN.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.typing_hidden = on);
    cx.refresh_windows();
}

pub fn typing_hidden() -> bool {
    TYPING_HIDDEN.load(Ordering::Relaxed)
}

pub fn set_always_animate(cx: &mut App, on: bool) {
    ALWAYS_ANIMATE.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.always_animate = on);
    cx.refresh_windows();
}

pub fn always_animate() -> bool {
    ALWAYS_ANIMATE.load(Ordering::Relaxed)
}

pub fn set_activity_toggle(cx: &mut App, on: bool) {
    ACTIVITY_TOGGLE.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.activity_toggle = on);
    cx.refresh_windows();
}

pub fn activity_toggle() -> bool {
    ACTIVITY_TOGGLE.load(Ordering::Relaxed)
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

pub fn set_profile_theme_sync(cx: &mut App, on: bool) {
    PROFILE_THEME_SYNC.store(on, Ordering::Relaxed);
    update(cx, |settings| settings.profile_theme_sync = on);
    cx.refresh_windows();
}

pub fn profile_theme_sync() -> bool {
    PROFILE_THEME_SYNC.load(Ordering::Relaxed)
}

pub fn set_streamer(cx: &mut App, streamer: StreamerMode) {
    if let Ok(mut current) = STREAMER.write() {
        *current = streamer;
    }
    update(cx, |settings| settings.streamer = streamer);
    cx.refresh_windows();
}

const BROADCASTERS: &[&str] = &[
    "obs",
    "obs32",
    "obs64",
    "xsplit.gamecaster",
    "xsplit.core",
    "gamecaster",
    "bebo",
    "streamlabs obs",
    "streamlabs desktop",
    "twitchstudio",
    "twitch studio",
];
const BROADCASTER_POLL: Duration = Duration::from_secs(5);

fn broadcaster_running() -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, ProcessRefreshKind::new());
    system.processes().values().any(|process| {
        let name = process.name().to_string_lossy().to_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        BROADCASTERS.contains(&name)
    })
}

pub fn watch_broadcasters(cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut last = Some(false);
        loop {
            if cx.update(|_| streamer().auto) {
                let running = cx
                    .background_executor()
                    .spawn(async { broadcaster_running() })
                    .await;
                let changed = last.is_some_and(|last| last != running);
                last = Some(running);
                if changed {
                    cx.update(|cx| {
                        let current = streamer();
                        if current.auto && current.enabled != running {
                            set_streamer(
                                cx,
                                StreamerMode {
                                    enabled: running,
                                    ..current
                                },
                            );
                        }
                    });
                }
            } else {
                last = None;
            }
            cx.background_executor().timer(BROADCASTER_POLL).await;
        }
    })
    .detach();
}

pub fn shown_username(username: &str) -> String {
    if streamer().hides_personal() {
        crate::store::conceal(username)
    } else {
        username.to_owned()
    }
}

pub fn streamer() -> StreamerMode {
    STREAMER
        .read()
        .map(|streamer| *streamer)
        .unwrap_or_default()
}

pub fn set_display(cx: &mut App, display: TextImages) {
    if DISPLAY.read().is_ok_and(|current| *current == display) {
        return;
    }
    GIF_AUTOPLAY.store(display.gif_auto_play, Ordering::Relaxed);
    EMOJI_ANIMATION.store(display.animate_emoji, Ordering::Relaxed);
    LINK_PREVIEWS.store(display.render_embeds, Ordering::Relaxed);
    update(cx, |settings| {
        settings.gif_autoplay_off = !display.gif_auto_play;
        settings.emoji_animation_off = !display.animate_emoji;
        settings.link_previews_off = !display.render_embeds;
    });
    if let Ok(mut current) = DISPLAY.write() {
        *current = display;
    }
    cx.refresh_windows();
}

pub fn display() -> TextImages {
    DISPLAY
        .read()
        .map(|display| display.clone())
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

    fn account(id: u64) -> SavedAccount {
        SavedAccount {
            id,
            username: format!("user{id}"),
            ..SavedAccount::default()
        }
    }

    fn channel(guild_id: u64, channel_id: u64) -> Option<SavedChannel> {
        Some(SavedChannel {
            guild_id,
            channel_id,
        })
    }

    #[test]
    fn first_account_adopts_settings_saved_before_accounts() {
        let mut settings = Settings {
            last_channel: channel(1, 2),
            recent_emojis: vec!["smile".into()],
            ..Settings::default()
        };
        assert!(settings.activate_account(10));
        assert_eq!(settings.active_account, Some(10));
        assert!(settings.last_channel == channel(1, 2));
        assert_eq!(settings.recent_emojis, ["smile"]);
        assert!(settings.stashed_accounts.is_empty());
        assert!(!settings.activate_account(10));
    }

    #[test]
    fn switching_accounts_swaps_their_own_settings() {
        let mut settings = Settings::default();
        settings.activate_account(10);
        settings.last_channel = channel(1, 2);
        settings.collapsed_categories.insert(3);
        settings.voice_channel = channel(1, 4);

        assert!(settings.activate_account(20));
        assert!(settings.last_channel.is_none());
        assert!(settings.collapsed_categories.is_empty());
        assert!(settings.voice_channel.is_none());
        settings.guild_channels.insert(5, 6);

        assert!(settings.activate_account(10));
        assert!(settings.last_channel == channel(1, 2));
        assert!(settings.collapsed_categories.contains(&3));
        assert!(settings.voice_channel == channel(1, 4));
        assert!(settings.guild_channels.is_empty());
        assert_eq!(
            settings
                .stashed_accounts
                .get(&20)
                .map(|scoped| &scoped.guild_channels),
            Some(&HashMap::from([(5, 6)]))
        );
        assert!(!settings.stashed_accounts.contains_key(&10));
    }

    #[test]
    fn remembering_accounts_updates_in_place_and_caps_the_list() {
        let mut settings = Settings::default();
        for id in 1..=5 {
            assert!(settings.remember_account(account(id)).is_empty());
        }
        settings
            .stashed_accounts
            .insert(5, AccountScoped::default());
        let renamed = SavedAccount {
            username: "renamed".into(),
            ..account(2)
        };
        assert!(settings.remember_account(renamed).is_empty());
        assert_eq!(settings.accounts[1].username, "renamed");

        assert_eq!(settings.remember_account(account(6)), [5]);
        let ids: Vec<u64> = settings.accounts.iter().map(|account| account.id).collect();
        assert_eq!(ids, [1, 2, 3, 4, 6]);
        assert!(!settings.stashed_accounts.contains_key(&5));
    }

    #[test]
    fn forgetting_the_active_account_drops_its_settings() {
        let mut settings = Settings::default();
        settings.remember_account(account(10));
        settings.remember_account(account(20));
        settings.activate_account(20);
        settings.recent_emojis = vec!["smile".into()];
        settings.activate_account(10);
        settings.last_channel = channel(1, 2);

        settings.forget_account(20);
        assert!(!settings.stashed_accounts.contains_key(&20));
        assert_eq!(settings.active_account, Some(10));

        settings.forget_account(10);
        assert_eq!(settings.active_account, None);
        assert!(settings.last_channel.is_none());
        assert!(settings.accounts.is_empty());
    }

    #[test]
    fn invalid_accounts_round_trip_through_json() {
        let mut settings = Settings::default();
        settings.remember_account(account(10));
        settings.set_account_invalid(10, true);
        settings.activate_account(10);
        settings.activate_account(20);
        settings.recent_emojis = vec!["wave".into()];
        settings.activate_account(10);
        let json = serde_json::to_value(&settings).unwrap();
        let restored: Settings = serde_json::from_value(json).unwrap();
        assert!(restored.accounts[0].invalid);
        assert_eq!(restored.active_account, Some(10));
        assert_eq!(
            restored
                .stashed_accounts
                .get(&20)
                .map(|scoped| scoped.recent_emojis.clone()),
            Some(vec!["wave".to_owned()])
        );
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
        assert!(!settings.user_panel_compact);
    }
}
