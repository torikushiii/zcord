mod members;
mod menus;
mod panel;

use members::remember_channel;
pub(crate) use members::{format_elapsed, live_badge, member_row};
use menus::menu_frame;
pub(crate) use menus::{menu, switch};
pub(crate) use panel::{panel, unavailable};

use std::{
    cell::Cell,
    collections::HashSet,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};
use gpui_component::ElementExt;

mod details;
pub(crate) mod device_menu;
pub(crate) mod self_preview;
mod share_panel;
pub(crate) mod share_picker;
pub(crate) mod stream_card;
mod stream_quality;

use crate::{
    app::{DiscordApp, calls::call_view::CallView, messages::message_body::label},
    discord::{ShareEvent, VoiceEvent},
    settings::{CustomStream, SavedChannel, StreamMode},
    store::{Store, VoiceMember},
    theme::{self, rpx},
};

pub(crate) const MEMBER_HEIGHT: f32 = 32.;
const MENU_EXIT: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(crate) struct Voice {
    pub(crate) self_mute: bool,
    pub(crate) self_deaf: bool,
    pub(crate) call: Option<Call>,
    pub(crate) speaking: HashSet<u64>,
    pub(crate) noise_suppression: bool,
    pub(crate) call_view: Option<CallView>,
    hovered_control: Option<Control>,
    status_hovered: bool,
    /// Until the first hover the title sits still, so it doesn't play the slide-back
    /// animation when the panel first appears.
    status_hovered_once: bool,
    menu: Option<Menu>,
    /// Which menu the last mouse-down-outside closed, and when.
    menu_closed: Option<(MenuKind, Instant)>,
    menu_opened: u64,
    menu_close: Option<Task<()>>,
    details_tab: DetailsTab,
    privacy_copied: bool,
    privacy_copied_reset: Option<Task<()>>,
    restored: bool,
    clock: Option<Task<()>>,
    sharing: Sharing,
    share_picker: Option<share_picker::SharePicker>,
    share_error: Option<String>,
    share_info: Option<share_panel::ShareInfo>,
    self_preview: self_preview::SelfPreview,
    stream_mode: StreamMode,
    stream_custom: CustomStream,
    stream_preview_hidden: bool,
    stream_audio_muted: bool,
    /// The stream preview upload in flight; a newer preview or the share ending drops it.
    preview_upload: Option<Task<()>>,
    stream_card: Option<stream_card::StreamCard>,
    stream_card_close: Option<Task<()>>,
    stream_card_hovered: bool,
    /// `(channel, user)` to watch once the call that was joined for it connects.
    pending_watch: Option<(u64, u64)>,
    /// The audio devices as last listed, for the device menus and Voice & Video settings.
    devices: Option<crate::discord::AudioDevices>,
    devices_loading: Option<Task<()>>,
    volume_slider: Option<device_menu::VolumeSlider>,
    output_slider: Option<device_menu::VolumeSlider>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Sharing {
    #[default]
    Off,
    Starting,
    Live,
}

#[derive(Clone, Copy)]
pub(crate) struct Joined {
    pub(crate) channel_id: u64,
    pub(crate) elapsed: Option<Duration>,
}

impl Voice {
    pub(crate) fn joined_call(&self) -> Option<(u64, u64)> {
        self.call
            .as_ref()
            .map(|call| (call.guild_id, call.channel_id))
    }

    pub(crate) fn sharing_screen(&self) -> bool {
        self.sharing != Sharing::Off
    }

    pub(crate) fn joined(&self, store: &Store) -> Option<Joined> {
        self.call.as_ref().map(|call| Joined {
            channel_id: call.channel_id,
            elapsed: store
                .voice_started_at(call.guild_id, call.channel_id)
                .map(|at| at.elapsed().unwrap_or_default()),
        })
    }

    pub(crate) fn from_settings(cx: &App) -> Self {
        let settings = crate::settings::get(cx);
        Self {
            self_mute: settings.voice_mute,
            self_deaf: settings.voice_deaf,
            noise_suppression: !settings.voice_noise_suppression_off,
            stream_mode: settings.stream_mode,
            stream_custom: settings.stream_custom.unwrap_or_default(),
            stream_preview_hidden: settings.stream_preview_hidden,
            stream_audio_muted: settings.stream_audio_muted,
            ..Self::default()
        }
    }
}

/// Voice panel controls that show a Discord-style tooltip while hovered.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Control {
    Signal,
    NoiseSuppression,
    Disconnect,
    Screen,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuKind {
    NoiseSuppression,
    Details,
    Stream,
    InputOptions,
    OutputOptions,
    CallAudio,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum DetailsTab {
    #[default]
    Connection,
    Privacy,
}

struct Menu {
    kind: MenuKind,
    position: Point<Pixels>,
    closing: bool,
    /// Unique per open, so reopening replays the entry animation.
    animation: u64,
    /// The switch only slides after a click; opening shows it at rest.
    switch_moved: bool,
    /// The stream menu's quality flyout is showing.
    flyout: bool,
    /// The flyout lies outside the menu; a click there must not count as outside.
    pointer_in_flyout: bool,
    flyout_left: bool,
    device_flyout: Option<device_menu::Flyout>,
    /// Dropped with the menu so its ticking stops.
    level: Option<Entity<device_menu::LevelBars>>,
}

/// `Pending` until Discord either finishes the DAVE handshake, which yields the privacy
/// code, or reports that the call isn't end-to-end encrypted.
#[derive(Clone, Default)]
pub(crate) enum Encryption {
    #[default]
    Pending,
    Off,
    On(String),
}

impl Encryption {
    fn privacy_code(&self) -> Option<&str> {
        match self {
            Self::On(code) if !code.trim().is_empty() => Some(code),
            _ => None,
        }
    }
}

pub(crate) struct Call {
    pub(crate) guild_id: u64,
    pub(crate) channel_id: u64,
    pub(crate) connected: bool,
    pub(crate) encryption: Encryption,
    pings: Vec<PingSample>,
    server: Option<String>,
    packet_loss: Option<f32>,
}

#[derive(Clone, Copy)]
struct PingSample {
    unix: i64,
    ms: u32,
}

/// Heartbeats arrive about every 14 seconds, so this covers the graph's window with room
/// to spare.
const PING_HISTORY: usize = 64;

impl Call {
    fn new(guild_id: u64, channel_id: u64) -> Self {
        Self {
            guild_id,
            channel_id,
            connected: false,
            encryption: Encryption::Pending,
            pings: Vec::new(),
            server: None,
            packet_loss: None,
        }
    }

    fn last_ping(&self) -> Option<u32> {
        self.pings.last().map(|sample| sample.ms)
    }
}

impl DiscordApp {
    pub(crate) fn refresh_voice_start_times(&self) {
        if let Some(call) = &self.voice.call
            && call.guild_id != 0
            && let Some(session) = self.session.as_ref().filter(|_| self.live)
        {
            session.request_voice_start_times(call.guild_id);
        }
    }

    pub(crate) fn join_voice(&mut self, guild_id: u64, channel_id: u64, cx: &mut Context<Self>) {
        if !self.store.can_connect_voice(guild_id, channel_id)
            || self
                .voice
                .call
                .as_ref()
                .is_some_and(|call| call.channel_id == channel_id)
        {
            return;
        }
        let Some(session) = self.session.as_ref().filter(|_| self.live) else {
            return;
        };
        self.calls.cancel_pending();
        session.set_self_mute(self.voice.self_mute);
        session.set_self_deaf(self.voice.self_deaf);
        session.set_noise_suppression(self.voice.noise_suppression);
        crate::app::settings_page::apply_preferences(session, &crate::settings::get(cx));
        if guild_id == 0 {
            session.join_dm_call(channel_id);
        } else {
            session.join_voice(guild_id, channel_id);
        }
        self.voice.call = Some(Call::new(guild_id, channel_id));
        self.voice.speaking.clear();
        if self.voice_lobby == Some((guild_id, channel_id)) {
            self.voice_lobby = None;
            self.open_call_view(cx);
        }
        self.sync_channel_rows();
        cx.notify();
    }

    pub(crate) fn leave_voice(&mut self, cx: &mut Context<Self>) {
        self.calls.cancel_pending();
        if self.voice.call.take().is_none() {
            return;
        }
        if let Some(session) = &self.session {
            session.leave_voice();
        }
        remember_channel(None, cx);
        self.voice.call_view = None;
        self.voice.pending_watch = None;
        self.voice.speaking.clear();
        self.voice.menu = None;
        self.voice.hovered_control = None;
        self.voice.status_hovered = false;
        self.voice.menu_close = None;
        self.voice.clock = None;
        self.sync_channel_rows();
        cx.notify();
    }

    pub(crate) fn toggle_self_mute(&mut self, cx: &mut Context<Self>) {
        let voice = &mut self.voice;
        // Unmuting while deafened undeafens too, like Discord.
        if voice.self_deaf {
            voice.self_deaf = false;
            voice.self_mute = false;
        } else {
            voice.self_mute = !voice.self_mute;
        }
        self.sync_voice_flags(cx);
        cx.notify();
    }

    pub(crate) fn toggle_self_deaf(&mut self, cx: &mut Context<Self>) {
        self.voice.self_deaf = !self.voice.self_deaf;
        self.sync_voice_flags(cx);
        cx.notify();
    }

    /// Changing it by hand makes the Input Profile Custom, as in Discord.
    pub(crate) fn toggle_noise_suppression(&mut self, cx: &mut Context<Self>) {
        self.set_noise_suppression(!self.voice.noise_suppression, cx);
        crate::settings::update(cx, |settings| {
            settings.voice_input_profile = Some(crate::settings::InputProfile::Custom)
        });
    }

    pub(crate) fn set_noise_suppression(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.voice.noise_suppression = enabled;
        if let Some(session) = &self.session {
            session.set_noise_suppression(enabled);
        }
        crate::settings::update(cx, |settings| {
            settings.voice_noise_suppression_off = !enabled
        });
        if let Some(menu) = &mut self.voice.menu {
            menu.switch_moved = true;
        }
        cx.notify();
    }

    pub(crate) fn stream_menu_open(&self) -> bool {
        self.voice
            .menu
            .as_ref()
            .is_some_and(|menu| menu.kind == MenuKind::Stream && !menu.closing)
    }

    pub(crate) fn toggle_live_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.toggle_menu(MenuKind::Stream, position, cx);
    }

    fn toggle_menu(&mut self, kind: MenuKind, position: Point<Pixels>, cx: &mut Context<Self>) {
        let voice = &mut self.voice;
        if voice
            .menu
            .as_ref()
            .is_some_and(|menu| menu.kind == kind && !menu.closing)
        {
            self.close_menu(cx);
            return;
        }
        // The press that closed this menu via mouse-down-outside must not reopen it here.
        let just_closed = voice.menu_closed.is_some_and(|(closed, at)| {
            closed == kind && at.elapsed() < Duration::from_millis(150)
        });
        if just_closed {
            return;
        }
        voice.menu_opened = voice.menu_opened.wrapping_add(1);
        voice.menu_close = None;
        voice.menu = Some(Menu {
            kind,
            position,
            closing: false,
            animation: voice.menu_opened,
            switch_moved: false,
            flyout: false,
            pointer_in_flyout: false,
            flyout_left: false,
            device_flyout: None,
            level: None,
        });
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.voice.menu.as_mut().filter(|menu| !menu.closing) else {
            return;
        };
        menu.closing = true;
        let animation = menu.animation;
        self.voice.menu_closed = Some((menu.kind, Instant::now()));
        self.voice.menu_close = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MENU_EXIT).await;
            this.update(cx, |app, cx| {
                let voice = &mut app.voice;
                if voice
                    .menu
                    .as_ref()
                    .is_some_and(|menu| menu.closing && menu.animation == animation)
                {
                    voice.menu = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn mark_privacy_code_copied(&mut self, cx: &mut Context<Self>) {
        self.voice.privacy_copied = true;
        self.voice.privacy_copied_reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            this.update(cx, |app, cx| {
                app.voice.privacy_copied = false;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn select_details_tab(&mut self, tab: DetailsTab, cx: &mut Context<Self>) {
        if tab == DetailsTab::Privacy
            && self
                .voice
                .call
                .as_ref()
                .and_then(|call| call.encryption.privacy_code())
                .is_none()
        {
            return;
        }
        self.voice.details_tab = tab;
        cx.notify();
    }

    fn sync_voice_flags(&self, cx: &mut App) {
        if let Some(session) = &self.session {
            session.set_self_mute(self.voice.self_mute);
            session.set_self_deaf(self.voice.self_deaf);
        }
        let (mute, deaf) = (self.voice.self_mute, self.voice.self_deaf);
        crate::settings::update(cx, |settings| {
            settings.voice_mute = mute;
            settings.voice_deaf = deaf;
        });
    }

    /// Rejoins the voice channel zcord was in when it last quit, like Discord's desktop client,
    /// but only while Discord still lists us there: after a long absence it drops the stale
    /// voice state for inactivity, and the call should stay ended. Runs once per launch, after
    /// the first READY_SUPPLEMENTAL, which carries the initial voice states.
    pub(crate) fn restore_voice(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.voice.restored, true) {
            return;
        }
        let Some(saved) = crate::settings::get(cx).voice_channel else {
            return;
        };
        let still_listed = self
            .store
            .voice_members(saved.guild_id, saved.channel_id)
            .iter()
            .any(|member| member.user_id == self.store.current_user_id);
        if still_listed {
            if saved.guild_id == 0 {
                self.start_dm_call(saved.channel_id, false, cx);
            } else {
                self.join_voice(saved.guild_id, saved.channel_id, cx);
            }
        } else {
            remember_channel(None, cx);
        }
    }

    /// Repaints the channel list once a second for the call timer beside the joined channel.
    fn start_call_clock(&mut self, cx: &mut Context<Self>) {
        if self.voice.clock.is_some() {
            return;
        }
        self.voice.clock = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update(cx, |app, cx| {
                    app.views.channel_list.update(cx, |_, cx| cx.notify());
                    app.tick_call_view(cx);
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    pub(crate) fn handle_voice_event(&mut self, event: VoiceEvent, cx: &mut Context<Self>) {
        if let VoiceEvent::Share(ShareEvent::Preview {
            stream_key,
            thumbnail,
        }) = event
        {
            self.upload_stream_preview(stream_key, thumbnail, cx);
            return;
        }
        let voice = &mut self.voice;
        match event {
            VoiceEvent::Connecting {
                guild_id,
                channel_id,
            } => {
                let previous = voice
                    .call
                    .take()
                    .filter(|call| call.channel_id == channel_id);
                let mut call = Call::new(guild_id, channel_id);
                if let Some(previous) = previous {
                    call.encryption = previous.encryption;
                    call.pings = previous.pings;
                }
                voice.call = Some(call);
            }
            VoiceEvent::Connected {
                guild_id,
                channel_id,
            } => {
                match &mut voice.call {
                    Some(call) if call.channel_id == channel_id => {
                        call.connected = true;
                    }
                    _ => {
                        voice.call = Some(Call {
                            connected: true,
                            ..Call::new(guild_id, channel_id)
                        })
                    }
                }
                self.start_call_clock(cx);
                self.refresh_voice_start_times();
                if guild_id == 0 {
                    self.ring_pending_dm_call(channel_id, cx);
                }
                remember_channel(
                    Some(SavedChannel {
                        guild_id,
                        channel_id,
                    }),
                    cx,
                );
                if let Some((_, user_id)) = self
                    .voice
                    .pending_watch
                    .take_if(|(channel, _)| *channel == channel_id)
                {
                    self.watch_stream(user_id, None, cx);
                }
            }
            VoiceEvent::Disconnected { reason } => {
                self.calls.cancel_pending();
                if let Some(reason) = reason {
                    log::warn!("Voice disconnected: {reason}");
                }
                voice.call = None;
                voice.call_view = None;
                voice.pending_watch = None;
                voice.menu = None;
                voice.menu_close = None;
                voice.hovered_control = None;
                voice.status_hovered = false;
                voice.speaking.clear();
                voice.clock = None;
                remember_channel(None, cx);
            }
            VoiceEvent::Speaking { user_id, speaking } => {
                let changed = if speaking {
                    voice.speaking.insert(user_id)
                } else {
                    voice.speaking.remove(&user_id)
                };
                // Speaking flips several times a second; only these two show it.
                if changed {
                    self.views.channel_list.update(cx, |_, cx| cx.notify());
                    if self.voice.call_view.is_some() {
                        self.views.overlays.update(cx, |_, cx| cx.notify());
                    }
                }
                return;
            }
            VoiceEvent::Ping(ms) => {
                if let Some(call) = &mut voice.call {
                    if call.pings.len() == PING_HISTORY {
                        call.pings.remove(0);
                    }
                    call.pings.push(PingSample {
                        unix: chrono::Utc::now().timestamp(),
                        ms,
                    });
                }
                self.notify_voice_stats(cx);
                return;
            }
            VoiceEvent::Server(host) => {
                if let Some(call) = &mut voice.call {
                    call.server = Some(host);
                }
                self.notify_voice_stats(cx);
                return;
            }
            VoiceEvent::PacketLoss(percent) => {
                if let Some(call) = &mut voice.call {
                    call.packet_loss = Some(percent);
                }
                self.notify_voice_stats(cx);
                return;
            }
            VoiceEvent::PrivacyCode(code) => {
                if let Some(call) = &mut voice.call {
                    call.encryption = code.map_or(Encryption::Off, Encryption::On);
                    if call.encryption.privacy_code().is_none() {
                        voice.details_tab = DetailsTab::Connection;
                    }
                }
                voice.privacy_copied = false;
                voice.privacy_copied_reset = None;
                cx.notify();
                return;
            }
            VoiceEvent::Stream(event) => self.handle_stream_event(event, cx),
            VoiceEvent::Share(event) => match event {
                ShareEvent::Starting => {
                    voice.sharing = Sharing::Starting;
                    self.start_self_preview(cx);
                }
                ShareEvent::Live => {
                    voice.sharing = Sharing::Live;
                    self.start_self_preview(cx);
                }
                ShareEvent::Ended { reason } => {
                    voice.sharing = Sharing::Off;
                    voice.share_error = reason;
                    voice.preview_upload = None;
                    self.clear_share_info(cx);
                    self.own_stream_ended(cx);
                }
                ShareEvent::Viewers(viewers) => {
                    if let Some(info) = &mut voice.share_info {
                        info.viewers = viewers;
                    }
                }
                ShareEvent::Preview { .. } => {}
            },
        }
        self.sync_channel_rows();
        cx.notify();
    }
}

impl DiscordApp {
    /// Connection stats show in the signal tooltip, which lives in the channel list, and in
    /// the details menu, which lives in the overlay; only repaint the overlay when it's open.
    fn notify_voice_stats(&mut self, cx: &mut Context<Self>) {
        if self
            .voice
            .menu
            .as_ref()
            .is_some_and(|menu| menu.kind == MenuKind::Details)
        {
            cx.notify();
        } else {
            self.views.channel_list.update(cx, |_, cx| cx.notify());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Call, Duration, Store, Voice};
    use crate::discord::{Event, model};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn store_with_start(start: Option<u64>) -> Store {
        let ready = serde_json::from_str(include_str!(
            "../../../crates/discord/src/fixtures/ready.json"
        ))
        .unwrap();
        let mut store = Store::from_ready(ready);
        store.apply(Event::VoiceChannelStartTimeUpdate(
            model::VoiceChannelStartTimeUpdate {
                guild_id: 200,
                id: 2005,
                voice_start_time: Some(start),
            },
        ));
        store
    }

    #[test]
    fn new_voice_connections_use_the_existing_server_session_duration() {
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 2400;
        let store = store_with_start(Some(start));
        for _ in 0..2 {
            let voice = Voice {
                call: Some(Call::new(200, 2005)),
                ..Voice::default()
            };
            let joined = voice.joined(&store).unwrap();
            assert_eq!(joined.channel_id, 2005);
            assert!(joined.elapsed.unwrap() >= Duration::from_secs(2400));
        }
    }

    #[test]
    fn unknown_session_is_not_replaced_with_a_personal_stopwatch() {
        let voice = Voice {
            call: Some(Call::new(200, 2005)),
            ..Voice::default()
        };
        assert_eq!(voice.joined(&store_with_start(None)).unwrap().elapsed, None);
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 86400;
        assert_eq!(
            voice
                .joined(&store_with_start(Some(future)))
                .unwrap()
                .elapsed,
            Some(Duration::ZERO)
        );
    }
}
