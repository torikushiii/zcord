use std::time::Duration;

use gpui::{prelude::*, *};

use crate::{
    app::{DiscordApp, embeds},
    images::{self, ImageSpec},
    theme::{self, rpx},
};

const MARGIN: f32 = 80.;
const FADE: Duration = Duration::from_millis(200);

pub(crate) struct Lightbox {
    spec: ImageSpec,
    width: u32,
    height: u32,
    /// Pixel budget the chat thumbnail was requested at, for the loading fallback.
    preview: u32,
    /// Original URL, opened by "Open in Browser".
    url: String,
    /// Key passed to `embeds::animated`; the source URL, as the chat thumbnail uses.
    key: String,
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

pub(super) fn render(
    app: &DiscordApp,
    window: &Window,
    cx: &mut Context<DiscordApp>,
) -> Option<AnyElement> {
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
    let image = images::resolve(&lightbox.spec, lightbox.width.max(lightbox.height), cx)
        .or_else(|| images::resolve(&lightbox.spec, lightbox.preview, cx));
    let close = app.entity.clone();
    let url = lightbox.url.clone();
    let key = lightbox.key.clone();
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
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .children(image.map(|path| {
                                embeds::animated(path, &key)
                                    .size_full()
                                    .object_fit(ObjectFit::Contain)
                                    .rounded(rpx(4.))
                            })),
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
