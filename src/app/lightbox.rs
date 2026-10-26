use std::time::Duration;

use gpui::{prelude::*, *};

use crate::{
    app::{DiscordApp, embeds},
    media::images::{self, ImageSpec},
    store::Media,
    theme::{self, WithoutTextSelection, rpx},
};

const MARGIN: f32 = 80.;
const FADE: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub(crate) struct Lightbox {
    spec: ImageSpec,
    poster: Option<ImageSpec>,
    width: u32,
    height: u32,
    /// Pixel budget the chat thumbnail was requested at, for the loading fallback.
    preview: u32,
    url: String,
    /// Key passed to `embeds::animated`; the source URL, as the chat thumbnail uses.
    key: String,
}

impl Lightbox {
    fn gifv(still: &Media, video: Option<&Media>, preview: u32) -> Self {
        let dimensions = video
            .filter(|video| video.width > 0 && video.height > 0)
            .unwrap_or(still);
        Self {
            spec: video.map_or_else(|| still.spec(), Media::video_spec),
            poster: video.map(|_| still.spec()),
            width: dimensions.width.max(1),
            height: dimensions.height.max(1),
            preview,
            url: video.unwrap_or(still).source.clone(),
            key: still.url.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Lightbox;
    use crate::{media::images::ImageSpec, store::Media};

    fn media(url: &str, width: u32, height: u32) -> Media {
        Media {
            url: url.into(),
            source: url.into(),
            width,
            height,
            animated: false,
        }
    }

    #[test]
    fn gifv_lightboxes_keep_the_video_and_a_loading_poster() {
        let still = media("https://media.discordapp.net/preview.png", 400, 300);
        let video = media("https://media.discordapp.net/clip.mp4", 720, 1280);
        let lightbox = Lightbox::gifv(&still, Some(&video), 512);
        assert_eq!(lightbox.spec, video.video_spec());
        assert_eq!(lightbox.poster, Some(still.spec()));
        assert_eq!((lightbox.width, lightbox.height), (720, 1280));
        assert_eq!(lightbox.url, video.source);
        assert_eq!(lightbox.preview, 512);
    }

    #[test]
    fn gifv_lightboxes_support_image_only_and_unknown_video_dimensions() {
        let still = media("https://media.discordapp.net/clip.gif", 400, 300);
        let image = Lightbox::gifv(&still, None, 512);
        assert_eq!(image.spec, still.spec());
        assert_eq!(image.poster, None);
        assert_eq!((image.width, image.height), (400, 300));

        let video = media("https://media.discordapp.net/clip.mp4", 0, 0);
        let fallback = Lightbox::gifv(&still, Some(&video), 512);
        assert_eq!((fallback.width, fallback.height), (400, 300));
        assert!(matches!(fallback.spec, ImageSpec::Video { .. }));
    }
}

pub(super) fn gifv_opener(
    app: &DiscordApp,
    still: &Media,
    video: Option<&Media>,
    preview: u32,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = app.entity.clone();
    let lightbox = Lightbox::gifv(still, video, preview);
    move |_, _, cx| {
        entity
            .update(cx, |app, cx| {
                app.lightbox = Some(lightbox.clone());
                cx.notify();
            })
            .ok();
    }
}

impl DiscordApp {
    pub(crate) fn open_lightbox(
        &mut self,
        spec: ImageSpec,
        (width, height): (u32, u32),
        preview: u32,
        url: String,
        cx: &mut Context<Self>,
    ) {
        self.lightbox = Some(Lightbox {
            spec,
            poster: None,
            width: width.max(1),
            height: height.max(1),
            preview,
            key: url.clone(),
            url,
        });
        cx.notify();
    }

    pub(crate) fn close_lightbox(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.lightbox.take().is_some();
        if open {
            cx.notify();
        }
        open
    }
}

pub(super) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    let lightbox = app.lightbox.as_ref()?;
    let css = |pixels: Pixels| pixels / window.rem_size() * theme::BASE_FONT_SIZE;
    let viewport = window.viewport_size();
    let max_width = (css(viewport.width) - MARGIN * 2.).max(1.);
    let max_height = (css(viewport.height) - MARGIN * 2.).max(1.);
    let (original_width, original_height) = (lightbox.width as f32, lightbox.height as f32);
    let scale = (max_width / original_width)
        .min(max_height / original_height)
        .min(1.);
    let (width, height) = (
        (original_width * scale).floor().max(1.),
        (original_height * scale).floor().max(1.),
    );
    let frames = matches!(lightbox.spec, ImageSpec::Video { .. })
        .then(|| {
            crate::media::video::resolve(&lightbox.spec, theme::image_px(width.max(height)), cx)
                .or_else(|| crate::media::video::resolve(&lightbox.spec, lightbox.preview, cx))
        })
        .flatten();
    let image = frames
        .map(|frames| {
            crate::media::animated::image(frames)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .rounded(rpx(4.))
                .id("lightbox-gifv")
                .into_any_element()
        })
        .or_else(|| {
            let spec = lightbox.poster.as_ref().unwrap_or(&lightbox.spec);
            images::resolve(spec, lightbox.width.max(lightbox.height), cx)
                .or_else(|| images::resolve(spec, lightbox.preview, cx))
                .map(|path| {
                    embeds::animated(path, &lightbox.key)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .rounded(rpx(4.))
                        .into_any_element()
                })
        });
    let close = app.entity.clone();
    let url = lightbox.url.clone();
    let animation = Animation::new(FADE).with_easing(ease_out_quint());
    Some(
        div()
            .id("lightbox-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .on_click(move |_, _, cx| {
                close.update(cx, |app, cx| app.close_lightbox(cx)).ok();
            })
            .with_animation(("lightbox-backdrop", 0u64), animation, |this, delta| {
                this.bg(hsla(0., 0., 0., 0.85 * delta))
            })
            .child(
                div()
                    .relative()
                    .w(rpx(width))
                    .h(rpx(height))
                    .child(
                        div()
                            .id("lightbox-image")
                            .w_full()
                            .h_full()
                            .rounded(rpx(4.))
                            .overflow_hidden()
                            .bg(rgb(theme::ELEVATED))
                            .without_text_selection()
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .children(image),
                    )
                    .child(
                        div()
                            .id("lightbox-open-browser")
                            .absolute()
                            .top(rpx(height + 8.))
                            .left_0()
                            .text_size(rpx(14.))
                            .line_height(rpx(18.))
                            .whitespace_nowrap()
                            .text_color(rgb(theme::MUTED))
                            .cursor_pointer()
                            .hover(|style| style.text_color(rgb(theme::STRONG)).underline())
                            .without_text_selection()
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                cx.open_url(&url);
                            })
                            .child("Open in Browser"),
                    ),
            )
            .into_any_element(),
    )
}
