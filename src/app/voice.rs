use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use gpui::{prelude::*, *};
use gpui_component::tooltip::Tooltip;

use crate::{
    app::{DiscordApp, message_body::label},
    discord::VoiceEvent,
    settings::SavedChannel,
    store::VoiceMember,
    theme::{self, rpx},
};

pub(crate) const MEMBER_HEIGHT: f32 = 32.;
const MEMBER_AVATAR: f32 = 24.;
const MEMBER_INDENT: f32 = 36.;

#[derive(Default)]
pub(crate) struct Voice {
    pub(crate) self_mute: bool,
    pub(crate) self_deaf: bool,
    pub(crate) call: Option<Call>,
    pub(crate) speaking: HashSet<u64>,
    restored: bool,
    clock: Option<Task<()>>,
}

#[derive(Clone, Copy)]
pub(crate) struct Joined {
    pub(crate) channel_id: u64,
    pub(crate) elapsed: Option<Duration>,
}

impl Voice {
    pub(crate) fn joined(&self) -> Option<Joined> {
        self.call.as_ref().map(|call| Joined {
            channel_id: call.channel_id,
            elapsed: call.connected_at.map(|at| at.elapsed()),
        })
    }

    pub(crate) fn from_settings(cx: &App) -> Self {
        let settings = crate::settings::get(cx);
        Self {
            self_mute: settings.voice_mute,
            self_deaf: settings.voice_deaf,
            ..Self::default()
        }
    }
}

pub(crate) struct Call {
    pub(crate) guild_id: u64,
    pub(crate) channel_id: u64,
    pub(crate) connected: bool,
    pub(crate) connected_at: Option<Instant>,
    pub(crate) privacy_code: Option<String>,
}

impl DiscordApp {
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
        session.set_self_mute(self.voice.self_mute);
        session.set_self_deaf(self.voice.self_deaf);
        session.join_voice(guild_id, channel_id);
        self.voice.call = Some(Call {
            guild_id,
            channel_id,
            connected: false,
            connected_at: None,
            privacy_code: None,
        });
        self.voice.speaking.clear();
        self.sync_channel_rows();
        cx.notify();
    }

    pub(crate) fn leave_voice(&mut self, cx: &mut Context<Self>) {
        if self.voice.call.take().is_none() {
            return;
        }
        if let Some(session) = &self.session {
            session.leave_voice();
        }
        remember_channel(None, cx);
        self.voice.speaking.clear();
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
            self.join_voice(saved.guild_id, saved.channel_id, cx);
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
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    pub(crate) fn handle_voice_event(&mut self, event: VoiceEvent, cx: &mut Context<Self>) {
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
                voice.call = Some(Call {
                    guild_id,
                    channel_id,
                    connected: false,
                    connected_at: previous.as_ref().and_then(|call| call.connected_at),
                    privacy_code: previous.and_then(|call| call.privacy_code),
                });
            }
            VoiceEvent::Connected {
                guild_id,
                channel_id,
            } => {
                match &mut voice.call {
                    Some(call) if call.channel_id == channel_id => {
                        call.connected = true;
                        call.connected_at.get_or_insert_with(Instant::now);
                    }
                    _ => {
                        voice.call = Some(Call {
                            guild_id,
                            channel_id,
                            connected: true,
                            connected_at: Some(Instant::now()),
                            privacy_code: None,
                        })
                    }
                }
                self.start_call_clock(cx);
                remember_channel(
                    Some(SavedChannel {
                        guild_id,
                        channel_id,
                    }),
                    cx,
                );
            }
            VoiceEvent::Disconnected { reason } => {
                if let Some(reason) = reason {
                    log::warn!("Voice disconnected: {reason}");
                }
                voice.call = None;
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
                // Speaking flips several times a second; only the channel list shows it.
                if changed {
                    self.views.channel_list.update(cx, |_, cx| cx.notify());
                }
                return;
            }
            VoiceEvent::PrivacyCode(code) => {
                if let Some(call) = &mut voice.call {
                    call.privacy_code = code;
                }
                cx.notify();
                return;
            }
        }
        self.sync_channel_rows();
        cx.notify();
    }
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

pub(crate) fn member_row(
    app: &DiscordApp,
    guild_id: u64,
    member: &VoiceMember,
    cx: &App,
) -> impl IntoElement + use<> {
    let user_id = member.user_id;
    let speaking = app.voice.speaking.contains(&user_id);
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
    let muted_color = |server: bool| if server { theme::RED } else { theme::MUTED };
    let row = div()
        .id(("voice-member", user_id))
        .flex()
        .items_center()
        .gap_2()
        .h(rpx(MEMBER_HEIGHT))
        .px_2()
        .rounded(rpx(6.))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(theme::HOVER)))
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
                                    .border_color(rgb(theme::BG)),
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
                    theme::STRONG
                } else {
                    theme::MUTED
                }))
                .child(label(("voice-member-name", user_id), &name)),
        )
        .children(tag)
        .child(div().flex_1())
        .when(member.streaming, |this| {
            this.child(
                div()
                    .flex_none()
                    .px(rpx(6.))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::RED))
                    .text_size(rpx(11.))
                    .line_height(rpx(16.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(white())
                    .child("LIVE"),
            )
        })
        .when(member.video, |this| {
            this.child(theme::icon("video", 16., theme::MUTED))
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

pub(crate) fn panel(app: &DiscordApp) -> Option<impl IntoElement + use<>> {
    let call = app.voice.call.as_ref()?;
    let guild = app
        .store
        .guilds
        .iter()
        .find(|guild| guild.id == call.guild_id);
    let channel = guild.and_then(|guild| guild.channel(call.channel_id));
    let place = match (channel, guild) {
        (Some(channel), Some(guild)) => format!("{} / {}", channel.name, guild.name),
        _ => String::new(),
    };
    let (status, color) = if call.connected {
        ("Voice Connected", theme::GREEN)
    } else {
        ("Connecting…", theme::YELLOW)
    };
    let encryption: SharedString = match &call.privacy_code {
        Some(code) => format!("End-to-end encrypted\nPrivacy code: {code}").into(),
        None if call.connected => "Not end-to-end encrypted".into(),
        None => "Connecting to voice…".into(),
    };
    let entity = app.entity.clone();
    let header = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(rpx(36.))
                .rounded(rpx(8.))
                .bg(rgba(color << 8 | 0x0C))
                .child(theme::icon("rss", 20., color)),
        )
        .child(
            div()
                .id("voice-status")
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .tooltip(move |window, cx| Tooltip::new(encryption.clone()).build(window, cx))
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(color))
                        .child(status),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(13.))
                        .line_height(rpx(16.))
                        .text_color(rgb(theme::MUTED))
                        .child(place),
                ),
        )
        .child(unavailable(
            theme::icon_button("voice-noise", "audio-lines", 20., theme::TEXT),
            "Noise suppression",
        ))
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
                .hover(|style| style.bg(rgb(theme::SELECTED)))
                .tooltip(|window, cx| Tooltip::new("Disconnect").build(window, cx))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |app, cx| app.leave_voice(cx)).ok();
                })
                // Lucide's handset tilted down into Discord's hang-up glyph.
                .child(theme::icon("phone", 20., theme::TEXT).with_transformation(
                    Transformation::rotate(radians(std::f32::consts::FRAC_PI_4 * 3.)),
                )),
        );
    let actions = div().flex().gap_2().children(
        [
            ("voice-camera", "video-off", "Camera"),
            ("voice-screen", "monitor-up", "Screen share"),
            ("voice-activities", "shapes", "Activities"),
            ("voice-soundboard", "megaphone", "Soundboard"),
        ]
        .map(|(id, icon, feature)| {
            unavailable(
                div()
                    .id(id)
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .h(rpx(32.))
                    .rounded(rpx(8.))
                    .bg(rgb(theme::ELEVATED))
                    .child(theme::icon(icon, 20., theme::TEXT)),
                feature,
            )
        }),
    );
    Some(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p(rpx(8.))
            .bg(black())
            .child(header)
            .child(actions),
    )
}

/// Discord's voice controls that zcord doesn't implement yet: shown for familiarity, with a
/// tooltip saying so instead of doing nothing silently.
fn unavailable(button: Stateful<Div>, feature: &'static str) -> Stateful<Div> {
    button.tooltip(move |window, cx| {
        Tooltip::new(format!("{feature} isn't available in zcord yet")).build(window, cx)
    })
}
