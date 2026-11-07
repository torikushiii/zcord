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
    app::{DiscordApp, call_view::CallView, message_body::label},
    discord::{ShareEvent, VoiceEvent},
    settings::{CustomStream, SavedChannel, StreamMode},
    store::{Store, VoiceMember},
    theme::{self, rpx},
};

pub(crate) const MEMBER_HEIGHT: f32 = 32.;
const MEMBER_AVATAR: f32 = 24.;
const MEMBER_INDENT: f32 = 36.;
const MENU_ENTER: Duration = Duration::from_millis(150);
const MENU_EXIT: Duration = Duration::from_millis(100);
const MENU_RISE: f32 = 6.;
const SWITCH_SLIDE: Duration = Duration::from_millis(180);

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
    pub(super) fn refresh_voice_start_times(&self) {
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

/// "Voice Connected" slides up into "Voice Details" on hover, like the account panel's
/// status line.
fn connected_title(details: bool, animate: bool, color: u32) -> AnyElement {
    const LINE: f32 = 20.;
    let lines = div()
        .relative()
        .flex()
        .flex_col()
        .child(div().h(rpx(LINE)).truncate().child("Voice Connected"))
        .child(div().h(rpx(LINE)).truncate().child("Voice Details"));
    let lines = if animate {
        let animation = Animation::new(Duration::from_millis(200)).with_easing(ease_out_quint());
        lines
            .with_animation(
                ("voice-title", u64::from(details)),
                animation,
                move |this, delta| {
                    let shown = if details { delta } else { 1. - delta };
                    this.top(rpx(-LINE * shown))
                },
            )
            .into_any_element()
    } else {
        lines
            .top(rpx(if details { -LINE } else { 0. }))
            .into_any_element()
    };
    div()
        .h(rpx(LINE))
        .min_w_0()
        .overflow_hidden()
        .text_color(rgb(color))
        .child(lines)
        .into_any_element()
}

pub(crate) fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn remember_channel(channel: Option<SavedChannel>, cx: &mut App) {
    crate::settings::update(cx, |settings| settings.voice_channel = channel);
}

pub(crate) fn live_badge() -> Div {
    div()
        .flex_none()
        .px(rpx(6.))
        .rounded(rpx(8.))
        .bg(rgb(theme::RED))
        .text_size(rpx(11.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::BOLD)
        .text_color(white())
        .child("LIVE")
}

pub(crate) fn member_row(
    app: &DiscordApp,
    guild_id: u64,
    channel_id: u64,
    member: &VoiceMember,
    cx: &App,
) -> impl IntoElement + use<> {
    let user_id = member.user_id;
    let connected_elsewhere = user_id == app.store.current_user_id
        && app
            .voice
            .call
            .as_ref()
            .is_none_or(|call| call.guild_id != guild_id || call.channel_id != channel_id);
    let speaking = !connected_elsewhere && app.voice.speaking.contains(&user_id);
    let name = app
        .store
        .member_name(guild_id, user_id)
        .map(str::to_owned)
        .or_else(|| app.store.user(user_id).map(|user| user.name.clone()))
        .unwrap_or_default();
    let tag = app
        .store
        .user(user_id)
        .and_then(|user| user.tag.as_ref())
        .map(|tag| theme::guild_tag(tag, cx));
    let muted_color = |server: bool| if server { theme::RED } else { theme::muted() };
    let can_watch = user_id != app.store.current_user_id;
    let watch = app.entity.clone();
    let streamer = can_watch && member.streaming;
    let card = app.entity.clone();
    let row_bounds = Rc::new(Cell::new(Bounds::default()));
    let row_paint = row_bounds.clone();
    let row = div()
        .id(("voice-member", user_id))
        .when(streamer, |this| {
            this.on_prepaint(move |bounds, _, _| row_paint.set(bounds))
                .on_hover(move |hovered, window, cx| {
                    let position = stream_card::position(row_bounds.get(), window);
                    card.update(cx, |app, cx| {
                        app.hover_streamer((guild_id, channel_id, user_id), *hovered, position, cx)
                    })
                    .ok();
                })
        })
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(MEMBER_HEIGHT))
        .px_2()
        .rounded(rpx(6.))
        .when(connected_elsewhere, |this| this.opacity(0.5))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::hover())))
        .child(
            div()
                .relative()
                .flex_none()
                .size(rpx(MEMBER_AVATAR))
                .child(theme::undecorated_member_avatar(
                    &app.store,
                    guild_id,
                    user_id,
                    MEMBER_AVATAR,
                    cx,
                ))
                // Discord's speaking ring: 2px green over the avatar's edge, then a 1px gap
                // in the background colour separating it from the picture.
                .when(speaking, |this| {
                    this.child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded_full()
                            .border_2()
                            .border_color(rgb(theme::GREEN))
                            .child(
                                div()
                                    .size_full()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(theme::bg())),
                            ),
                    )
                }),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(if speaking {
                    theme::strong()
                } else {
                    theme::muted()
                }))
                .child(label(("voice-member-name", user_id), &name)),
        )
        .children(tag)
        .child(div().flex_1())
        .when(member.streaming, |this| {
            this.child(
                live_badge()
                    .id(("voice-member-live", user_id))
                    .when(can_watch, |this| {
                        this.cursor_pointer().on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            watch
                                .update(cx, |app, cx| {
                                    app.watch_or_join((guild_id, channel_id, user_id), window, cx)
                                })
                                .ok();
                        })
                    }),
            )
        })
        .when(member.video, |this| {
            this.child(theme::icon("video", 16., theme::muted()))
        })
        .when(
            member.mute || (member.self_mute && !member.self_deaf),
            |this| this.child(theme::icon("mic-off", 16., muted_color(member.mute))),
        )
        .when(member.deaf || member.self_deaf, |this| {
            this.child(theme::icon("headphone-off", 16., muted_color(member.deaf)))
        });
    // List items are laid out as roots, which ignore their own margins, so indent with padding.
    div()
        .w_full()
        .pl(rpx(MEMBER_INDENT))
        .pr_2()
        .py(rpx(1.))
        .child(row)
}

pub(crate) fn panel(app: &DiscordApp, cx: &App) -> Option<impl IntoElement + use<>> {
    let call = app.voice.call.as_ref()?;
    let guild = app
        .store
        .guilds
        .iter()
        .find(|guild| guild.id == call.guild_id);
    let channel = guild.and_then(|guild| guild.channel(call.channel_id));
    let place = match (channel, guild) {
        (Some(channel), Some(guild)) => format!("{} / {}", channel.name, guild.name),
        _ if call.guild_id == 0 => app.store.private_channel_name(call.channel_id),
        _ => String::new(),
    };
    let (status, color) = if call.connected {
        ("Voice Connected", theme::GREEN)
    } else {
        ("Connecting…", theme::YELLOW)
    };
    let details_open = app
        .voice
        .menu
        .as_ref()
        .is_some_and(|menu| menu.kind == MenuKind::Details && !menu.closing);
    let title = if call.connected {
        connected_title(
            app.voice.status_hovered || details_open,
            app.voice.status_hovered_once,
            color,
        )
    } else {
        div().child(status).into_any_element()
    };
    let ping: SharedString = match call.last_ping() {
        Some(ms) => format!("{ms} ms").into(),
        None if call.connected => "Measuring ping…".into(),
        None => "Connecting to voice…".into(),
    };
    let entity = app.entity.clone();
    let header = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .id("voice-signal")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(32.))
                .rounded(rpx(8.))
                .bg(rgba(color << 8 | 0x14))
                .child(theme::icon("rss", 18., color))
                .map(|this| with_tooltip(this, app, Control::Signal, ping)),
        )
        .child(
            div()
                .id("voice-status")
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .when(call.connected, |this| {
                    let hover = app.entity.clone();
                    let open = app.entity.clone();
                    this.cursor_pointer()
                        .on_hover(move |hovered, _, cx| {
                            let hovered = *hovered;
                            hover
                                .update(cx, |app, cx| {
                                    app.voice.status_hovered = hovered;
                                    app.voice.status_hovered_once = true;
                                    app.views.channel_list.update(cx, |_, cx| cx.notify());
                                })
                                .ok();
                        })
                        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                            cx.stop_propagation();
                            let position = event.position;
                            open.update(cx, |app, cx| {
                                app.toggle_menu(MenuKind::Details, position, cx)
                            })
                            .ok();
                        })
                })
                .child(
                    div()
                        .mt(rpx(-1.))
                        .truncate()
                        .text_size(rpx(14.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(color))
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::muted()))
                        .child(place),
                ),
        )
        .child({
            let entity = app.entity.clone();
            let color = if app.voice.noise_suppression {
                theme::text()
            } else {
                theme::faint()
            };
            let menu_open = app
                .voice
                .menu
                .as_ref()
                .is_some_and(|menu| menu.kind == MenuKind::NoiseSuppression && !menu.closing);
            theme::icon_button("voice-noise", "audio-lines", 20., color)
                .when(!menu_open, |this| {
                    with_tooltip(
                        this,
                        app,
                        Control::NoiseSuppression,
                        "Noise Suppression powered by RNNoise",
                    )
                })
                .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                    cx.stop_propagation();
                    let position = event.position;
                    entity
                        .update(cx, |app, cx| {
                            app.toggle_menu(MenuKind::NoiseSuppression, position, cx)
                        })
                        .ok();
                })
        })
        .child(
            div()
                .id("voice-disconnect")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(28.))
                .rounded(rpx(4.))
                .cursor_pointer()
                .hover(|style| style.bg(rgba(theme::selected())))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |app, cx| app.leave_voice(cx)).ok();
                })
                .map(|this| with_tooltip(this, app, Control::Disconnect, "Disconnect"))
                // Lucide's handset tilted down into Discord's hang-up glyph.
                .child(
                    theme::icon("phone", 20., theme::text()).with_transformation(
                        Transformation::rotate(radians(std::f32::consts::FRAC_PI_4 * 3.)),
                    ),
                ),
        );
    let action = |id: &'static str, icon: &'static str| {
        div()
            .id(id)
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .h(rpx(32.))
            .rounded(rpx(8.))
            .bg(rgba(theme::hover()))
            .child(theme::icon(icon, 20., theme::text()))
    };
    let sharing = app.voice.sharing != Sharing::Off;
    let share = app.entity.clone();
    let stream_menu_open = app
        .voice
        .menu
        .as_ref()
        .is_some_and(|menu| menu.kind == MenuKind::Stream && !menu.closing);
    let screen = action("voice-screen", "monitor-up")
        .cursor_pointer()
        .when(sharing, |this| this.bg(rgb(theme::GREEN)))
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            cx.stop_propagation();
            let position = event.position;
            share
                .update(cx, |app, cx| {
                    if app.voice.sharing == Sharing::Off {
                        app.screen_button(cx);
                    } else {
                        app.toggle_menu(MenuKind::Stream, position, cx);
                    }
                })
                .ok();
        })
        .when(!stream_menu_open, |this| {
            with_tooltip(
                this,
                app,
                Control::Screen,
                if sharing {
                    "Stream Settings"
                } else {
                    "Share Your Screen"
                },
            )
        });
    let actions = div()
        .flex()
        .gap_2()
        .child(unavailable(action("voice-camera", "video-off"), "Camera"))
        .child(screen)
        .child(unavailable(
            action("voice-activities", "shapes"),
            "Activities",
        ))
        .child(unavailable(
            action("voice-soundboard", "megaphone"),
            "Soundboard",
        ));
    let stream = sharing
        .then(|| share_panel::render(app, call.guild_id, cx))
        .flatten();
    Some(
        div()
            .flex()
            .flex_col()
            .bg(rgb(theme::bg()))
            .children(stream)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p(rpx(8.))
                    .child(header)
                    .child(actions),
            ),
    )
}

fn with_tooltip(
    element: Stateful<Div>,
    app: &DiscordApp,
    control: Control,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    let entity = app.entity.clone();
    element
        .relative()
        .on_hover(move |hovered, _, cx| {
            let hovered = *hovered;
            entity
                .update(cx, |app, cx| {
                    let voice = &mut app.voice;
                    if hovered {
                        voice.hovered_control = Some(control);
                    } else if voice.hovered_control == Some(control) {
                        voice.hovered_control = None;
                    }
                    app.views.channel_list.update(cx, |_, cx| cx.notify());
                })
                .ok();
        })
        .when(app.voice.hovered_control == Some(control), |this| {
            this.child(theme::tooltip_above(label))
        })
}

/// Discord's voice controls that zcord doesn't implement yet: shown for familiarity, with a
/// tooltip saying so instead of doing nothing silently.
pub(crate) fn unavailable(button: Stateful<Div>, feature: &'static str) -> Stateful<Div> {
    button.relative().child(theme::hover_tooltip(
        SharedString::from(format!("unavailable-{feature}")),
        format!("{feature} isn't available in zcord yet"),
    ))
}

/// Noise suppression popover above the voice panel's waveform button. Rendered from the
/// overlay view because deferred popups can't live inside the cached channel list view.
/// The voice panel's popups. Rendered from the overlay view because deferred popups can't
/// live inside the cached channel list view.
pub(crate) fn menu(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let menu = app.voice.menu.as_ref()?;
    let call = app.voice.call.as_ref();
    let card = match menu.kind {
        MenuKind::NoiseSuppression => noise_card(app, menu, cx),
        MenuKind::Details => details::card(app, call?, app.voice.details_tab, menu.closing, cx),
        MenuKind::Stream => share_panel::live_menu(app, menu),
        MenuKind::InputOptions => device_menu::card(app, menu, device_menu::DeviceKind::Input, cx),
        MenuKind::OutputOptions => {
            device_menu::card(app, menu, device_menu::DeviceKind::Output, cx)
        }
        MenuKind::CallAudio => device_menu::call_card(app, menu, cx),
    };
    let close = app.entity.clone();
    let card = card.when(!menu.closing, |this| {
        this.occlude().on_mouse_down_out(move |_, _, cx| {
            close
                .update(cx, |app, cx| {
                    let in_flyout = app
                        .voice
                        .menu
                        .as_ref()
                        .is_some_and(|menu| menu.pointer_in_flyout);
                    if !in_flyout {
                        app.close_menu(cx);
                    }
                })
                .ok();
        })
    });
    Some(popup(card, menu, cx))
}

fn noise_card(app: &DiscordApp, menu: &Menu, cx: &App) -> Stateful<Div> {
    let closing = menu.closing;
    let reduce_motion = cx.reduce_motion();
    let enabled = app.voice.noise_suppression;
    let toggle = app.entity.clone();
    let slide = (menu.switch_moved && !reduce_motion).then_some(menu.animation);
    let title = div()
        .id("noise-suppression-toggle")
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .cursor_pointer()
        .when(!closing, |this| {
            this.on_click(move |_, _, cx| {
                toggle
                    .update(cx, |app, cx| app.toggle_noise_suppression(cx))
                    .ok();
            })
        })
        .child(
            div()
                .text_size(rpx(16.))
                .line_height(rpx(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::strong()))
                .child("Noise Suppression"),
        )
        .child(switch("noise-switch", enabled, slide));
    menu_frame("noise-menu")
        .gap_2()
        .w(rpx(296.))
        .p(rpx(16.))
        .child(title)
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::text()))
                .child(
                    "Filters steady background noise like fans, hum and typing out of your \
                     microphone before it's sent. Sudden sounds, such as clapping, may still \
                     get through.",
                ),
        )
        .child(
            div()
                .pt(rpx(4.))
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .text_color(rgb(theme::muted()))
                .child("Powered by RNNoise"),
        )
}

fn menu_frame(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .rounded(rpx(8.))
        .bg(rgb(theme::elevated()))
        .border_1()
        .border_color(rgba(theme::border()))
        .shadow_lg()
}

fn popup(card: Stateful<Div>, menu: &Menu, cx: &App) -> AnyElement {
    let closing = menu.closing;
    let card = if cx.reduce_motion() {
        card.into_any_element()
    } else {
        let key = if closing {
            "voice-menu-out"
        } else {
            "voice-menu-in"
        };
        let animation = Animation::new(if closing { MENU_EXIT } else { MENU_ENTER })
            .with_easing(ease_out_quint());
        card.with_animation((key, menu.animation), animation, move |this, delta| {
            let progress = if closing { 1. - delta } else { delta };
            this.relative()
                .top(rpx((1. - progress) * MENU_RISE))
                .opacity(progress)
        })
        .into_any_element()
    };
    deferred(
        anchored()
            .position(menu.position - point(px(0.), px(16.)))
            .anchor(Anchor::BottomCenter)
            .snap_to_window_with_margin(px(8.))
            .child(card),
    )
    .with_priority(2)
    .into_any_element()
}

/// Discord's settings toggle. `slide` carries the menu's animation id once the user has
/// flipped it, so the knob and track ease into the new state instead of jumping.
pub(crate) fn switch(id: &'static str, on: bool, slide: Option<u64>) -> AnyElement {
    const OFF_LEFT: f32 = 2.;
    const ON_LEFT: f32 = 18.;
    let (icon, icon_color) = if on {
        ("check", theme::BLURPLE)
    } else {
        ("x", theme::faint())
    };
    let at = move |t: f32| if on { t } else { 1. - t };
    let knob = div()
        .absolute()
        .top(rpx(2.))
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(20.))
        .rounded_full()
        .bg(rgb(theme::control_thumb()))
        .child(theme::icon(icon, 12., icon_color));
    let track = div()
        .relative()
        .flex_none()
        .w(rpx(40.))
        .h(rpx(24.))
        .rounded_full();
    let Some(slide) = slide else {
        return track
            .bg(blend(theme::faint(), theme::BLURPLE, at(1.)))
            .child(knob.left(rpx(OFF_LEFT + (ON_LEFT - OFF_LEFT) * at(1.))))
            .into_any_element();
    };
    let animation = Animation::new(SWITCH_SLIDE).with_easing(ease_out_quint());
    let key = slide << 1 | u64::from(on);
    track
        .child(knob.with_animation(
            ElementId::NamedInteger(format!("{id}-knob").into(), key),
            animation.clone(),
            move |this, delta| this.left(rpx(OFF_LEFT + (ON_LEFT - OFF_LEFT) * at(delta))),
        ))
        .with_animation(
            ElementId::NamedInteger(format!("{id}-track").into(), key),
            animation,
            move |this, delta| this.bg(blend(theme::faint(), theme::BLURPLE, at(delta))),
        )
        .into_any_element()
}

fn blend(from: u32, to: u32, t: f32) -> Rgba {
    let (from, to) = (rgb(from), rgb(to));
    Rgba {
        r: from.r + (to.r - from.r) * t,
        g: from.g + (to.g - from.g) * t,
        b: from.b + (to.b - from.b) * t,
        a: 1.,
    }
}

#[cfg(test)]
mod tests {
    use super::{Call, Duration, Store, Voice};
    use crate::discord::{Event, model};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn store_with_start(start: Option<u64>) -> Store {
        let ready =
            serde_json::from_str(include_str!("../../crates/discord/src/fixtures/ready.json"))
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
