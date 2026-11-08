use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_tokio::Tokio;

use crate::{
    app::{
        DiscordApp,
        menu::{checkbox, divider, panel, radio, row},
    },
    discord::{Error, StreamQuality},
    settings::{CustomStream, StreamMode},
    theme::{self, WithoutTextSelection, rpx},
};

const MENU_WIDTH: f32 = 248.;
const FLYOUT_WIDTH: f32 = 188.;
const GEAR_SIZE: f32 = 40.;
const RESOLUTIONS: [u32; 4] = [720, 1080, 1440, 0];
const FRAME_RATES: [u32; 3] = [15, 30, 60];
/// Discord's presets, best first; a preset streams at the first one the account may use.
const GAMING: [(u32, u32); 4] = [(1440, 60), (1080, 60), (720, 60), (720, 30)];
const SCREENSHARE: [(u32, u32); 2] = [(0, 15), (0, 5)];
const FALLBACK: (u32, u32) = (720, 30);
pub(super) const LOCKED: &str = "Needs Nitro or a boosted server";
pub(super) const NITRO_PINK: u32 = 0xFF73FA;
/// A failed preview upload is tried again a minute later, up to three tries in all.
const PREVIEW_ATTEMPTS: u32 = 3;
const PREVIEW_RETRY: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Flyout {
    #[default]
    None,
    Advanced,
    Resolution,
    FrameRate,
}

#[derive(Default)]
pub(super) struct StreamMenu {
    open: bool,
    flyout: Flyout,
    /// Flyouts lie outside the menu and the picker; a click there must not count as outside.
    pub(super) pointer_in_flyout: bool,
    gear_hovered: bool,
}

/// What the account and the call's server unlock, after Discord's stream quality table.
#[derive(Clone, Copy)]
struct Perks {
    /// Nitro: source resolution.
    high: bool,
    /// Nitro or Nitro Classic: 1080p, 1440p and 60fps.
    mid: bool,
    guild_tier: u8,
}

impl Perks {
    fn of(app: &DiscordApp) -> Self {
        let premium = app.store.premium_type();
        let guild_tier = app
            .voice
            .joined_call()
            .filter(|(guild_id, _)| *guild_id != 0)
            .and_then(|(guild_id, _)| app.store.guild_by_id(guild_id))
            .map_or(0, |guild| guild.premium_tier);
        Self {
            high: premium == Some(2),
            mid: matches!(premium, Some(1 | 2)),
            guild_tier,
        }
    }

    fn allows(self, mode: StreamMode, resolution: u32, fps: u32) -> bool {
        match (resolution, fps) {
            (0, 5) => mode == StreamMode::Screenshare,
            (0, _) => self.high,
            (1080 | 1440, _) => self.mid || self.guild_tier >= 2,
            (_, 60) => self.mid || self.guild_tier >= 1,
            _ => true,
        }
    }
}

fn preset_quality(perks: Perks, mode: StreamMode, custom: CustomStream) -> (u32, u32) {
    let first = |options: &[(u32, u32)]| {
        options
            .iter()
            .copied()
            .find(|&(resolution, fps)| perks.allows(mode, resolution, fps))
    };
    match mode {
        StreamMode::Gaming => first(&GAMING),
        StreamMode::Screenshare => first(&SCREENSHARE),
        StreamMode::Custom => Some((custom.resolution, custom.fps))
            .filter(|&(resolution, fps)| perks.allows(mode, resolution, fps)),
    }
    .unwrap_or(FALLBACK)
}

pub(super) fn may_stream(app: &DiscordApp, resolution: u32, fps: u32) -> bool {
    Perks::of(app).allows(StreamMode::Custom, resolution, fps)
}

pub(super) fn next_share_quality(app: &DiscordApp) -> StreamQuality {
    let (resolution, fps) = preset_quality(
        Perks::of(app),
        app.voice.stream_mode,
        app.voice.stream_custom,
    );
    StreamQuality { resolution, fps }
}

fn resolution_label(resolution: u32) -> String {
    if resolution == 0 {
        "Source".into()
    } else {
        format!("{resolution}p")
    }
}

fn mode_label(mode: StreamMode) -> (&'static str, Option<&'static str>) {
    match mode {
        StreamMode::Gaming => ("Gaming", Some("Smoother video")),
        StreamMode::Screenshare => ("Screenshare", Some("Clearer text")),
        StreamMode::Custom => ("Custom", None),
    }
}

impl DiscordApp {
    fn refresh_share_picker(&mut self, cx: &mut Context<Self>) {
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }

    fn stream_menu(&mut self) -> Option<&mut StreamMenu> {
        self.voice
            .share_picker
            .as_mut()
            .map(|picker| &mut picker.stream_menu)
    }

    fn toggle_stream_menu(&mut self, cx: &mut Context<Self>) {
        let Some(menu) = self.stream_menu() else {
            return;
        };
        *menu = StreamMenu {
            open: !menu.open,
            gear_hovered: menu.gear_hovered,
            ..StreamMenu::default()
        };
        self.refresh_share_picker(cx);
    }

    pub(crate) fn close_stream_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.stream_menu().filter(|menu| menu.open) else {
            return false;
        };
        *menu = StreamMenu {
            gear_hovered: menu.gear_hovered,
            ..StreamMenu::default()
        };
        self.refresh_share_picker(cx);
        true
    }

    fn show_stream_flyout(&mut self, flyout: Flyout, cx: &mut Context<Self>) {
        let Some(menu) = self.stream_menu().filter(|menu| menu.flyout != flyout) else {
            return;
        };
        menu.flyout = flyout;
        menu.pointer_in_flyout = false;
        self.refresh_share_picker(cx);
    }

    fn set_stream_mode(&mut self, mode: StreamMode, cx: &mut Context<Self>) {
        self.voice.stream_mode = mode;
        crate::settings::update(cx, |settings| settings.stream_mode = mode);
        self.refresh_share_picker(cx);
    }

    /// Takes effect on the next stream: the sound is set up when sharing starts.
    fn toggle_stream_audio(&mut self, cx: &mut Context<Self>) {
        let muted = !self.voice.stream_audio_muted;
        self.voice.stream_audio_muted = muted;
        crate::settings::update(cx, |settings| settings.stream_audio_muted = muted);
        self.refresh_share_picker(cx);
    }

    fn toggle_stream_preview(&mut self, cx: &mut Context<Self>) {
        let hidden = !self.voice.stream_preview_hidden;
        self.voice.stream_preview_hidden = hidden;
        crate::settings::update(cx, |settings| settings.stream_preview_hidden = hidden);
        if hidden {
            self.voice.preview_upload = None;
        }
        self.refresh_share_picker(cx);
    }

    /// Sends a still of the stream for Discord to show, unless previews are hidden. A newer
    /// still replaces an upload still waiting to retry.
    pub(crate) fn upload_stream_preview(
        &mut self,
        stream_key: String,
        thumbnail: String,
        cx: &mut Context<Self>,
    ) {
        if self.voice.stream_preview_hidden || !self.live {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.voice.preview_upload = Some(cx.spawn(async move |_, cx| {
            for attempt in 1..=PREVIEW_ATTEMPTS {
                let upload = Tokio::spawn(cx, {
                    let (session, stream_key, thumbnail) =
                        (session.clone(), stream_key.clone(), thumbnail.clone());
                    async move { session.upload_stream_preview(stream_key, thumbnail).await }
                });
                let wait = match upload.await {
                    Ok(Ok(())) => return,
                    // Discord limits how often previews change; wait at least as long as it asks.
                    Ok(Err(Error::RateLimited { retry_after })) => {
                        log::debug!("Stream preview rate limited for {retry_after:?}");
                        PREVIEW_RETRY.max(retry_after)
                    }
                    Ok(Err(error)) => {
                        log::warn!("Stream preview upload {attempt} failed: {error:?}");
                        PREVIEW_RETRY
                    }
                    Err(_) => return,
                };
                if attempt < PREVIEW_ATTEMPTS {
                    cx.background_executor().timer(wait).await;
                }
            }
        }));
    }

    fn set_custom_stream(&mut self, custom: CustomStream, cx: &mut Context<Self>) {
        self.voice.stream_mode = StreamMode::Custom;
        self.voice.stream_custom = custom;
        crate::settings::update(cx, |settings| {
            settings.stream_mode = StreamMode::Custom;
            settings.stream_custom = Some(custom);
        });
        if let Some(menu) = self.stream_menu() {
            menu.flyout = Flyout::None;
            menu.pointer_in_flyout = false;
        }
        self.refresh_share_picker(cx);
    }
}

/// The picker footer: the stream mode's summary, with the gear that opens its menu.
pub(super) fn card(app: &DiscordApp, menu: &StreamMenu) -> Div {
    let mode = app.voice.stream_mode;
    let (title, description) = mode_label(mode);
    let StreamQuality { resolution, fps } = next_share_quality(app);
    let parts = description
        .map(str::to_owned)
        .into_iter()
        .chain([resolution_label(resolution), format!("{fps}fps")]);
    let mut subtitle = div()
        .flex()
        .items_center()
        .gap(rpx(8.))
        .min_w_0()
        .text_size(rpx(12.))
        .text_color(rgb(theme::muted()));
    for (index, part) in parts.enumerate() {
        if index > 0 {
            subtitle = subtitle.child(div().size(rpx(3.)).rounded_full().bg(rgb(theme::faint())));
        }
        subtitle = subtitle.child(div().flex_none().child(part));
    }
    let toggle = app.entity.clone();
    let hover = app.entity.clone();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_3()
        .px(rpx(8.))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(rpx(2.))
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child(title),
                )
                .child(subtitle),
        )
        .child(
            div()
                .relative()
                .flex_none()
                .child(
                    div()
                        .id("stream-settings-gear")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(rpx(GEAR_SIZE))
                        .rounded(rpx(8.))
                        .cursor_pointer()
                        .bg(rgb(theme::surface_highest()))
                        .border_1()
                        .border_color(rgba(theme::border()))
                        .hover(|style| style.border_color(rgba(theme::border_strong())))
                        .on_hover(move |hovered, _, cx| {
                            hover
                                .update(cx, |app, _| {
                                    if let Some(menu) = app.stream_menu() {
                                        menu.gear_hovered = *hovered;
                                    }
                                })
                                .ok();
                        })
                        .on_click(move |_, _, cx| {
                            toggle.update(cx, |app, cx| app.toggle_stream_menu(cx)).ok();
                        })
                        .child(theme::icon("settings", 20., theme::text())),
                )
                .when(menu.open, |this| this.child(menu_panel(app, menu))),
        )
}

fn label_block(title: impl Into<SharedString>, subtitle: Option<String>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(rpx(2.))
        .child(div().truncate().child(title.into()))
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

fn shows_flyout_on_hover(app: &DiscordApp, row: Stateful<Div>, flyout: Flyout) -> Stateful<Div> {
    let entity = app.entity.clone();
    row.on_hover(move |hovered, _, cx| {
        if *hovered {
            entity
                .update(cx, |app, cx| app.show_stream_flyout(flyout, cx))
                .ok();
        }
    })
}

/// A checkbox row that stays open after the click, like Discord's menu toggles.
fn toggle_row(
    app: &DiscordApp,
    key: &'static str,
    label: &'static str,
    checked: bool,
    flyout: Flyout,
    toggle: fn(&mut DiscordApp, &mut Context<DiscordApp>),
) -> Stateful<Div> {
    let entity = app.entity.clone();
    shows_flyout_on_hover(
        app,
        row(key, true)
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                entity.update(cx, toggle).ok();
            })
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(checkbox(checked)),
        flyout,
    )
}

fn flyout_row(
    app: &DiscordApp,
    key: &'static str,
    label: String,
    flyout: Flyout,
    content: Option<Stateful<Div>>,
) -> Stateful<Div> {
    let open = content.is_some();
    let entity = app.entity.clone();
    shows_flyout_on_hover(
        app,
        row(key, true)
            .when(open, |this| this.bg(rgba(theme::selected())))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(theme::icon("chevron-right", 20., theme::muted()))
            .children(content.map(|content| {
                div()
                    .absolute()
                    .top(rpx(-9.))
                    .left(relative(1.))
                    .pl(rpx(8.))
                    .child(content.on_hover(move |hovered, _, cx| {
                        entity
                            .update(cx, |app, _| {
                                if let Some(menu) = app.stream_menu() {
                                    menu.pointer_in_flyout = *hovered;
                                }
                            })
                            .ok();
                    }))
            })),
        flyout,
    )
}

fn mode_row(app: &DiscordApp, perks: Perks, mode: StreamMode) -> Stateful<Div> {
    let (title, description) = mode_label(mode);
    let subtitle = description.map(|description| {
        let (resolution, fps) = preset_quality(perks, mode, app.voice.stream_custom);
        format!("{description} ({}, {fps}fps)", resolution_label(resolution))
    });
    let entity = app.entity.clone();
    shows_flyout_on_hover(
        app,
        row(("stream-mode", mode as u64), true)
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                entity
                    .update(cx, |app, cx| app.set_stream_mode(mode, cx))
                    .ok();
            })
            .child(label_block(title, subtitle))
            .child(radio(app.voice.stream_mode == mode)),
        Flyout::None,
    )
}

/// Radio rows for one quality setting. Picking one switches to Custom, keeping the other
/// setting at what the current mode streams, as Discord does. Options that need Nitro carry
/// its badge; ones the account can't use stay dimmed.
fn choice_flyout(
    app: &DiscordApp,
    perks: Perks,
    id: &'static str,
    options: impl Iterator<Item = (String, CustomStream, bool)>,
    selected: CustomStream,
) -> Stateful<Div> {
    panel(id, FLYOUT_WIDTH).children(options.enumerate().map(
        |(index, (label, custom, premium))| {
            let allowed = perks.allows(StreamMode::Custom, custom.resolution, custom.fps);
            let entity = app.entity.clone();
            row((id, index as u64), allowed)
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    if allowed {
                        entity
                            .update(cx, |app, cx| app.set_custom_stream(custom, cx))
                            .ok();
                    }
                })
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .gap(rpx(4.))
                        .child(div().min_w_0().truncate().child(label))
                        .when(premium, |this| {
                            this.child(theme::icon("nitro", 16., NITRO_PINK))
                        }),
                )
                .child(radio(custom == selected))
                .when(!allowed, |this| {
                    this.child(theme::hover_tooltip((id, 1000 + index as u64), LOCKED))
                })
        },
    ))
}

fn menu_panel(app: &DiscordApp, menu: &StreamMenu) -> Div {
    let perks = Perks::of(app);
    let StreamQuality { resolution, fps } = next_share_quality(app);
    let current = CustomStream { resolution, fps };
    let close = app.entity.clone();
    let open = |flyout| menu.flyout == flyout;
    let resolution_flyout = open(Flyout::Resolution).then(|| {
        choice_flyout(
            app,
            perks,
            "stream-resolution",
            RESOLUTIONS.into_iter().map(|resolution| {
                (
                    resolution_label(resolution),
                    CustomStream {
                        resolution,
                        ..current
                    },
                    resolution != 720,
                )
            }),
            current,
        )
    });
    let frame_rate_flyout = open(Flyout::FrameRate).then(|| {
        choice_flyout(
            app,
            perks,
            "stream-frame-rate",
            FRAME_RATES.into_iter().map(|fps| {
                (
                    format!("{fps}fps"),
                    CustomStream { fps, ..current },
                    fps == 60,
                )
            }),
            current,
        )
    });
    let advanced_flyout = open(Flyout::Advanced).then(|| {
        panel("stream-advanced", FLYOUT_WIDTH).child(toggle_row(
            app,
            "stream-hide-preview",
            "Hide stream preview",
            app.voice.stream_preview_hidden,
            Flyout::Advanced,
            DiscordApp::toggle_stream_preview,
        ))
    });
    div()
        .absolute()
        .right(rpx(-24.))
        .bottom(relative(1.))
        .pb(rpx(8.))
        .child(
            panel("stream-settings-menu", MENU_WIDTH)
                .on_mouse_down_out(move |_, _, cx| {
                    close
                        .update(cx, |app, cx| {
                            let keep = app
                                .stream_menu()
                                .is_some_and(|menu| menu.pointer_in_flyout || menu.gear_hovered);
                            if !keep {
                                app.close_stream_menu(cx);
                            }
                        })
                        .ok();
                })
                .child(
                    div()
                        .p(rpx(8.))
                        .text_size(rpx(14.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::muted()))
                        .without_text_selection()
                        .child("Stream Mode"),
                )
                .child(mode_row(app, perks, StreamMode::Gaming))
                .child(mode_row(app, perks, StreamMode::Screenshare))
                .child(mode_row(app, perks, StreamMode::Custom))
                .when(app.voice.stream_mode == StreamMode::Custom, |this| {
                    this.child(divider())
                        .child(flyout_row(
                            app,
                            "stream-custom-resolution",
                            "Screen Resolution".into(),
                            Flyout::Resolution,
                            resolution_flyout,
                        ))
                        .child(flyout_row(
                            app,
                            "stream-custom-frame-rate",
                            "Frame Rate".into(),
                            Flyout::FrameRate,
                            frame_rate_flyout,
                        ))
                })
                .child(divider())
                .child(toggle_row(
                    app,
                    "stream-mute-audio",
                    "Mute stream audio",
                    app.voice.stream_audio_muted,
                    Flyout::None,
                    DiscordApp::toggle_stream_audio,
                ))
                .child(flyout_row(
                    app,
                    "stream-advanced-row",
                    "Advanced".into(),
                    Flyout::Advanced,
                    advanced_flyout,
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::{CustomStream, FALLBACK, Perks, StreamMode, preset_quality};

    const FREE: Perks = Perks {
        high: false,
        mid: false,
        guild_tier: 0,
    };
    const NITRO: Perks = Perks {
        high: true,
        mid: true,
        guild_tier: 0,
    };

    #[test]
    fn presets_fall_back_to_what_the_account_may_stream() {
        let custom = CustomStream {
            resolution: 1080,
            fps: 60,
        };
        assert_eq!(
            preset_quality(NITRO, StreamMode::Gaming, custom),
            (1440, 60)
        );
        assert_eq!(preset_quality(FREE, StreamMode::Gaming, custom), (720, 30));
        assert_eq!(
            preset_quality(
                Perks {
                    guild_tier: 1,
                    ..FREE
                },
                StreamMode::Gaming,
                custom
            ),
            (720, 60)
        );
        assert_eq!(
            preset_quality(NITRO, StreamMode::Screenshare, custom),
            (0, 15)
        );
        assert_eq!(
            preset_quality(FREE, StreamMode::Screenshare, custom),
            (0, 5)
        );
        assert_eq!(
            preset_quality(NITRO, StreamMode::Custom, custom),
            (1080, 60)
        );
        assert_eq!(preset_quality(FREE, StreamMode::Custom, custom), FALLBACK);
    }

    #[test]
    fn source_resolution_needs_full_nitro() {
        let classic = Perks {
            high: false,
            mid: true,
            guild_tier: 3,
        };
        assert!(!classic.allows(StreamMode::Custom, 0, 30));
        assert!(classic.allows(StreamMode::Custom, 1440, 60));
        assert!(NITRO.allows(StreamMode::Custom, 0, 60));
    }
}
