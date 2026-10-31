use std::{collections::HashMap, sync::Arc};

use gpui::{prelude::*, *};

use super::{Sharing, Voice};
use crate::{
    app::DiscordApp,
    discord::{ScreenSource, StreamFrame},
    theme::{self, rpx},
};

const THUMBNAIL_HEIGHT: f32 = 200.;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Applications,
    Screens,
}

#[derive(Default)]
pub(super) struct SharePicker {
    tab: Tab,
    sources: Vec<ScreenSource>,
    selected: Option<usize>,
    thumbnails: HashMap<usize, Arc<RenderImage>>,
}

fn thumbnail(frame: StreamFrame) -> Option<Arc<RenderImage>> {
    let image = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
}

impl DiscordApp {
    fn open_share_picker(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let sources = session.screen_sources();
        let tab = if sources.iter().any(|source| source.is_window) {
            Tab::Applications
        } else {
            Tab::Screens
        };
        self.voice.share_picker = Some(SharePicker {
            tab,
            sources: sources.clone(),
            ..SharePicker::default()
        });
        self.views.overlays.update(cx, |_, cx| cx.notify());
        cx.spawn(async move |this, cx| {
            for (index, source) in sources.into_iter().enumerate() {
                let session = session.clone();
                let frame = cx
                    .background_executor()
                    .spawn(async move { session.screen_thumbnail(&source) })
                    .await;
                let Some(image) = frame.and_then(thumbnail) else {
                    continue;
                };
                let open = this.update(cx, |app, cx| {
                    let Some(picker) = app.voice.share_picker.as_mut() else {
                        cx.drop_image(image.clone(), None);
                        return false;
                    };
                    picker.thumbnails.insert(index, image.clone());
                    app.views.overlays.update(cx, |_, cx| cx.notify());
                    true
                });
                if !matches!(open, Ok(true)) {
                    return;
                }
            }
        })
        .detach();
    }

    pub(crate) fn close_share_picker(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picker) = self.voice.share_picker.take() else {
            return false;
        };
        for image in picker.thumbnails.into_values() {
            cx.drop_image(image, None);
        }
        self.views.overlays.update(cx, |_, cx| cx.notify());
        true
    }

    pub(crate) fn screen_button(&mut self, cx: &mut Context<Self>) {
        if self.voice.sharing == Sharing::Off {
            self.open_share_picker(cx);
            return;
        }
        if let Some(session) = &self.session {
            session.stop_share();
        }
        self.voice.sharing = Sharing::Off;
        self.sync_channel_rows();
        cx.notify();
    }

    fn start_picked_share(&mut self, cx: &mut Context<Self>) {
        let source = self.voice.share_picker.as_ref().and_then(|picker| {
            picker
                .selected
                .and_then(|index| picker.sources.get(index).cloned())
        });
        let (Some(source), Some(session)) = (source, self.session.clone().filter(|_| self.live))
        else {
            return;
        };
        session.start_share(source);
        self.voice.sharing = Sharing::Starting;
        self.voice.share_error = None;
        self.close_share_picker(cx);
        self.sync_channel_rows();
        cx.notify();
    }

    fn pick_share_source(
        &mut self,
        selected: Option<usize>,
        tab: Option<Tab>,
        cx: &mut Context<Self>,
    ) {
        let Some(picker) = self.voice.share_picker.as_mut() else {
            return;
        };
        picker.selected = selected;
        if let Some(tab) = tab {
            picker.tab = tab;
        }
        self.views.overlays.update(cx, |_, cx| cx.notify());
    }
}

fn tab(
    app: &DiscordApp,
    picker: &SharePicker,
    kind: Tab,
    icon: &str,
    label: &'static str,
) -> Stateful<Div> {
    let active = picker.tab == kind;
    let entity = app.entity.clone();
    let color = if active { theme::STRONG } else { theme::MUTED };
    div()
        .id(label)
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .h(rpx(36.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .when(active, |this| this.bg(rgb(theme::SURFACE_HIGHEST)))
        .text_size(rpx(15.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.pick_share_source(None, Some(kind), cx))
                .ok();
        })
        .child(theme::icon(icon, 18., color))
        .child(label)
}

fn source_tile(
    app: &DiscordApp,
    picker: &SharePicker,
    index: usize,
    source: &ScreenSource,
) -> Stateful<Div> {
    let selected = picker.selected == Some(index);
    let entity = app.entity.clone();
    let image = picker.thumbnails.get(&index).cloned();
    div()
        .id(("share-source", index))
        .flex()
        .flex_col()
        .gap_2()
        .w(relative(0.5))
        .p(rpx(8.))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.pick_share_source(Some(index), None, cx))
                .ok();
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .h(rpx(THUMBNAIL_HEIGHT))
                .rounded(rpx(8.))
                .overflow_hidden()
                .bg(rgb(theme::BG))
                .border_2()
                .border_color(if selected {
                    rgb(theme::BLURPLE).into()
                } else {
                    rgba(0x00000000)
                })
                .hover(|style| style.border_color(rgba(theme::BORDER_STRONG)))
                .child(match image {
                    Some(image) => img(image)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element(),
                    None => theme::icon("monitor-filled", 40., theme::FAINT).into_any_element(),
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(theme::icon(
                    if source.is_window {
                        "app-window"
                    } else {
                        "monitor-filled"
                    },
                    18.,
                    theme::MUTED,
                ))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(15.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::STRONG))
                        .child(source.name.clone()),
                ),
        )
}

pub(crate) fn render(app: &DiscordApp) -> Option<AnyElement> {
    let picker = app.voice.share_picker.as_ref()?;
    let Voice { share_error, .. } = &app.voice;
    let wanted = picker.tab == Tab::Applications;
    let close = app.entity.clone();
    let stream = app.entity.clone();
    let ready = picker.selected.is_some();
    let tiles = picker
        .sources
        .iter()
        .enumerate()
        .filter(|(_, source)| source.is_window == wanted)
        .map(|(index, source)| source_tile(app, picker, index, source));
    let note = share_error.clone().or_else(|| {
        picker
            .sources
            .is_empty()
            .then(|| "Sharing needs Windows.".to_owned())
    });
    Some(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x000000B3))
            .child(
                div()
                    .id("share-picker")
                    .flex()
                    .flex_col()
                    .w(rpx(860.))
                    .h(rpx(640.))
                    .max_w(relative(0.92))
                    .max_h(relative(0.92))
                    .p(rpx(20.))
                    .gap(rpx(16.))
                    .rounded(rpx(12.))
                    .bg(rgb(theme::SURFACE_HIGH))
                    .border_1()
                    .border_color(rgba(theme::BORDER))
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down_out(move |_, _, cx| {
                        close.update(cx, |app, cx| app.close_share_picker(cx)).ok();
                    })
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap_1()
                            .p(rpx(4.))
                            .rounded(rpx(10.))
                            .bg(rgb(theme::BG))
                            .child(tab(
                                app,
                                picker,
                                Tab::Applications,
                                "app-window",
                                "Applications",
                            ))
                            .child(tab(
                                app,
                                picker,
                                Tab::Screens,
                                "monitor-filled",
                                "Entire Screen",
                            )),
                    )
                    .child(
                        div()
                            .id("share-sources")
                            .flex()
                            .flex_wrap()
                            .content_start()
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .children(tiles),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(rpx(16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(theme::STRONG))
                                            .child("Standard"),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(rpx(14.))
                                            .text_color(rgb(theme::MUTED))
                                            .child(note.unwrap_or_else(|| "720p · 30fps".into())),
                                    ),
                            )
                            .child(
                                div()
                                    .id("share-start")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .w(rpx(112.))
                                    .h(rpx(44.))
                                    .rounded(rpx(8.))
                                    .bg(rgb(theme::BLURPLE))
                                    .text_size(rpx(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(white())
                                    .child("Stream")
                                    .map(|this| {
                                        if ready {
                                            this.cursor_pointer().on_click(move |_, _, cx| {
                                                stream
                                                    .update(cx, |app, cx| {
                                                        app.start_picked_share(cx)
                                                    })
                                                    .ok();
                                            })
                                        } else {
                                            this.opacity(0.5)
                                        }
                                    }),
                            ),
                    ),
            )
            .into_any_element(),
    )
}
