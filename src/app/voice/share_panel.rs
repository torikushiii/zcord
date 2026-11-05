use std::sync::Arc;

use gpui::{prelude::*, *};

use super::{
    Menu, MenuKind, Sharing,
    stream_quality::{self, LOCKED, NITRO_PINK},
};
use crate::{
    app::{
        DiscordApp,
        menu::{checkbox, divider, panel, radio, row},
    },
    discord::StreamQuality,
    theme::{self, rpx},
};

const MENU_WIDTH: f32 = 220.;
const FLYOUT_WIDTH: f32 = 200.;
const FRAME_RATES: [u32; 3] = [15, 30, 60];
const RESOLUTIONS: [u32; 5] = [480, 720, 1080, 1440, 0];

const ICON: f32 = 32.;
const SPECTATOR_AVATAR: f32 = 24.;
/// Avatars past this many fold into a "+N".
const SPECTATORS_SHOWN: usize = 8;

/// What the user is streaming, for the panel above Voice Connected.
pub(super) struct ShareInfo {
    pub(super) name: String,
    pub(super) is_window: bool,
    pub(super) icon: Option<Arc<RenderImage>>,
    pub(super) quality: StreamQuality,
    pub(super) shares_audio: bool,
    /// Everyone watching, from Discord's `STREAM_UPDATE`; it never lists the streamer.
    pub(super) viewers: Vec<u64>,
}

impl DiscordApp {
    pub(super) fn clear_share_info(&mut self, cx: &mut Context<Self>) {
        if let Some(icon) = self.voice.share_info.take().and_then(|info| info.icon) {
            cx.drop_image(icon, None);
        }
    }

    pub(super) fn stop_share(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            session.stop_share();
        }
        self.voice.sharing = Sharing::Off;
        self.clear_share_info(cx);
        if self
            .voice
            .menu
            .as_ref()
            .is_some_and(|menu| menu.kind == MenuKind::Stream)
        {
            self.close_menu(cx);
        }
        self.sync_channel_rows();
        cx.notify();
    }

    fn change_stream(&mut self, cx: &mut Context<Self>) {
        self.close_menu(cx);
        self.open_share_picker(cx);
    }

    fn set_live_quality(&mut self, quality: StreamQuality, cx: &mut Context<Self>) {
        let Some(info) = self.voice.share_info.as_mut() else {
            return;
        };
        if info.quality == quality {
            return;
        }
        info.quality = quality;
        if let Some(session) = &self.session {
            session.change_share(None, Some(quality));
        }
        self.views.channel_list.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn toggle_live_audio(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.voice.share_info.as_mut() else {
            return;
        };
        info.shares_audio = !info.shares_audio;
        if let Some(session) = &self.session {
            session.set_share_audio(info.shares_audio);
        }
        cx.notify();
    }

    fn show_quality_flyout(&mut self, open: bool, cx: &mut Context<Self>) {
        let Some(menu) = self.voice.menu.as_mut().filter(|menu| menu.flyout != open) else {
            return;
        };
        menu.flyout = open;
        menu.pointer_in_flyout = false;
        cx.notify();
    }
}

/// "1440p 60FPS", or "Source 15FPS" when the picture keeps the window's size.
fn quality_label(quality: StreamQuality) -> String {
    let resolution = match quality.resolution {
        0 => "Source".to_owned(),
        height => format!("{height}p"),
    };
    format!("{resolution} {}FPS", quality.fps)
}

/// Discord only spells out the quality, with its Nitro mark, when it isn't the standard
/// 720p at 30fps.
fn shows_quality(quality: StreamQuality) -> bool {
    quality != StreamQuality::STANDARD
}

pub(super) fn render(app: &DiscordApp, guild_id: u64, cx: &App) -> Option<impl IntoElement> {
    let info = app.voice.share_info.as_ref()?;
    let icon = match info.icon.clone() {
        Some(icon) => img(icon)
            .size_full()
            .rounded(rpx(8.))
            .object_fit(ObjectFit::Cover)
            .into_any_element(),
        None => theme::icon(
            if info.is_window {
                "app-window"
            } else {
                "monitor-filled"
            },
            24.,
            theme::muted(),
        )
        .into_any_element(),
    };
    let icon = div()
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(ICON))
        .child(icon)
        .child(
            div()
                .absolute()
                .right(rpx(-3.))
                .bottom(rpx(-3.))
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(16.))
                .rounded(rpx(4.))
                .bg(rgb(theme::elevated()))
                .child(theme::icon("video-2", 12., theme::strong())),
        );
    let quality = shows_quality(info.quality).then(|| {
        div()
            .flex()
            .items_center()
            .gap(rpx(2.))
            .min_w_0()
            .text_size(rpx(12.))
            .line_height(rpx(16.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(NITRO_PINK))
            .child(theme::icon("nitro", 14., NITRO_PINK))
            .child(div().truncate().child(quality_label(info.quality)))
    });
    let stop = app.entity.clone();
    let stop = div()
        .id("share-panel-stop")
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(32.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme::hover())))
        .on_click(move |_, _, cx| {
            stop.update(cx, |app, cx| app.stop_share(cx)).ok();
        })
        .child(theme::icon("monitor-x", 20., theme::muted()))
        .child(theme::hover_tooltip(
            "share-panel-stop-tip",
            "Stop Streaming",
        ));
    let header = div()
        .flex()
        .items_center()
        .gap(rpx(12.))
        .child(icon)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(rpx(16.))
                        .line_height(rpx(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::strong()))
                        .child(info.name.clone()),
                )
                .children(quality),
        )
        .child(stop);
    let spectators = if info.viewers.is_empty() {
        div()
            .text_size(rpx(14.))
            .line_height(rpx(18.))
            .text_color(rgb(theme::muted()))
            .child("No spectators")
            .into_any_element()
    } else {
        let hidden = info.viewers.len().saturating_sub(SPECTATORS_SHOWN);
        div()
            .flex()
            .items_center()
            .gap(rpx(4.))
            .h(rpx(SPECTATOR_AVATAR))
            .children(info.viewers.iter().take(SPECTATORS_SHOWN).map(|&user_id| {
                theme::undecorated_member_avatar(
                    &app.store,
                    guild_id,
                    user_id,
                    SPECTATOR_AVATAR,
                    cx,
                )
            }))
            .when(hidden > 0, |this| {
                this.child(
                    div()
                        .pl(rpx(2.))
                        .text_size(rpx(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(theme::muted()))
                        .child(format!("+{hidden}")),
                )
            })
            .into_any_element()
    };
    Some(
        div()
            .flex()
            .flex_col()
            .gap(rpx(8.))
            .p(rpx(8.))
            .pb(rpx(10.))
            .border_b_1()
            .border_color(rgba(theme::border()))
            .child(header)
            .child(
                div()
                    .text_size(rpx(16.))
                    .line_height(rpx(20.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(theme::strong()))
                    .child(format!("SPECTATORS - {}", info.viewers.len())),
            )
            .child(spectators),
    )
}

fn closes_flyout_row(app: &DiscordApp, key: &'static str) -> Stateful<Div> {
    let entity = app.entity.clone();
    row(key, true).on_hover(move |hovered, _, cx| {
        if *hovered {
            entity
                .update(cx, |app, cx| app.show_quality_flyout(false, cx))
                .ok();
        }
    })
}

pub(super) fn live_menu(app: &DiscordApp, menu: &Menu) -> Stateful<Div> {
    let info = app.voice.share_info.as_ref();
    let quality = info.map_or(StreamQuality::STANDARD, |info| info.quality);
    let audio = info.is_some_and(|info| info.shares_audio);
    let (stop, change, toggle, hover) = (
        app.entity.clone(),
        app.entity.clone(),
        app.entity.clone(),
        app.entity.clone(),
    );
    let flyout = menu.flyout.then(|| {
        let entity = app.entity.clone();
        div()
            .id("stream-live-quality-flyout")
            .absolute()
            // The menu sits low, above the voice panel, so the flyout grows upward from
            // the row instead of running off the bottom of the window.
            .bottom(rpx(-9.))
            .left(relative(1.))
            .pl(rpx(8.))
            // On the panel itself: it occludes, so a hover on this wrapper would never fire
            // over it, and a click on an option would count as outside the menu and close it.
            .child(
                quality_flyout(app, quality).on_hover(move |hovered, _, cx| {
                    entity
                        .update(cx, |app, _| {
                            if let Some(menu) = app.voice.menu.as_mut() {
                                menu.pointer_in_flyout = *hovered;
                            }
                        })
                        .ok();
                }),
            )
    });
    panel("stream-live-menu", MENU_WIDTH)
        .child(
            closes_flyout_row(app, "stream-live-stop")
                .text_color(rgb(theme::RED))
                .on_click(move |_, _, cx| {
                    stop.update(cx, |app, cx| app.stop_share(cx)).ok();
                })
                .child(theme::icon("monitor-x", 20., theme::RED))
                .child("Stop Streaming"),
        )
        .child(
            closes_flyout_row(app, "stream-live-change")
                .on_click(move |_, _, cx| {
                    change.update(cx, |app, cx| app.change_stream(cx)).ok();
                })
                .child(theme::icon("monitor-share", 20., theme::text()))
                .child("Change Stream"),
        )
        .child(
            row("stream-live-quality", true)
                .when(menu.flyout, |this| this.bg(rgba(theme::selected())))
                .on_hover(move |hovered, _, cx| {
                    if *hovered {
                        hover
                            .update(cx, |app, cx| app.show_quality_flyout(true, cx))
                            .ok();
                    }
                })
                .child(div().flex_1().min_w_0().truncate().child("Stream Quality"))
                .child(theme::icon("chevron-right", 20., theme::muted()))
                .children(flyout),
        )
        .child(
            closes_flyout_row(app, "stream-live-audio")
                .on_click(move |_, _, cx| {
                    toggle.update(cx, |app, cx| app.toggle_live_audio(cx)).ok();
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child("Share Stream Audio"),
                )
                .child(checkbox(audio)),
        )
}

fn flyout_heading(title: &'static str) -> Div {
    div()
        .px(rpx(8.))
        .pt(rpx(8.))
        .pb(rpx(4.))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::muted()))
        .child(title)
}

/// Frame rate and resolution radios that switch the running stream straight away. Options the
/// account can't use stay dimmed.
fn quality_flyout(app: &DiscordApp, current: StreamQuality) -> Stateful<Div> {
    let option = |key: (&'static str, u64), label: String, choice: StreamQuality, selected| {
        let allowed = stream_quality::may_stream(app, choice.resolution, choice.fps);
        let entity = app.entity.clone();
        row(key, allowed)
            .on_click(move |_, _, cx| {
                if allowed {
                    entity
                        .update(cx, |app, cx| app.set_live_quality(choice, cx))
                        .ok();
                }
            })
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(radio(selected))
            .when(!allowed, |this| {
                this.child(theme::hover_tooltip((key.0, 1000 + key.1), LOCKED))
            })
    };
    panel("stream-live-quality", FLYOUT_WIDTH)
        .child(flyout_heading("Frame Rate"))
        .children(FRAME_RATES.into_iter().map(|fps| {
            option(
                ("stream-live-fps", u64::from(fps)),
                format!("{fps} FPS"),
                StreamQuality { fps, ..current },
                fps == current.fps,
            )
        }))
        .child(divider())
        .child(flyout_heading("Resolution"))
        .children(RESOLUTIONS.into_iter().map(|resolution| {
            let label = match resolution {
                0 => "Source".to_owned(),
                height => format!("{height}p"),
            };
            option(
                ("stream-live-resolution", u64::from(resolution)),
                label,
                StreamQuality {
                    resolution,
                    ..current
                },
                resolution == current.resolution,
            )
        }))
}

#[cfg(test)]
mod tests {
    use super::{StreamQuality, quality_label, shows_quality};

    #[test]
    fn labels_presets_and_source() {
        let quality = |resolution, fps| StreamQuality { resolution, fps };
        assert_eq!(quality_label(quality(1440, 60)), "1440p 60FPS");
        assert_eq!(quality_label(quality(0, 15)), "Source 15FPS");
        assert!(!shows_quality(StreamQuality::STANDARD));
        assert!(shows_quality(quality(480, 15)));
    }
}
