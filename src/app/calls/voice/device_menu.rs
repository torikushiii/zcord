use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_component::slider::{Slider, SliderEvent, SliderState};

use super::{Menu, MenuKind};
use crate::{
    app::{
        DiscordApp,
        menu::{checkbox, divider, panel, radio, row},
    },
    discord::{AudioDeviceKind, AudioDevices, Session, audio_devices},
    settings::InputProfile,
    theme::{self, rpx},
};

const MENU_WIDTH: f32 = 240.;
const DEVICE_FLYOUT_WIDTH: f32 = 312.;
const PROFILE_FLYOUT_WIDTH: f32 = 212.;
const FLYOUT_OFFSET: f32 = 16.;
const LEVEL_SEGMENTS: usize = 28;
const LEVEL_TICK: Duration = Duration::from_millis(50);
pub(crate) const DEFAULT_DEVICE: &str = if cfg!(windows) {
    "Windows Default"
} else {
    "System Default"
};
const PROFILES: [InputProfile; 3] = [
    InputProfile::VoiceIsolation,
    InputProfile::Studio,
    InputProfile::Custom,
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceKind {
    Input,
    Output,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flyout {
    Device,
    Profile,
    OutputDevice,
}

pub(super) struct VolumeSlider {
    state: Entity<SliderState>,
    _changes: Subscription,
}

pub(crate) struct LevelBars {
    session: Option<Session>,
    shown: (usize, bool),
    _tick: Task<()>,
}

impl LevelBars {
    fn new(session: Option<Session>, cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(LEVEL_TICK).await;
                let polled = this.update(cx, |bars, cx| {
                    let reading = bars.reading();
                    if reading != bars.shown {
                        bars.shown = reading;
                        cx.notify();
                    }
                });
                if polled.is_err() {
                    break;
                }
            }
        });
        let mut bars = Self {
            session,
            shown: (0, false),
            _tick: tick,
        };
        bars.shown = bars.reading();
        bars
    }

    fn reading(&self) -> (usize, bool) {
        let (level, open) = self.session.as_ref().map_or((None, false), |session| {
            (session.input_level(), session.input_gate_open())
        });
        let fill = level.map_or(0., |db| ((db + 100.) / 100.).clamp(0., 1.));
        ((fill * LEVEL_SEGMENTS as f32).round() as usize, open)
    }
}

impl Render for LevelBars {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (lit, open) = self.shown;
        div()
            .flex()
            .justify_between()
            .h(rpx(18.))
            .children((0..LEVEL_SEGMENTS).map(|index| {
                div().w(rpx(4.)).h_full().rounded_full().bg(if index < lit {
                    rgb(if open { theme::GREEN } else { theme::YELLOW })
                } else {
                    rgba(theme::selected())
                })
            }))
    }
}

pub(crate) struct DeviceChoice {
    pub(crate) name: Option<String>,
    pub(crate) label: String,
    pub(crate) detail: Option<String>,
    pub(crate) icon: &'static str,
}

fn icon(kind: Option<AudioDeviceKind>, direction: DeviceKind) -> &'static str {
    match (kind, direction) {
        (Some(AudioDeviceKind::Bluetooth), _) => "bluetooth",
        (Some(AudioDeviceKind::Display), _) => "monitor-filled",
        (Some(AudioDeviceKind::Headphones), _) => "headphones",
        (Some(AudioDeviceKind::Microphone), _) => "mic",
        (Some(AudioDeviceKind::Speaker), _) => "volume-2",
        (_, DeviceKind::Input) => "mic",
        (_, DeviceKind::Output) => "headphones",
    }
}

pub(crate) fn choices(devices: Option<&AudioDevices>, kind: DeviceKind) -> Vec<DeviceChoice> {
    let (list, default) = match (devices, kind) {
        (Some(devices), DeviceKind::Input) => (&devices.inputs[..], devices.default_input.as_ref()),
        (Some(devices), DeviceKind::Output) => {
            (&devices.outputs[..], devices.default_output.as_ref())
        }
        (None, _) => (&[][..], None),
    };
    std::iter::once(DeviceChoice {
        name: None,
        label: DEFAULT_DEVICE.to_owned(),
        detail: default.map(|device| device.label.clone()),
        icon: icon(default.map(|device| device.kind), kind),
    })
    .chain(list.iter().map(|device| DeviceChoice {
        name: Some(device.name.clone()),
        label: device.label.clone(),
        detail: device.detail.clone(),
        icon: icon(Some(device.kind), kind),
    }))
    .collect()
}

pub(crate) fn current(
    devices: Option<&AudioDevices>,
    kind: DeviceKind,
    saved: Option<&str>,
) -> DeviceChoice {
    let mut choices = choices(devices, kind);
    let found = choices
        .iter()
        .position(|choice| choice.name.as_deref() == saved);
    match (found, saved) {
        (Some(index), _) => choices.swap_remove(index),
        (None, Some(saved)) => DeviceChoice {
            name: Some(saved.to_owned()),
            label: saved.to_owned(),
            detail: None,
            icon: icon(None, kind),
        },
        (None, None) => choices.swap_remove(0),
    }
}

fn saved_device(kind: DeviceKind, cx: &App) -> Option<String> {
    let settings = crate::settings::get(cx);
    match kind {
        DeviceKind::Input => settings.voice_input_device,
        DeviceKind::Output => settings.voice_output_device,
    }
}

fn profile_label(profile: InputProfile) -> &'static str {
    match profile {
        InputProfile::VoiceIsolation => "Voice Isolation",
        InputProfile::Studio => "Studio",
        InputProfile::Custom => "Custom",
    }
}

impl DiscordApp {
    pub(crate) fn refresh_audio_devices(&mut self, cx: &mut Context<Self>) {
        self.voice.devices_loading = Some(cx.spawn(async move |this, cx| {
            let devices = cx
                .background_executor()
                .spawn(async { audio_devices() })
                .await;
            this.update(cx, |app, cx| {
                app.voice.devices = Some(devices);
                cx.notify();
            })
            .ok();
        }));
    }

    pub(crate) fn device_menu_open(&self, kind: DeviceKind) -> bool {
        let menu = match kind {
            DeviceKind::Input => MenuKind::InputOptions,
            DeviceKind::Output => MenuKind::OutputOptions,
        };
        self.voice
            .menu
            .as_ref()
            .is_some_and(|open| open.kind == menu && !open.closing)
    }

    pub(crate) fn audio_devices(&self) -> Option<&AudioDevices> {
        self.voice.devices.as_ref()
    }

    pub(crate) fn toggle_device_menu(
        &mut self,
        kind: DeviceKind,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let menu = match kind {
            DeviceKind::Input => MenuKind::InputOptions,
            DeviceKind::Output => MenuKind::OutputOptions,
        };
        let opening = self
            .voice
            .menu
            .as_ref()
            .is_none_or(|open| open.kind != menu || open.closing);
        if opening {
            self.refresh_audio_devices(cx);
            self.voice.volume_slider = Some(volume_slider(kind, cx));
        }
        self.toggle_menu(menu, position, cx);
    }

    pub(crate) fn toggle_call_audio_menu(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let opening = self
            .voice
            .menu
            .as_ref()
            .is_none_or(|open| open.kind != MenuKind::CallAudio || open.closing);
        if opening {
            self.refresh_audio_devices(cx);
            self.voice.volume_slider = Some(volume_slider(DeviceKind::Input, cx));
            self.voice.output_slider = Some(volume_slider(DeviceKind::Output, cx));
        }
        self.toggle_menu(MenuKind::CallAudio, position, cx);
        let session = self.session.clone();
        if let Some(menu) = self
            .voice
            .menu
            .as_mut()
            .filter(|menu| menu.kind == MenuKind::CallAudio && menu.level.is_none())
        {
            menu.level = Some(cx.new(|cx| LevelBars::new(session, cx)));
        }
    }

    pub(crate) fn call_audio_menu_open(&self) -> bool {
        self.voice
            .menu
            .as_ref()
            .is_some_and(|open| open.kind == MenuKind::CallAudio && !open.closing)
    }

    fn show_device_flyout(&mut self, flyout: Option<Flyout>, cx: &mut Context<Self>) {
        let Some(menu) = self
            .voice
            .menu
            .as_mut()
            .filter(|menu| menu.device_flyout != flyout)
        else {
            return;
        };
        menu.device_flyout = flyout;
        menu.pointer_in_flyout = false;
        cx.notify();
    }

    fn pick_device(&mut self, kind: DeviceKind, name: Option<String>, cx: &mut Context<Self>) {
        self.select_device(kind, name, cx);
        self.close_menu(cx);
    }

    pub(crate) fn input_profile(&self, cx: &App) -> InputProfile {
        crate::settings::get(cx)
            .voice_input_profile
            .unwrap_or(if self.voice.noise_suppression {
                InputProfile::VoiceIsolation
            } else {
                InputProfile::Custom
            })
    }

    fn set_input_profile(&mut self, profile: InputProfile, cx: &mut Context<Self>) {
        match profile {
            InputProfile::VoiceIsolation => self.set_noise_suppression(true, cx),
            InputProfile::Studio => self.set_noise_suppression(false, cx),
            InputProfile::Custom => {}
        }
        crate::settings::update(cx, |settings| settings.voice_input_profile = Some(profile));
        cx.notify();
    }

    fn open_voice_settings_from_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_menu(cx);
        self.open_voice_settings(window, cx);
    }
}

fn volume_slider(kind: DeviceKind, cx: &mut Context<DiscordApp>) -> VolumeSlider {
    let saved = crate::settings::get(cx);
    let gain = match kind {
        DeviceKind::Input => saved.voice_input_volume,
        DeviceKind::Output => saved.voice_output_volume,
    };
    let state = cx.new(|_| {
        SliderState::new()
            .min(0.)
            .max(200.)
            .step(1.)
            .default_value(gain.unwrap_or(1.) * 100.)
    });
    let changes = cx.subscribe(&state, move |app, _, event: &SliderEvent, cx| {
        if let SliderEvent::Change(value) = event {
            let gain = value.start() / 100.;
            match kind {
                DeviceKind::Input => app.set_input_volume(gain, cx),
                DeviceKind::Output => app.set_output_volume(gain, cx),
            }
        }
    });
    VolumeSlider {
        state,
        _changes: changes,
    }
}

fn closes_flyout_row(app: &DiscordApp, key: &'static str) -> Stateful<Div> {
    let entity = app.entity.clone();
    row(key, true).on_hover(move |hovered, _, cx| {
        if *hovered {
            entity
                .update(cx, |app, cx| app.show_device_flyout(None, cx))
                .ok();
        }
    })
}

fn title_block(title: &'static str, subtitle: Option<String>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(div().truncate().child(title))
        .children(subtitle.map(|subtitle| {
            div()
                .truncate()
                .text_size(rpx(12.))
                .line_height(rpx(16.))
                .font_weight(FontWeight::NORMAL)
                .text_color(rgb(theme::muted()))
                .child(subtitle)
        }))
}

fn flyout_row(
    app: &DiscordApp,
    menu: &Menu,
    key: &'static str,
    title: &'static str,
    subtitle: String,
    flyout: Flyout,
    content: impl FnOnce() -> Stateful<Div>,
) -> Stateful<Div> {
    let open = menu.device_flyout == Some(flyout);
    let hover = app.entity.clone();
    let inside = app.entity.clone();
    row(key, true)
        .when(open, |this| this.bg(rgba(theme::selected())))
        .on_hover(move |hovered, _, cx| {
            if *hovered {
                hover
                    .update(cx, |app, cx| app.show_device_flyout(Some(flyout), cx))
                    .ok();
            }
        })
        .child(title_block(title, Some(subtitle)))
        .child(theme::icon("chevron-right", 20., theme::muted()))
        .when(open, |this| {
            this.child(
                div().absolute().top(rpx(-9.)).left(relative(1.)).child(
                    anchored().snap_to_window_with_margin(px(8.)).child(
                        // A margin, since the anchored list ignores the wrapper's padding. The hover goes on the
                        // panel itself: it occludes, so a wrapper would never see it.
                        content()
                            .ml(rpx(FLYOUT_OFFSET))
                            .on_hover(move |hovered, _, cx| {
                                inside
                                    .update(cx, |app, _| {
                                        if let Some(menu) = app.voice.menu.as_mut() {
                                            menu.pointer_in_flyout = *hovered;
                                        }
                                    })
                                    .ok();
                            }),
                    ),
                ),
            )
        })
}

pub(crate) fn device_line(choice: &DeviceChoice, bold: bool) -> Div {
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(rpx(12.))
        .child(theme::icon(choice.icon, 20., theme::muted()))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .map(|this| {
                            if bold {
                                this.font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(theme::strong()))
                            } else {
                                this.font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(theme::muted()))
                            }
                        })
                        .child(choice.label.clone()),
                )
                .children(choice.detail.clone().map(|detail| {
                    div()
                        .truncate()
                        .text_size(rpx(12.))
                        .line_height(rpx(16.))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(rgb(theme::muted()))
                        .child(detail)
                })),
        )
}

fn device_flyout(app: &DiscordApp, kind: DeviceKind, cx: &App) -> Stateful<Div> {
    let saved = saved_device(kind, cx);
    panel("device-menu-devices", DEVICE_FLYOUT_WIDTH).children(
        choices(app.audio_devices(), kind)
            .into_iter()
            .enumerate()
            .map(|(index, choice)| {
                let selected = choice.name == saved;
                let entity = app.entity.clone();
                let line = device_line(&choice, true);
                let name = choice.name;
                row(("device-menu-device", index as u64), true)
                    .gap(rpx(12.))
                    .on_click(move |_, _, cx| {
                        let name = name.clone();
                        entity
                            .update(cx, |app, cx| app.pick_device(kind, name, cx))
                            .ok();
                    })
                    .child(line)
                    .child(radio(selected))
            }),
    )
}

fn profile_flyout(app: &DiscordApp, cx: &App) -> Stateful<Div> {
    let current = app.input_profile(cx);
    panel("device-menu-profiles", PROFILE_FLYOUT_WIDTH).children(PROFILES.into_iter().map(
        |profile| {
            let entity = app.entity.clone();
            row(("device-menu-profile", profile as u64), true)
                .on_click(move |_, _, cx| {
                    entity
                        .update(cx, |app, cx| app.set_input_profile(profile, cx))
                        .ok();
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(profile_label(profile)),
                )
                .child(radio(profile == current))
        },
    ))
}

pub(super) fn card(app: &DiscordApp, menu: &Menu, kind: DeviceKind, cx: &App) -> Stateful<Div> {
    let (device_title, volume_title) = match kind {
        DeviceKind::Input => ("Input Device", "Input Volume"),
        DeviceKind::Output => ("Output Device", "Output Volume"),
    };
    let saved = saved_device(kind, cx);
    let device = current(app.audio_devices(), kind, saved.as_deref());
    panel("device-menu", MENU_WIDTH)
        .child(flyout_row(
            app,
            menu,
            "device-menu-device-row",
            device_title,
            device.label,
            Flyout::Device,
            || device_flyout(app, kind, cx),
        ))
        .when(kind == DeviceKind::Input, |this| {
            this.child(flyout_row(
                app,
                menu,
                "device-menu-profile-row",
                "Input Profile",
                profile_label(app.input_profile(cx)).to_owned(),
                Flyout::Profile,
                || profile_flyout(app, cx),
            ))
        })
        .child(divider())
        .child(
            volume_block(app, "device-menu-volume", volume_title)
                .children(app.voice.volume_slider.as_ref().map(slider)),
        )
        .child(divider())
        .child(settings_row(app))
}

fn volume_block(app: &DiscordApp, id: &'static str, title: &'static str) -> Stateful<Div> {
    let entity = app.entity.clone();
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .p(rpx(8.))
        .on_hover(move |hovered, _, cx| {
            if *hovered {
                entity
                    .update(cx, |app, cx| app.show_device_flyout(None, cx))
                    .ok();
            }
        })
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::text()))
                .child(title),
        )
}

fn slider(slider: &VolumeSlider) -> Div {
    div().h(rpx(20.)).child(
        Slider::new(&slider.state)
            .bg(rgb(theme::BLURPLE))
            .text_color(white()),
    )
}

fn settings_row(app: &DiscordApp) -> Stateful<Div> {
    let settings = app.entity.clone();
    closes_flyout_row(app, "device-menu-settings")
        .on_click(move |_, window, cx| {
            settings
                .update(cx, |app, cx| app.open_voice_settings_from_menu(window, cx))
                .ok();
        })
        .child(theme::icon("settings", 20., theme::muted()))
        .child(div().flex_1().min_w_0().truncate().child("Voice Settings"))
}

pub(super) fn call_card(app: &DiscordApp, menu: &Menu, cx: &App) -> Stateful<Div> {
    let saved = |kind| saved_device(kind, cx);
    let input = current(
        app.audio_devices(),
        DeviceKind::Input,
        saved(DeviceKind::Input).as_deref(),
    );
    let output = current(
        app.audio_devices(),
        DeviceKind::Output,
        saved(DeviceKind::Output).as_deref(),
    );
    let deafen = app.entity.clone();
    panel("call-audio-menu", MENU_WIDTH)
        .child(flyout_row(
            app,
            menu,
            "call-audio-input-row",
            "Input Device",
            input.label,
            Flyout::Device,
            || device_flyout(app, DeviceKind::Input, cx),
        ))
        .child(flyout_row(
            app,
            menu,
            "call-audio-profile-row",
            "Input Profile",
            profile_label(app.input_profile(cx)).to_owned(),
            Flyout::Profile,
            || profile_flyout(app, cx),
        ))
        .child(flyout_row(
            app,
            menu,
            "call-audio-output-row",
            "Output Device",
            output.label,
            Flyout::OutputDevice,
            || device_flyout(app, DeviceKind::Output, cx),
        ))
        .child(divider())
        .child(
            volume_block(app, "call-audio-input-volume", "Input Volume")
                .children(app.voice.volume_slider.as_ref().map(slider)),
        )
        .child(volume_block(app, "call-audio-level", "Input Level").children(menu.level.clone()))
        .child(
            volume_block(app, "call-audio-output-volume", "Output Volume")
                .children(app.voice.output_slider.as_ref().map(slider)),
        )
        .child(divider())
        .child(
            closes_flyout_row(app, "call-audio-deafen")
                .on_click(move |_, _, cx| {
                    deafen.update(cx, |app, cx| app.toggle_self_deaf(cx)).ok();
                })
                .child(div().flex_1().min_w_0().truncate().child("Deafen"))
                .child(checkbox(app.voice.self_deaf)),
        )
        .child(settings_row(app))
}
