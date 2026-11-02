use gpui_component::scroll::ScrollableElement as _;
use std::{cell::Cell, rc::Rc, time::Duration};

use gpui::{prelude::*, *};
use gpui_component::{
    ElementExt,
    slider::{Slider, SliderEvent, SliderState},
};

use super::{
    SettingsPage, moved,
    sections::{note, row, section},
};
use crate::{
    app::{DiscordApp, voice::switch},
    discord::{AudioDevice, MicTest, Session, audio_devices},
    settings::Settings,
    theme::{self, rpx},
};

const METER_TICK: Duration = Duration::from_millis(50);
const AUTOMATIC_THRESHOLD: f32 = -50.;
const DEVICE_LIST_MAX_HEIGHT: f32 = 260.;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DeviceKind {
    Input,
    Output,
}

/// Voice & Video page state, created the first time the page is shown.
pub(super) struct VoiceAudio {
    devices: Option<(Vec<AudioDevice>, Vec<AudioDevice>)>,
    open_list: Option<DeviceKind>,
    open_list_trigger: Rc<Cell<Option<Bounds<Pixels>>>>,
    input_volume: Entity<SliderState>,
    output_volume: Entity<SliderState>,
    sensitivity: Entity<SliderState>,
    meter: Entity<LevelMeter>,
    test: Option<Rc<MicTest>>,
    test_error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
    _devices: Task<()>,
}

/// Applies the saved audio preferences to a session, before joining a call.
pub(crate) fn apply_preferences(session: &Session, settings: &Settings) {
    session.set_input_device(settings.voice_input_device.clone());
    session.set_output_device(settings.voice_output_device.clone());
    session.set_input_volume(settings.voice_input_volume.unwrap_or(1.));
    session.set_output_volume(settings.voice_output_volume.unwrap_or(1.));
    session.set_input_sensitivity(settings.voice_sensitivity);
}

impl DiscordApp {
    pub(super) fn ensure_voice_audio(&mut self, cx: &mut Context<Self>) {
        if self
            .settings_page
            .as_ref()
            .is_none_or(|settings| settings.voice_audio.is_some())
        {
            return;
        }
        let saved = crate::settings::get(cx);
        let percent = |gain: Option<f32>| gain.unwrap_or(1.) * 100.;
        let slider = |cx: &mut Context<Self>, min: f32, max: f32, value: f32| {
            cx.new(|_| {
                SliderState::new()
                    .min(min)
                    .max(max)
                    .step(1.)
                    .default_value(value)
            })
        };
        let input_volume = slider(cx, 0., 200., percent(saved.voice_input_volume));
        let output_volume = slider(cx, 0., 200., percent(saved.voice_output_volume));
        let sensitivity = slider(
            cx,
            -100.,
            0.,
            saved.voice_sensitivity.unwrap_or(AUTOMATIC_THRESHOLD),
        );
        let subscriptions = vec![
            cx.subscribe(&input_volume, |app, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    app.set_input_volume(value.start() / 100., cx);
                }
            }),
            cx.subscribe(&output_volume, |app, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    app.set_output_volume(value.start() / 100., cx);
                }
            }),
            cx.subscribe(&sensitivity, |app, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    app.set_input_sensitivity(Some(value.start()), cx);
                }
            }),
        ];
        let session = self.session.clone().filter(|_| self.voice.call.is_some());
        let meter = cx.new(|cx| LevelMeter::new(session, saved.voice_sensitivity, cx));
        let devices = cx.spawn(async move |this, cx| {
            let devices = cx
                .background_executor()
                .spawn(async { audio_devices() })
                .await;
            this.update(cx, |app, cx| {
                if let Some(audio) = app
                    .settings_page
                    .as_mut()
                    .and_then(|settings| settings.voice_audio.as_mut())
                {
                    audio.devices = Some(devices);
                    cx.notify();
                }
            })
            .ok();
        });
        if let Some(settings) = &mut self.settings_page {
            settings.voice_audio = Some(VoiceAudio {
                devices: None,
                open_list: None,
                open_list_trigger: Rc::default(),
                input_volume,
                output_volume,
                sensitivity,
                meter,
                test: None,
                test_error: None,
                _subscriptions: subscriptions,
                _devices: devices,
            });
        }
    }

    fn voice_audio(&mut self) -> Option<&mut VoiceAudio> {
        self.settings_page
            .as_mut()
            .and_then(|settings| settings.voice_audio.as_mut())
    }

    fn toggle_device_list(&mut self, kind: DeviceKind, cx: &mut Context<Self>) {
        if let Some(audio) = self.voice_audio() {
            audio.open_list = (audio.open_list != Some(kind)).then_some(kind);
            cx.notify();
        }
    }

    fn close_device_list(&mut self, cx: &mut Context<Self>) {
        if let Some(audio) = self.voice_audio() {
            audio.open_list = None;
            cx.notify();
        }
    }

    fn select_device(&mut self, kind: DeviceKind, name: Option<String>, cx: &mut Context<Self>) {
        crate::settings::update(cx, |settings| match kind {
            DeviceKind::Input => settings.voice_input_device = name.clone(),
            DeviceKind::Output => settings.voice_output_device = name.clone(),
        });
        if let Some(session) = &self.session {
            match kind {
                DeviceKind::Input => session.set_input_device(name),
                DeviceKind::Output => session.set_output_device(name),
            }
        }
        let testing = self.voice_audio().is_some_and(|audio| audio.test.is_some());
        if let Some(audio) = self.voice_audio() {
            audio.open_list = None;
        }
        // A running mic test holds the old devices open; restart it on the new ones.
        if testing {
            self.stop_mic_test(cx);
            self.start_mic_test(cx);
        }
        cx.notify();
    }

    fn set_input_volume(&mut self, gain: f32, cx: &mut Context<Self>) {
        crate::settings::update(cx, |settings| {
            settings.voice_input_volume = (gain != 1.).then_some(gain)
        });
        if let Some(session) = &self.session {
            session.set_input_volume(gain);
        }
        if let Some(test) = self.voice_audio().and_then(|audio| audio.test.clone()) {
            test.set_input_gain(gain);
        }
    }

    fn set_output_volume(&mut self, volume: f32, cx: &mut Context<Self>) {
        crate::settings::update(cx, |settings| {
            settings.voice_output_volume = (volume != 1.).then_some(volume)
        });
        if let Some(session) = &self.session {
            session.set_output_volume(volume);
        }
        if let Some(test) = self.voice_audio().and_then(|audio| audio.test.clone()) {
            test.set_output_volume(volume);
        }
    }

    fn set_input_sensitivity(&mut self, threshold: Option<f32>, cx: &mut Context<Self>) {
        crate::settings::update(cx, |settings| settings.voice_sensitivity = threshold);
        if let Some(session) = &self.session {
            session.set_input_sensitivity(threshold);
        }
        if let Some(audio) = self.voice_audio() {
            if let Some(test) = &audio.test {
                test.set_input_sensitivity(threshold);
            }
            audio
                .meter
                .update(cx, |meter, cx| meter.set_threshold(threshold, cx));
        }
        cx.notify();
    }

    fn toggle_automatic_sensitivity(&mut self, cx: &mut Context<Self>) {
        let automatic = crate::settings::get(cx).voice_sensitivity.is_none();
        let manual = self
            .voice_audio()
            .map(|audio| audio.sensitivity.read(cx).value().start())
            .unwrap_or(AUTOMATIC_THRESHOLD);
        self.mark_switch_moved("settings-auto-sensitivity");
        self.set_input_sensitivity(automatic.then_some(manual), cx);
    }

    fn start_mic_test(&mut self, cx: &mut Context<Self>) {
        let saved = crate::settings::get(cx);
        let noise = self.voice.noise_suppression;
        let result = MicTest::start(
            saved.voice_input_device.clone(),
            saved.voice_output_device.clone(),
            saved.voice_input_volume.unwrap_or(1.),
            saved.voice_output_volume.unwrap_or(1.),
            noise,
        );
        let Some(audio) = self.voice_audio() else {
            return;
        };
        match result {
            Ok(test) => {
                test.set_input_sensitivity(saved.voice_sensitivity);
                let test = Rc::new(test);
                audio.test = Some(test.clone());
                audio.test_error = None;
                audio
                    .meter
                    .update(cx, |meter, cx| meter.set_source(Source::Test(test), cx));
            }
            Err(error) => audio.test_error = Some(error.into()),
        }
        cx.notify();
    }

    fn stop_mic_test(&mut self, cx: &mut Context<Self>) {
        let session = self.session.clone().filter(|_| self.voice.call.is_some());
        if let Some(audio) = self.voice_audio() {
            audio.test = None;
            audio.meter.update(cx, |meter, cx| {
                meter.set_source(session.map_or(Source::None, Source::Call), cx)
            });
        }
        cx.notify();
    }
}

pub(super) fn page(app: &DiscordApp, settings: &SettingsPage, cx: &App) -> Vec<AnyElement> {
    let Some(audio) = settings.voice_audio.as_ref() else {
        return Vec::new();
    };
    let saved = crate::settings::get(cx);
    let in_call = app.voice.call.is_some();
    let devices = div()
        .flex()
        .gap(rpx(16.))
        .child(device_picker(
            app,
            audio,
            DeviceKind::Input,
            saved.voice_input_device.as_deref(),
        ))
        .child(device_picker(
            app,
            audio,
            DeviceKind::Output,
            saved.voice_output_device.as_deref(),
        ));
    let volumes = div()
        .flex()
        .gap(rpx(16.))
        .child(labeled_slider("Input Volume", &audio.input_volume, cx))
        .child(labeled_slider("Output Volume", &audio.output_volume, cx));
    let test_button = {
        let entity = app.entity.clone();
        let testing = audio.test.is_some();
        let label = if testing {
            "Stop Testing"
        } else {
            "Let's Check"
        };
        div()
            .id("settings-mic-test")
            .flex()
            .flex_none()
            .items_center()
            .h(rpx(32.))
            .px(rpx(16.))
            .rounded(rpx(8.))
            .text_size(rpx(14.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(white())
            .map(|this| {
                if in_call {
                    this.bg(rgba(theme::selected()))
                        .text_color(rgb(theme::faint()))
                } else {
                    this.cursor_pointer()
                        .bg(rgb(theme::BLURPLE))
                        .hover(|style| style.opacity(0.9))
                        .on_click(move |_, _, cx| {
                            entity
                                .update(cx, |app, cx| {
                                    if testing {
                                        app.stop_mic_test(cx);
                                    } else {
                                        app.start_mic_test(cx);
                                    }
                                })
                                .ok();
                        })
                }
            })
            .child(label)
    };
    let test = div()
        .flex()
        .flex_col()
        .gap(rpx(8.))
        .child(
            div()
                .text_size(rpx(14.))
                .line_height(rpx(18.))
                .text_color(rgb(theme::muted()))
                .child(if in_call {
                    "Your input level below is live from the call. The mic test is off while \
                     you're in a call."
                } else {
                    "Having mic issues? Start a test and say something fun. We'll play your \
                     voice back to you."
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(rpx(16.))
                .child(test_button)
                .child(div().flex_1().child(audio.meter.clone())),
        )
        .children(audio.test_error.clone().map(|error| {
            div()
                .text_size(rpx(14.))
                .text_color(rgb(theme::RED))
                .child(error)
        }));
    let automatic = saved.voice_sensitivity.is_none();
    let auto_toggle = {
        let entity = app.entity.clone();
        div()
            .id("settings-auto-sensitivity")
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| app.toggle_automatic_sensitivity(cx))
                    .ok();
            })
            .child(switch(
                "settings-auto-sensitivity",
                automatic,
                moved(settings, "settings-auto-sensitivity"),
            ))
    };
    let noise_toggle = {
        let entity = app.entity.clone();
        div()
            .id("settings-noise-suppression")
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                entity
                    .update(cx, |app, cx| {
                        app.mark_switch_moved("settings-noise");
                        app.toggle_noise_suppression(cx);
                        let on = app.voice.noise_suppression;
                        if let Some(test) = app.voice_audio().and_then(|audio| audio.test.clone()) {
                            test.set_noise_suppression(on);
                        }
                    })
                    .ok();
            })
            .child(switch(
                "settings-noise",
                app.voice.noise_suppression,
                moved(settings, "settings-noise"),
            ))
    };
    let mut sensitivity_rows = vec![row(
        "Automatically determine input sensitivity",
        Some("Uses RNNoise to tell your voice apart from background noise."),
        auto_toggle.into_any_element(),
    )];
    if !automatic {
        sensitivity_rows.push(
            div()
                .flex()
                .flex_col()
                .gap(rpx(8.))
                .pb(rpx(8.))
                .child(div().h(rpx(20.)).child(
                    Slider::new(&audio.sensitivity)
                        .bg(rgb(theme::BLURPLE))
                        .text_color(white()),
                ))
                .child(
                    div()
                        .text_size(rpx(14.))
                        .text_color(rgb(theme::muted()))
                        .child(format!(
                            "Your mic opens above {:.0} dB. Green in the meter means you'd be heard.",
                            audio.sensitivity.read(cx).value().start()
                        )),
                )
                .into_any_element(),
        );
    }
    vec![
        section(
            "Voice",
            vec![
                devices.into_any_element(),
                div().h(rpx(24.)).into_any_element(),
                volumes.into_any_element(),
            ],
        ),
        section("Mic Test", vec![test.into_any_element()]),
        section("Input Sensitivity", sensitivity_rows),
        section(
            "Voice Processing",
            vec![
                row(
                    "Noise Suppression",
                    Some(
                        "Filters steady background noise like fans, hum and typing out of your \
                         microphone. Powered by RNNoise.",
                    ),
                    noise_toggle.into_any_element(),
                ),
                note("Changes apply right away, including in a call you're already in."),
            ],
        ),
    ]
}

fn device_picker(
    app: &DiscordApp,
    audio: &VoiceAudio,
    kind: DeviceKind,
    selected: Option<&str>,
) -> impl IntoElement {
    let (label, id) = match kind {
        DeviceKind::Input => ("Input Device", "settings-input-device"),
        DeviceKind::Output => ("Output Device", "settings-output-device"),
    };
    let current = selected.unwrap_or("Windows default").to_owned();
    let open = audio.open_list == Some(kind);
    let toggle = app.entity.clone();
    let trigger = audio.open_list_trigger.clone();
    let button = div()
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .gap(rpx(8.))
        .h(rpx(40.))
        .px(rpx(12.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .bg(rgb(theme::bg()))
        .border_1()
        .border_color(if open {
            rgb(theme::focus())
        } else {
            rgba(theme::border())
        })
        .on_click(move |_, _, cx| {
            toggle
                .update(cx, |app, cx| app.toggle_device_list(kind, cx))
                .ok();
        })
        .when(open, |this| {
            this.on_prepaint(move |bounds, _, _| trigger.set(Some(bounds)))
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(rpx(14.))
                .text_color(rgb(theme::strong()))
                .child(current),
        )
        .child(theme::icon(
            if open { "chevron-up" } else { "chevron-down" },
            16.,
            theme::muted(),
        ));
    let names: Vec<Option<String>> = match &audio.devices {
        Some((inputs, outputs)) => std::iter::once(None)
            .chain(
                match kind {
                    DeviceKind::Input => inputs,
                    DeviceKind::Output => outputs,
                }
                .iter()
                .map(|device| Some(device.name.clone())),
            )
            .collect(),
        None => vec![None],
    };

    let list = open.then(|| {
        let close = app.entity.clone();
        let trigger = audio.open_list_trigger.clone();
        deferred(
            div()
                .id("settings-device-list")
                .absolute()
                .top(rpx(40.))
                .left_0()
                .right_0()
                .flex()
                .flex_col()
                .mt(rpx(4.))
                .p(rpx(4.))
                .rounded(rpx(8.))
                .bg(rgb(theme::bg()))
                .border_1()
                .border_color(rgba(theme::border()))
                .max_h(rpx(DEVICE_LIST_MAX_HEIGHT))
                .overflow_y_scrollbar()
                .occlude()
                .on_mouse_down_out(move |event, _, cx| {
                    // The trigger closes on click; closing on mouse-down would reopen it.
                    if trigger
                        .get()
                        .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        return;
                    }
                    close.update(cx, |app, cx| app.close_device_list(cx)).ok();
                })
                .children(names.into_iter().enumerate().map(|(index, name)| {
                    let chosen = name.as_deref() == selected;
                    let entity = app.entity.clone();
                    let text = name.clone().unwrap_or_else(|| "Windows default".to_owned());
                    div()
                        .id((id, index as u64))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_between()
                        .gap(rpx(8.))
                        .h(rpx(36.))
                        .px(rpx(8.))
                        .rounded(rpx(6.))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgba(theme::hover())))
                        .on_click(move |_, _, cx| {
                            entity
                                .update(cx, |app, cx| app.select_device(kind, name.clone(), cx))
                                .ok();
                        })
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(rpx(14.))
                                .text_color(rgb(if chosen {
                                    theme::strong()
                                } else {
                                    theme::text()
                                }))
                                .child(text),
                        )
                        .when(chosen, |this| {
                            this.child(theme::icon("check", 16., theme::BLURPLE))
                        })
                }))
                .when(audio.devices.is_none(), |this| {
                    this.child(
                        div()
                            .flex_none()
                            .px(rpx(8.))
                            .py(rpx(6.))
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::muted()))
                            .child("Looking for devices…"),
                    )
                }),
        )
        .with_priority(1)
    });
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(rpx(8.))
        .child(field_label(label))
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .child(button)
                .children(list),
        )
}

fn labeled_slider(label: &'static str, state: &Entity<SliderState>, cx: &App) -> impl IntoElement {
    let percent = state.read(cx).value().start().round() as u32;
    div()
        .flex()
        .flex_col()
        .flex_1()
        .gap(rpx(8.))
        .child(
            div()
                .flex()
                .justify_between()
                .child(field_label(label))
                .child(
                    div()
                        .text_size(rpx(14.))
                        .text_color(rgb(theme::muted()))
                        .child(format!("{percent}%")),
                ),
        )
        .child(
            div().h(rpx(20.)).child(
                Slider::new(state)
                    .bg(rgb(theme::BLURPLE))
                    .text_color(white()),
            ),
        )
}

fn field_label(text: &'static str) -> impl IntoElement {
    div()
        .text_size(rpx(12.))
        .line_height(rpx(16.))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(theme::muted()))
        .child(text.to_uppercase())
}

pub(super) enum Source {
    None,
    Call(Session),
    Test(Rc<MicTest>),
}

/// Live mic level bar. Its own view, so its 20 fps repaint stays inside the settings dialog.
pub(super) struct LevelMeter {
    source: Source,
    threshold: Option<f32>,
    _tick: Task<()>,
}

impl LevelMeter {
    fn new(session: Option<Session>, threshold: Option<f32>, cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(METER_TICK).await;
                let alive = this.update(cx, |meter, cx| {
                    if !matches!(meter.source, Source::None) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        Self {
            source: session.map_or(Source::None, Source::Call),
            threshold,
            _tick: tick,
        }
    }

    fn set_source(&mut self, source: Source, cx: &mut Context<Self>) {
        self.source = source;
        cx.notify();
    }

    fn set_threshold(&mut self, threshold: Option<f32>, cx: &mut Context<Self>) {
        self.threshold = threshold;
        cx.notify();
    }
}

impl Render for LevelMeter {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (level, open) = match &self.source {
            Source::None => (None, false),
            Source::Call(session) => (session.input_level(), session.input_gate_open()),
            Source::Test(test) => (test.level(), test.gate_open()),
        };
        let fill = level.map_or(0., |db| ((db + 100.) / 100.).clamp(0., 1.));
        let marker = self.threshold.map(|db| ((db + 100.) / 100.).clamp(0., 1.));
        div()
            .relative()
            .h(rpx(8.))
            .rounded_full()
            .overflow_hidden()
            .bg(rgba(theme::selected()))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(relative(fill))
                    .bg(rgb(if open { theme::GREEN } else { theme::YELLOW })),
            )
            .children(marker.map(|fraction| {
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(fraction))
                    .w(px(2.))
                    .bg(rgb(theme::bright()))
            }))
    }
}
