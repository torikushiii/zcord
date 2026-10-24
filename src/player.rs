use crate::{
    app::DiscordApp,
    images::{self, ImageSpec},
    theme::{self, rpx},
};
use gpui::{prelude::*, *};
use std::sync::{Arc, Mutex};

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum Command {
    Play,
    Pause,
    Seek(f64),
    SetMuted(bool),
    SetVolume(f64),
    Stop,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum VideoDrag {
    Seek,
    Volume,
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum PlayerEvent {
    Frame(Arc<RenderImage>),
    State,
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Default)]
struct PlayerState {
    frame: Option<Arc<RenderImage>>,
    retired: Option<Arc<RenderImage>>,
    stopped: bool,
    duration: f64,
    position: f64,
    paused: bool,
    ended: bool,
    buffering: bool,
    error: Option<String>,
    width: u32,
    height: u32,
    muted: bool,
    volume: f64,
}

#[derive(Clone, Default)]
pub(crate) struct PlayerView {
    pub frame: Option<Arc<RenderImage>>,
    pub duration: f64,
    pub position: f64,
    pub paused: bool,
    pub ended: bool,
    pub buffering: bool,
    pub error: Option<String>,
    pub muted: bool,
    pub volume: f64,
}

pub(crate) struct Player {
    pub key: u64,
    state: Arc<Mutex<PlayerState>>,
    commands: std::sync::mpsc::Sender<Command>,
    events: Option<tokio::sync::mpsc::UnboundedReceiver<PlayerEvent>>,
}

impl Player {
    pub(crate) fn start(key: u64, url: String, width: u32, height: u32) -> Option<Self> {
        let url = images::trusted_video_url(&url)?;
        platform::start(key, url, width.clamp(2, 1920), height.clamp(2, 1920))
    }

    pub(crate) fn view(&self) -> PlayerView {
        let state = self.state.lock().unwrap();
        PlayerView {
            frame: state.frame.clone(),
            duration: state.duration,
            position: state.position,
            paused: state.paused,
            ended: state.ended,
            buffering: state.buffering,
            error: state.error.clone(),
            muted: state.muted,
            volume: state.volume,
        }
    }

    pub(crate) fn toggle(&self) {
        let command = {
            let mut state = self.state.lock().unwrap();
            if state.ended {
                state.ended = false;
                state.paused = false;
                state.position = 0.;
                self.commands.send(Command::Seek(0.)).ok();
                Command::Play
            } else {
                state.paused = !state.paused;
                if state.paused {
                    Command::Pause
                } else {
                    Command::Play
                }
            }
        };
        self.commands.send(command).ok();
    }

    pub(crate) fn seek(&self, seconds: f64) {
        let seconds = {
            let mut state = self.state.lock().unwrap();
            let seconds = seconds.clamp(0., state.duration.max(0.));
            state.position = seconds;
            state.ended = false;
            seconds
        };
        self.commands.send(Command::Seek(seconds)).ok();
    }

    pub(crate) fn toggle_muted(&self) {
        let (muted, volume) = {
            let mut state = self.state.lock().unwrap();
            state.muted = !state.muted;
            // Unmuting at zero volume would stay silent, so restore full volume.
            if !state.muted && state.volume <= 0. {
                state.volume = 1.;
            }
            (state.muted, state.volume)
        };
        self.commands.send(Command::SetVolume(volume)).ok();
        self.commands.send(Command::SetMuted(muted)).ok();
    }

    pub(crate) fn set_volume(&self, volume: f64) {
        let volume = volume.clamp(0., 1.);
        let unmute = {
            let mut state = self.state.lock().unwrap();
            state.volume = volume;
            let unmute = state.muted && volume > 0.;
            state.muted &= !unmute;
            unmute
        };
        self.commands.send(Command::SetVolume(volume)).ok();
        if unmute {
            self.commands.send(Command::SetMuted(false)).ok();
        }
    }

    pub(crate) fn attach(&mut self, cx: &mut Context<DiscordApp>) {
        let Some(mut events) = self.events.take() else {
            return;
        };
        let state = self.state.clone();
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let retired = match event {
                    PlayerEvent::Frame(image) => {
                        let mut state = state.lock().unwrap();
                        if state.stopped {
                            Some(image)
                        } else {
                            match state.frame.replace(image) {
                                Some(previous) => state.retired.replace(previous),
                                None => state.retired.take(),
                            }
                        }
                    }
                    PlayerEvent::State => None,
                };
                // Videos play inline in messages or in the lightbox overlay; redraw only those
                // views rather than refreshing every cached panel at the video's frame rate.
                this.update(cx, |app, cx| {
                    app.views.messages.update(cx, |_, cx| cx.notify());
                    app.views.overlays.update(cx, |_, cx| cx.notify());
                })
                .ok();
                if let Some(image) = retired {
                    cx.update(|cx| cx.drop_image(image, None));
                }
            }
        })
        .detach();
    }

    pub(crate) fn take_frames(&self) -> Vec<Arc<RenderImage>> {
        let mut state = self.state.lock().unwrap();
        state.stopped = true;
        state
            .frame
            .take()
            .into_iter()
            .chain(state.retired.take())
            .collect()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.commands.send(Command::Stop).ok();
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub(super) fn start(_: u64, _: String, _: u32, _: u32) -> Option<Player> {
        None
    }
}

pub(crate) struct Video<'a> {
    pub key: u64,
    pub url: &'a str,
    pub poster: Option<ImageSpec>,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub fullscreen: bool,
}

pub(crate) struct Fullscreen {
    pub key: u64,
    pub url: String,
    pub poster: Option<ImageSpec>,
    pub entered_window_fullscreen: bool,
}

pub(crate) fn fullscreen(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let fullscreen = app.video_fullscreen.as_ref()?;
    let video = video_box(
        app,
        Video {
            key: fullscreen.key,
            url: &fullscreen.url,
            poster: fullscreen.poster.clone(),
            width: 0.,
            height: 0.,
            radius: 0.,
            fullscreen: true,
        },
        cx,
    );
    Some(
        div()
            .id("video-fullscreen")
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgb(0x000000))
            .child(video)
            .into_any_element(),
    )
}

pub(crate) fn video_box(app: &DiscordApp, video: Video<'_>, cx: &App) -> AnyElement {
    let state = app
        .player
        .as_ref()
        .filter(|player| player.key == video.key)
        .map(Player::view);
    let active = state.is_some();
    let error = state.as_ref().is_some_and(|state| state.error.is_some());
    let poster = video
        .poster
        .as_ref()
        .and_then(|spec| images::resolve(spec, theme::image_px(video.width.max(video.height)), cx));
    let frame = state.as_ref().and_then(|state| state.frame.clone());
    let radius = video.radius;
    let fullscreen = video.fullscreen;
    // The full-window player owns the controls while it is open.
    let covered = !fullscreen
        && app
            .video_fullscreen
            .as_ref()
            .is_some_and(|open| open.key == video.key);
    let fit = if fullscreen {
        ObjectFit::Contain
    } else {
        ObjectFit::Cover
    };

    let visual: AnyElement = if let Some(frame) = frame {
        img(frame)
            .size_full()
            .object_fit(fit)
            .rounded(rpx(radius))
            .into_any_element()
    } else if let Some(poster) = poster {
        img(poster)
            .size_full()
            .object_fit(fit)
            .rounded(rpx(radius))
            .into_any_element()
    } else {
        div().size_full().bg(rgb(theme::BORDER)).into_any_element()
    };

    let hovered = app.hovered_video == Some(video.key);
    let click_entity = app.entity.clone();
    let hover_entity = app.entity.clone();
    let move_entity = app.entity.clone();
    let up_entity = app.entity.clone();
    let up_out_entity = app.entity.clone();
    let key = video.key;
    let width = video.width;
    let height = video.height;
    let url = video.url.to_owned();
    let target = FullscreenTarget {
        key,
        url: url.clone(),
        poster: video.poster.clone(),
    };

    let mut body = div()
        .id(("video-box", key))
        .relative()
        .map(|this| {
            if fullscreen {
                this.size_full().bg(rgb(0x000000))
            } else {
                this.flex_none()
                    .w(rpx(width))
                    .h(rpx(height))
                    .rounded(rpx(radius))
                    .bg(rgb(theme::ELEVATED))
            }
        })
        .overflow_hidden()
        .cursor_pointer()
        .on_hover(move |hovered, _, cx| {
            hover_entity
                .update(cx, |app, cx| app.set_video_hover(key, *hovered, cx))
                .ok();
        })
        .on_mouse_move(move |event, _, cx| {
            if event.pressed_button == Some(MouseButton::Left) {
                move_entity
                    .update(cx, |app, cx| app.drag_video(key, event.position, cx))
                    .ok();
            }
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
            up_entity
                .update(cx, |app, cx| app.end_video_drag(key, cx))
                .ok();
        })
        .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
            up_out_entity
                .update(cx, |app, cx| app.end_video_drag(key, cx))
                .ok();
        })
        .on_click(move |_, window, cx| {
            let entity = click_entity.clone();
            entity
                .update(cx, |app, cx| {
                    if active {
                        app.video_click(key, cx);
                    } else if !error {
                        app.start_video(key, url.clone(), width, height, window, cx);
                    }
                })
                .ok();
        })
        .child(visual);

    if !active {
        body = body.child(center_play());
    } else if let Some(state) = &state {
        if let Some(error) = &state.error {
            body = body.child(error_overlay(error, &video.url));
        } else {
            if state.buffering {
                body = body.child(loading_overlay());
            }
            if state.paused || state.ended {
                body = body.child(center_play());
            }
            if !covered && (hovered || state.paused || state.ended || app.video_volume_open()) {
                body = body.child(control_bar(app, &target, fullscreen, state));
            }
        }
    }

    body.into_any_element()
}

fn center_play() -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(56.))
                .rounded_full()
                .bg(rgba(0x000000A6))
                .child(theme::icon("video-play", 28., theme::STRONG)),
        )
        .into_any_element()
}

fn loading_overlay() -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .px(rpx(12.))
                .py(rpx(6.))
                .rounded_full()
                .bg(rgba(0x000000A6))
                .text_size(rpx(13.))
                .text_color(rgb(theme::STRONG))
                .child("Loading…"),
        )
        .into_any_element()
}

fn error_overlay(error: &str, url: &str) -> AnyElement {
    let url = url.to_owned();
    div()
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(rpx(4.))
        .bg(rgba(0x000000B3))
        .child(
            div()
                .text_size(rpx(14.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(theme::STRONG))
                .child("Couldn't play video"),
        )
        .child(
            div()
                .max_w(rpx(280.))
                .text_size(rpx(11.))
                .text_color(rgb(theme::MUTED))
                .child(error.to_owned()),
        )
        .child(
            div()
                .id("video-open")
                .cursor_pointer()
                .text_size(rpx(13.))
                .text_color(rgb(theme::BLURPLE))
                .hover(|style| style.underline())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    cx.open_url(&url);
                })
                .child("Open"),
        )
        .into_any_element()
}

#[derive(Clone)]
struct FullscreenTarget {
    key: u64,
    url: String,
    poster: Option<ImageSpec>,
}

fn control_bar(
    app: &DiscordApp,
    target: &FullscreenTarget,
    fullscreen: bool,
    state: &PlayerView,
) -> AnyElement {
    let key = target.key;
    let play_entity = app.entity.clone();
    let seek_entity = app.entity.clone();
    let seek_bounds = app.video_seek_bounds.clone();
    let progress = if state.duration > 0. {
        (state.position / state.duration).clamp(0., 1.) as f32
    } else {
        0.
    };

    let play = control_button(("video-play", key))
        .child(theme::icon(
            if state.paused || state.ended {
                "video-play"
            } else {
                "video-pause"
            },
            22.,
            theme::STRONG,
        ))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            play_entity
                .update(cx, |app, cx| app.toggle_video(key, cx))
                .ok();
        });

    let seek = div()
        .id(("video-seek", key))
        .flex_1()
        .min_w_0()
        .h(rpx(24.))
        .cursor_pointer()
        .on_click(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            cx.stop_propagation();
            seek_entity
                .update(cx, |app, cx| {
                    app.begin_video_drag(key, VideoDrag::Seek, event.position, cx)
                })
                .ok();
        })
        .child(
            canvas(
                move |bounds, _, _| {
                    seek_bounds.borrow_mut().replace((key, bounds));
                },
                move |bounds, _, window, _| {
                    let thickness = rpx(6.).to_pixels(window.rem_size());
                    let track = Bounds::new(
                        point(
                            bounds.origin.x,
                            bounds.origin.y + (bounds.size.height - thickness) / 2.,
                        ),
                        size(bounds.size.width, thickness),
                    );
                    paint_slider(window, track, progress, false);
                },
            )
            .size_full(),
        );

    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .px(rpx(8.))
        .pt(rpx(24.))
        .pb(rpx(6.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgba(0x00000000), 0.),
            linear_color_stop(rgba(0x000000E6), 1.),
        ))
        .id(("video-controls", key))
        .on_click(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(play)
        .child(seek)
        .child(volume_control(app, key, state))
        .child(fullscreen_button(app, target, fullscreen))
        .into_any_element()
}

fn fullscreen_button(app: &DiscordApp, target: &FullscreenTarget, open: bool) -> impl IntoElement {
    let entity = app.entity.clone();
    let target = target.clone();
    control_button(("video-fullscreen", target.key))
        .child(theme::icon(
            if open {
                "video-exit-fullscreen"
            } else {
                "video-fullscreen"
            },
            20.,
            theme::STRONG,
        ))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            let target = target.clone();
            entity
                .update(cx, |app, cx| {
                    if open {
                        app.exit_video_fullscreen(window, cx);
                    } else {
                        app.enter_video_fullscreen(
                            Fullscreen {
                                key: target.key,
                                url: target.url,
                                poster: target.poster,
                                entered_window_fullscreen: false,
                            },
                            window,
                            cx,
                        );
                    }
                })
                .ok();
        })
}

fn volume_control(app: &DiscordApp, key: u64, state: &PlayerView) -> impl IntoElement {
    let mute_entity = app.entity.clone();
    let button_hover = app.entity.clone();
    let popup_hover = app.entity.clone();
    let drag_entity = app.entity.clone();
    let volume_bounds = app.video_volume_bounds.clone();
    let level = if state.muted { 0. } else { state.volume as f32 };
    let open = app.video_volume_open();

    let button = control_button(("video-mute", key))
        .on_hover(move |hovered, _, cx| {
            button_hover
                .update(cx, |app, cx| app.set_video_volume_hover(0, *hovered, cx))
                .ok();
        })
        .child(theme::icon(
            if level <= 0. {
                "video-muted"
            } else {
                "video-volume"
            },
            22.,
            theme::STRONG,
        ))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            mute_entity
                .update(cx, |app, cx| app.toggle_video_mute(key, cx))
                .ok();
        });

    // The popup's bottom padding bridges the gap to the button so the pointer can
    // travel between them without either losing hover.
    let popup = div()
        .id(("video-volume", key))
        .absolute()
        .left_0()
        .bottom(relative(1.))
        .w(rpx(32.))
        .pb(rpx(4.))
        .on_hover(move |hovered, _, cx| {
            popup_hover
                .update(cx, |app, cx| app.set_video_volume_hover(1, *hovered, cx))
                .ok();
        })
        .on_click(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            cx.stop_propagation();
            drag_entity
                .update(cx, |app, cx| {
                    app.begin_video_drag(key, VideoDrag::Volume, event.position, cx)
                })
                .ok();
        })
        .child(
            div()
                .h(rpx(96.))
                .py(rpx(10.))
                .rounded(rpx(8.))
                .bg(rgba(0x000000B3))
                .cursor_pointer()
                .child(
                    canvas(
                        move |bounds, _, _| {
                            volume_bounds.borrow_mut().replace((key, bounds));
                        },
                        move |bounds, _, window, _| {
                            let thickness = rpx(6.).to_pixels(window.rem_size());
                            let track = Bounds::new(
                                point(
                                    bounds.origin.x + (bounds.size.width - thickness) / 2.,
                                    bounds.origin.y,
                                ),
                                size(thickness, bounds.size.height),
                            );
                            paint_slider(window, track, level, true);
                        },
                    )
                    .size_full(),
                ),
        );

    div()
        .relative()
        .flex_none()
        .child(button)
        .when(open, |this| this.child(popup))
}

fn control_button(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(rpx(32.))
        .rounded(rpx(6.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(0xFFFFFF1A)))
}

fn paint_slider(window: &mut Window, track: Bounds<Pixels>, fraction: f32, vertical: bool) {
    if track.is_empty() {
        return;
    }
    let radius = track.size.width.min(track.size.height) / 2.;
    window.paint_quad(fill(track, rgba(0xFFFFFF40)).corner_radii(radius));
    let fraction = fraction.clamp(0., 1.);
    if fraction <= 0. {
        return;
    }
    let filled = if vertical {
        let height = track.size.height * fraction;
        Bounds::new(
            point(track.origin.x, track.origin.y + track.size.height - height),
            size(track.size.width, height),
        )
    } else {
        Bounds::new(
            track.origin,
            size(track.size.width * fraction, track.size.height),
        )
    };
    window.paint_quad(fill(filled, rgb(theme::BLURPLE)).corner_radii(radius));
}

#[cfg(windows)]
mod platform {
    use super::*;
    use anyhow::{Context, Result};
    use std::time::Duration;
    use windows::{
        Win32::{
            Foundation::{HMODULE, RECT},
            Graphics::{
                Direct3D::{
                    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
                    D3D_FEATURE_LEVEL_11_0,
                },
                Direct3D10::ID3D10Multithread,
                Direct3D11::*,
                Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
            },
            Media::MediaFoundation::*,
            System::Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
            },
        },
        core::{BSTR, Interface},
    };

    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    struct Foundation;
    impl Drop for Foundation {
        fn drop(&mut self) {
            unsafe { MFShutdown().ok() };
        }
    }

    #[windows::core::implement(IMFMediaEngineNotify)]
    struct Notify {
        events: std::sync::mpsc::Sender<(u32, usize)>,
    }

    impl IMFMediaEngineNotify_Impl for Notify_Impl {
        fn EventNotify(
            &self,
            event: u32,
            param1: usize,
            _param2: u32,
        ) -> windows::core::Result<()> {
            self.events.send((event, param1)).ok();
            Ok(())
        }
    }

    pub(super) fn start(key: u64, url: String, width: u32, height: u32) -> Option<Player> {
        let (commands, command_rx) = std::sync::mpsc::channel();
        let (events, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify, notify_rx) = std::sync::mpsc::channel();
        let state = Arc::new(Mutex::new(PlayerState {
            buffering: true,
            volume: 1.,
            ..Default::default()
        }));
        let thread_state = state.clone();
        std::thread::Builder::new()
            .name("zcord-video".into())
            .spawn(move || {
                let _com = unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    Com
                };
                let _foundation = unsafe {
                    let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
                    Foundation
                };
                if let Err(error) = run(
                    &url,
                    width,
                    height,
                    command_rx,
                    notify,
                    notify_rx,
                    &events,
                    &thread_state,
                ) {
                    log::warn!("Video player failed: {error:#}");
                    let mut state = thread_state.lock().unwrap();
                    state.error = Some(format!("{error}"));
                    state.buffering = false;
                    events.send(PlayerEvent::State).ok();
                }
            })
            .ok()?;
        Some(Player {
            key,
            state,
            commands,
            events: Some(event_rx),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        url: &str,
        width: u32,
        height: u32,
        commands: std::sync::mpsc::Receiver<Command>,
        notify: std::sync::mpsc::Sender<(u32, usize)>,
        notify_rx: std::sync::mpsc::Receiver<(u32, usize)>,
        events: &tokio::sync::mpsc::UnboundedSender<PlayerEvent>,
        state: &Arc<Mutex<PlayerState>>,
    ) -> Result<()> {
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[
                    D3D_FEATURE_LEVEL_11_0,
                    D3D_FEATURE_LEVEL_10_1,
                    D3D_FEATURE_LEVEL_10_0,
                ]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        let device = device.context("no D3D11 device")?;
        let context = context.context("no D3D11 context")?;
        let multithread: ID3D10Multithread = device.cast()?;
        unsafe {
            let _ = multithread.SetMultithreadProtected(true);
        };

        let mut token = 0;
        let mut manager = None;
        unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager)? };
        let manager = manager.context("no DXGI device manager")?;
        unsafe { manager.ResetDevice(&device, token)? };

        let mut attributes = None;
        unsafe { MFCreateAttributes(&mut attributes, 4)? };
        let attributes = attributes.context("no media attributes")?;
        let callback: IMFMediaEngineNotify = Notify {
            events: notify.clone(),
        }
        .into();
        unsafe {
            attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &callback)?;
            attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &manager)?;
            attributes.SetUINT32(
                &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
            )?;
        }

        let factory: IMFMediaEngineClassFactory =
            unsafe { CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_ALL)? };
        let engine = unsafe { factory.CreateInstance(0, &attributes)? };
        let source = BSTR::from(url);
        unsafe {
            engine.SetAutoPlay(false)?;
            engine.SetMuted(false)?;
            engine.SetVolume(1.)?;
            engine.SetSource(&source)?;
            engine.Load()?;
            engine.Play()?;
        }

        let target = texture(&device, width, height, false)?;
        let staging = texture(&device, width, height, true)?;
        let rect = RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let border = MFARGB {
            rgbBlue: 0,
            rgbGreen: 0,
            rgbRed: 0,
            rgbAlpha: 255,
        };
        let mut last_pts = None;

        loop {
            let delay = if state.lock().unwrap().paused {
                Duration::from_millis(16)
            } else {
                Duration::from_millis(4)
            };
            match commands.recv_timeout(delay) {
                Ok(Command::Stop) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    unsafe { engine.Shutdown().ok() };
                    return Ok(());
                }
                Ok(command) => {
                    match command {
                        Command::Play => unsafe { engine.Play()? },
                        Command::Pause => unsafe { engine.Pause()? },
                        Command::Seek(seconds) => {
                            unsafe { engine.SetCurrentTime(seconds)? };
                            last_pts = None;
                        }
                        Command::SetMuted(muted) => unsafe { engine.SetMuted(muted)? },
                        Command::SetVolume(volume) => unsafe { engine.SetVolume(volume)? },
                        Command::Stop => unreachable!(),
                    }
                    events.send(PlayerEvent::State).ok();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }

            while let Ok((event, param)) = notify_rx.try_recv() {
                let mut state = state.lock().unwrap();
                match event {
                    value if value == MF_MEDIA_ENGINE_EVENT_BUFFERINGSTARTED.0 as u32 => {
                        state.buffering = true;
                    }
                    value
                        if value == MF_MEDIA_ENGINE_EVENT_BUFFERINGENDED.0 as u32
                            || value == MF_MEDIA_ENGINE_EVENT_CANPLAY.0 as u32
                            || value == MF_MEDIA_ENGINE_EVENT_CANPLAYTHROUGH.0 as u32
                            || value == MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY.0 as u32 =>
                    {
                        state.buffering = false;
                    }
                    value if value == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 => {
                        state.buffering = false;
                        state.error = Some(error_text(param as i32));
                    }
                    _ => {}
                }
                drop(state);
                events.send(PlayerEvent::State).ok();
            }
            if state.lock().unwrap().error.is_some() {
                unsafe { engine.Shutdown().ok() };
                return Ok(());
            }

            if let Ok(pts) = unsafe { engine.OnVideoStreamTick() } {
                if last_pts != Some(pts) {
                    last_pts = Some(pts);
                    if unsafe { engine.TransferVideoFrame(&target, None, &rect, Some(&border)) }
                        .is_ok()
                    {
                        unsafe { context.CopyResource(&staging, &target) };
                        if let Some(frame) = copy_frame(&context, &staging, width, height) {
                            let mut state = state.lock().unwrap();
                            state.buffering = false;
                            drop(state);
                            events.send(PlayerEvent::Frame(frame)).ok();
                        }
                    }
                }
            }

            let changed = update_state(&engine, state);
            if changed {
                events.send(PlayerEvent::State).ok();
            }
        }
    }

    fn texture(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        staging: bool,
    ) -> Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: if staging {
                D3D11_USAGE_STAGING
            } else {
                D3D11_USAGE_DEFAULT
            },
            BindFlags: if staging {
                0
            } else {
                D3D11_BIND_RENDER_TARGET.0 as u32
            },
            CPUAccessFlags: if staging {
                D3D11_CPU_ACCESS_READ.0 as u32
            } else {
                0
            },
            MiscFlags: 0,
        };
        let mut texture = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        texture.context("no video texture")
    }

    fn copy_frame(
        context: &ID3D11DeviceContext,
        staging: &ID3D11Texture2D,
        width: u32,
        height: u32,
    ) -> Option<Arc<RenderImage>> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .ok()?;
        }
        let row = width as usize * 4;
        let mut pixels = Vec::with_capacity(row * height as usize);
        let result = if mapped.pData.is_null() || mapped.RowPitch < row as u32 {
            None
        } else {
            for y in 0..height as usize {
                let start =
                    unsafe { (mapped.pData as *const u8).add(y * mapped.RowPitch as usize) };
                let row = unsafe { std::slice::from_raw_parts(start, row) };
                pixels.extend_from_slice(row);
            }
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
            image::RgbaImage::from_raw(width, height, pixels)
        };
        unsafe { context.Unmap(staging, 0) };
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(result?)])))
    }

    fn update_state(engine: &IMFMediaEngine, state: &Arc<Mutex<PlayerState>>) -> bool {
        let mut width = 0;
        let mut height = 0;
        unsafe {
            engine
                .GetNativeVideoSize(Some(&mut width), Some(&mut height))
                .ok()
        };
        let duration = unsafe { engine.GetDuration() };
        let position = unsafe { engine.GetCurrentTime() };
        let paused = unsafe { engine.IsPaused() }.as_bool();
        let ended = unsafe { engine.IsEnded() }.as_bool();
        let mut state = state.lock().unwrap();
        let mut changed = false;
        if width > 0 && height > 0 && (state.width != width || state.height != height) {
            state.width = width;
            state.height = height;
            changed = true;
        }
        if duration.is_finite() && duration >= 0. && (state.duration - duration).abs() > 0.001 {
            state.duration = duration;
            changed = true;
        }
        if position.is_finite() && position >= 0. {
            state.position = position;
        }
        if state.paused != paused {
            state.paused = paused;
            changed = true;
        }
        if state.ended != ended {
            state.ended = ended;
            changed = true;
        }
        changed
    }

    fn error_text(code: i32) -> String {
        match MF_MEDIA_ENGINE_ERR(code) {
            MF_MEDIA_ENGINE_ERR_ABORTED => "Playback was aborted".into(),
            MF_MEDIA_ENGINE_ERR_NETWORK => "Network error".into(),
            MF_MEDIA_ENGINE_ERR_DECODE => "Decode error".into(),
            MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED => "This format isn't supported".into(),
            MF_MEDIA_ENGINE_ERR_ENCRYPTED => "Encrypted video isn't supported".into(),
            _ => "Playback failed".into(),
        }
    }
}
