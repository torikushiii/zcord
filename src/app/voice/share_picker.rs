use gpui_component::scroll::ScrollableElement as _;
use std::{collections::HashMap, sync::Arc};

use gpui::{prelude::*, *};

use super::{Sharing, Voice, share_panel::ShareInfo, stream_quality};
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
    thumbnails: HashMap<usize, Arc<RenderImage>>,
    icons: HashMap<usize, Arc<RenderImage>>,
    pub(super) stream_menu: stream_quality::StreamMenu,
    /// Fills in icons and thumbnails; closing the picker drops it, so a reopened picker never
    /// receives an older one's images.
    _loading: Option<Task<()>>,
}

fn thumbnail(frame: StreamFrame) -> Option<Arc<RenderImage>> {
    let image = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
}

impl DiscordApp {
    pub(super) fn open_share_picker(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        // A picker already open would be replaced without its images being freed.
        self.close_share_picker(cx);
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
        let loading = cx.spawn(async move |this, cx| {
            // Icons come first: reading one is quick, while a thumbnail waits for a captured frame.
            let icons = cx
                .background_executor()
                .spawn({
                    let session = session.clone();
                    let sources = sources.clone();
                    async move {
                        sources
                            .iter()
                            .map(|source| session.screen_icon(source))
                            .collect::<Vec<_>>()
                    }
                })
                .await;
            let icons: Vec<_> = icons
                .into_iter()
                .enumerate()
                .filter_map(|(index, frame)| Some((index, thumbnail(frame?)?)))
                .collect();
            let open = this.update(cx, |app, cx| {
                let Some(picker) = app.voice.share_picker.as_mut() else {
                    for (_, image) in &icons {
                        cx.drop_image(image.clone(), None);
                    }
                    return false;
                };
                picker.icons.extend(icons.iter().cloned());
                app.views.overlays.update(cx, |_, cx| cx.notify());
                true
            });
            if !matches!(open, Ok(true)) {
                return;
            }
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
        });
        if let Some(picker) = self.voice.share_picker.as_mut() {
            picker._loading = Some(loading);
        }
    }

    pub(crate) fn close_share_picker(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picker) = self.voice.share_picker.take() else {
            return false;
        };
        for image in picker
            .thumbnails
            .into_values()
            .chain(picker.icons.into_values())
        {
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
        self.stop_share(cx);
    }

    fn start_share_at(&mut self, index: usize, cx: &mut Context<Self>) {
        let source = self
            .voice
            .share_picker
            .as_ref()
            .and_then(|picker| picker.sources.get(index).cloned());
        let (Some(source), Some(session)) = (source, self.session.clone().filter(|_| self.live))
        else {
            return;
        };
        // The panel keeps the picker's icon, so closing the picker mustn't drop it.
        let icon = self
            .voice
            .share_picker
            .as_mut()
            .and_then(|picker| picker.icons.remove(&index));
        let changing = self.voice.sharing != Sharing::Off;
        let previous = changing.then(|| self.voice.share_info.take()).flatten();
        if let Some(old) = previous.as_ref().and_then(|info| info.icon.clone()) {
            cx.drop_image(old, None);
        }
        let info = match previous {
            Some(previous) => ShareInfo {
                name: source.name.clone(),
                is_window: source.is_window,
                icon,
                ..previous
            },
            None => ShareInfo {
                name: source.name.clone(),
                is_window: source.is_window,
                icon,
                quality: stream_quality::next_share_quality(self),
                shares_audio: !self.voice.stream_audio_muted,
                viewers: Vec::new(),
            },
        };
        if changing {
            session.change_share(Some(source), None);
        } else {
            self.clear_share_info(cx);
            session.start_share(source, info.quality, info.shares_audio);
            self.voice.sharing = Sharing::Starting;
        }
        self.voice.share_info = Some(info);
        self.voice.share_error = None;
        self.close_share_picker(cx);
        self.sync_channel_rows();
        cx.notify();
    }

    fn set_share_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        let Some(picker) = self.voice.share_picker.as_mut() else {
            return;
        };
        picker.tab = tab;
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
    let color = if active {
        theme::strong()
    } else {
        theme::muted()
    };
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
        .when(active, |this| this.bg(rgb(theme::surface_highest())))
        .text_size(rpx(15.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.set_share_tab(kind, cx))
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
    let entity = app.entity.clone();
    let image = picker.thumbnails.get(&index).cloned();
    let group = SharedString::from(format!("share-source-{index}"));
    // Like Discord, hovering a source dims it behind a Share Screen button, and a click
    // anywhere on it starts the stream.
    div()
        .id(("share-source", index))
        .group(group.clone())
        .flex()
        .flex_col()
        .gap_2()
        .w(relative(0.5))
        .p(rpx(8.))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            entity
                .update(cx, |app, cx| app.start_share_at(index, cx))
                .ok();
        })
        .child(
            div()
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .h(rpx(THUMBNAIL_HEIGHT))
                .rounded(rpx(8.))
                .overflow_hidden()
                .bg(rgb(theme::bg()))
                .border_2()
                .border_color(rgba(0x00000000))
                .group_hover(group.clone(), |style| {
                    style.border_color(rgba(theme::border_strong()))
                })
                .child(match image {
                    Some(image) => img(image)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element(),
                    None => theme::icon("monitor-filled", 40., theme::faint()).into_any_element(),
                })
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(rgba(0x00000099))
                        .opacity(0.)
                        .group_hover(group, |style| style.opacity(1.))
                        .child(
                            div()
                                .px(rpx(28.))
                                .py(rpx(12.))
                                .rounded(rpx(8.))
                                .bg(white())
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(black())
                                .child("Share Screen"),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(match picker.icons.get(&index).cloned() {
                    Some(icon) => img(icon)
                        .size(rpx(20.))
                        .flex_none()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element(),
                    None => theme::icon(
                        if source.is_window {
                            "app-window"
                        } else {
                            "monitor-filled"
                        },
                        20.,
                        theme::muted(),
                    )
                    .into_any_element(),
                })
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(rpx(15.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme::strong()))
                        .child(source.name.clone()),
                ),
        )
}

pub(crate) fn render(app: &DiscordApp) -> Option<AnyElement> {
    let picker = app.voice.share_picker.as_ref()?;
    let Voice { share_error, .. } = &app.voice;
    let wanted = picker.tab == Tab::Applications;
    let close = app.entity.clone();
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
                    .bg(rgb(theme::surface_high()))
                    .border_1()
                    .border_color(rgba(theme::border()))
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down_out(move |_, _, cx| {
                        close
                            .update(cx, |app, cx| {
                                let in_flyout = app
                                    .voice
                                    .share_picker
                                    .as_ref()
                                    .is_some_and(|picker| picker.stream_menu.pointer_in_flyout);
                                if !in_flyout {
                                    app.close_share_picker(cx);
                                }
                            })
                            .ok();
                    })
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .gap_1()
                            .p(rpx(4.))
                            .rounded(rpx(10.))
                            .bg(rgb(theme::bg()))
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
                            .overflow_y_scrollbar()
                            .children(tiles),
                    )
                    .children(note.map(|note| {
                        div()
                            .flex_none()
                            .truncate()
                            .text_size(rpx(14.))
                            .text_color(rgb(theme::muted()))
                            .child(note)
                    }))
                    .child(stream_quality::card(app, &picker.stream_menu)),
            )
            .into_any_element(),
    )
}
