use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_tokio::Tokio;

use super::Voice;
use crate::{
    app::DiscordApp,
    media::images::{self, ImageSpec},
    theme::{self, rpx},
};

const CARD_WIDTH: f32 = 280.;
const PREVIEW_WIDTH: f32 = 248.;
const PREVIEW_HEIGHT: f32 = 142.;
const PREVIEW_BG: u32 = 0x2C2D32;
const LIVE_RED: u32 = 0xD22D39;
const WATCH_GREEN: u32 = 0x008545;
const ROW_GAP: f32 = 16.;
const PREVIEW_FADE: Duration = Duration::from_millis(150);
const CLOSE_DELAY: Duration = Duration::from_millis(300);

pub(super) struct StreamCard {
    guild_id: u64,
    channel_id: u64,
    user_id: u64,
    position: Point<Pixels>,
    preview: Option<String>,
    preview_hovered: Option<bool>,
}

pub(super) fn position(row: Bounds<Pixels>, window: &Window) -> Point<Pixels> {
    point(
        row.right() + rpx(ROW_GAP).to_pixels(window.rem_size()),
        row.top(),
    )
}

impl DiscordApp {
    pub(super) fn hover_streamer(
        &mut self,
        (guild_id, channel_id, user_id): (u64, u64, u64),
        hovered: bool,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.voice.stream_card_close = None;
        if !hovered {
            self.schedule_stream_card_close(cx);
            return;
        }
        if self
            .voice
            .stream_card
            .as_ref()
            .is_some_and(|card| card.user_id == user_id)
        {
            return;
        }
        self.voice.stream_card = Some(StreamCard {
            guild_id,
            channel_id,
            user_id,
            position,
            preview: None,
            preview_hovered: None,
        });
        self.views.overlays.update(cx, |_, cx| cx.notify());
        let Some(session) = self.session.clone().filter(|_| self.live) else {
            return;
        };
        let guild = (guild_id != 0).then_some(guild_id);
        let request = Tokio::spawn(cx, async move {
            session.stream_preview(guild, channel_id, user_id).await
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(url)) = request.await else {
                return;
            };
            this.update(cx, |app, cx| {
                if let Some(card) = app
                    .voice
                    .stream_card
                    .as_mut()
                    .filter(|card| card.user_id == user_id)
                {
                    card.preview = Some(url);
                    app.views.overlays.update(cx, |_, cx| cx.notify());
                }
            })
            .ok();
        })
        .detach();
    }

    /// The streamer row unmounts without a hover-out when the sidebar switches guilds.
    pub(crate) fn dismiss_stream_card(&mut self) {
        self.voice.stream_card = None;
        self.voice.stream_card_close = None;
        self.voice.stream_card_hovered = false;
    }

    fn schedule_stream_card_close(&mut self, cx: &mut Context<Self>) {
        self.voice.stream_card_close = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CLOSE_DELAY).await;
            this.update(cx, |app, cx| {
                // The row's leave and the card's enter arrive in either order.
                if app.voice.stream_card_hovered {
                    return;
                }
                app.voice.stream_card = None;
                app.views.overlays.update(cx, |_, cx| cx.notify());
            })
            .ok();
        }));
    }

    /// Discord drops a watch sent before the call connects, so joining defers it to `Connected`.
    pub(crate) fn watch_or_join(
        &mut self,
        (guild_id, channel_id, user_id): (u64, u64, u64),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connected = self
            .voice
            .call
            .as_ref()
            .is_some_and(|call| call.connected && call.channel_id == channel_id);
        if connected {
            self.watch_stream(user_id, Some(window), cx);
            return;
        }
        self.voice.pending_watch = self
            .store
            .can_connect_voice(guild_id, channel_id)
            .then_some((channel_id, user_id));
        self.join_voice(guild_id, channel_id, cx);
    }
}

pub(crate) fn render(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    let Voice { stream_card, .. } = &app.voice;
    let card = stream_card.as_ref()?;
    let target = (card.guild_id, card.channel_id, card.user_id);
    let in_call = app
        .voice
        .call
        .as_ref()
        .is_some_and(|call| call.channel_id == card.channel_id);
    let can_watch = in_call || app.store.can_connect_voice(card.guild_id, card.channel_id);
    let preview = card.preview.as_ref().and_then(|url| {
        images::resolve(
            &ImageSpec::Media {
                url: url.clone(),
                width: 0,
                height: 0,
                animated: false,
            },
            theme::image_px(PREVIEW_WIDTH),
            cx,
        )
    });
    let hover = app.entity.clone();
    let entity = app.entity.clone();
    let watch = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
        entity
            .update(cx, |app, cx| {
                app.voice.stream_card = None;
                app.voice.stream_card_hovered = false;
                app.watch_or_join(target, window, cx);
            })
            .ok();
    };
    let watch_preview = watch.clone();
    let hover_preview = app.entity.clone();
    let preview_hovered = card.preview_hovered;
    Some(
        deferred(
            anchored()
                .position(card.position)
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id("stream-card")
                        .flex()
                        .flex_col()
                        .w(rpx(CARD_WIDTH))
                        .p(rpx(15.))
                        .rounded(rpx(12.))
                        .bg(rgb(theme::surface_high()))
                        .border_1()
                        .border_color(rgba(theme::outline()))
                        .shadow_lg()
                        .occlude()
                        .on_hover(move |hovered, _, cx| {
                            hover
                                .update(cx, |app, cx| {
                                    app.voice.stream_card_close = None;
                                    app.voice.stream_card_hovered = *hovered;
                                    if !*hovered {
                                        app.schedule_stream_card_close(cx);
                                    }
                                })
                                .ok();
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .h(rpx(16.))
                                .mb(rpx(8.))
                                .text_size(rpx(12.))
                                .line_height(rpx(16.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(theme::faint()))
                                .child("Streaming Now")
                                .child(
                                    super::live_badge()
                                        .px(rpx(8.))
                                        .rounded_full()
                                        .bg(rgb(LIVE_RED))
                                        .text_size(rpx(12.))
                                        .font_weight(FontWeight::SEMIBOLD),
                                ),
                        )
                        .child(
                            div()
                                .id("stream-card-preview")
                                .relative()
                                .flex()
                                .items_center()
                                .justify_center()
                                .h(rpx(PREVIEW_HEIGHT))
                                .rounded(rpx(8.))
                                .overflow_hidden()
                                .bg(rgb(PREVIEW_BG))
                                .child(match preview {
                                    Some(path) => img(path)
                                        .size_full()
                                        .rounded(rpx(8.))
                                        .object_fit(ObjectFit::Contain)
                                        .into_any_element(),
                                    None => theme::icon("monitor-filled", 40., theme::faint())
                                        .into_any_element(),
                                })
                                .when(can_watch, |this| {
                                    this.cursor_pointer()
                                        .on_click(watch_preview)
                                        .on_hover(move |hovered, _, cx| {
                                            hover_preview
                                                .update(cx, |app, cx| {
                                                    if let Some(card) = &mut app.voice.stream_card {
                                                        card.preview_hovered = Some(*hovered);
                                                        app.views
                                                            .overlays
                                                            .update(cx, |_, cx| cx.notify());
                                                    }
                                                })
                                                .ok();
                                        })
                                        .children(preview_hovered.map(preview_overlay))
                                }),
                        )
                        .child(
                            div()
                                .id("stream-card-watch")
                                .flex()
                                .items_center()
                                .justify_center()
                                .gap(rpx(4.))
                                .mt(rpx(8.))
                                .h(rpx(32.))
                                .rounded(rpx(8.))
                                .bg(rgb(WATCH_GREEN))
                                .text_size(rpx(14.))
                                .line_height(rpx(18.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(white())
                                .when(!can_watch, |this| this.opacity(0.5))
                                .when(can_watch, |this| {
                                    this.cursor_pointer()
                                        .hover(|style| style.opacity(0.9))
                                        .on_click(watch)
                                })
                                .child(theme::icon("monitor-up", 16., 0xFFFFFF))
                                .child("Watch Stream"),
                        ),
                ),
        )
        .with_priority(2)
        .into_any_element(),
    )
}

fn preview_overlay(hovered: bool) -> impl IntoElement {
    let progress = move |delta: f32| if hovered { delta } else { 1. - delta };
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(rpx(8.))
        .bg(rgba(0x00000099))
        .text_size(rpx(14.))
        .line_height(rpx(18.))
        .text_color(white())
        .child("Watch Stream")
        .with_animation(
            ("stream-card-preview-fade", u64::from(hovered)),
            Animation::new(PREVIEW_FADE).with_easing(ease_out_quint()),
            move |this, delta| this.opacity(progress(delta)),
        )
}
