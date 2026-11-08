mod menus;
mod toolbar;

use menus::context_menu;
use toolbar::{author_chip, gallery_button, toolbar};

use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_tokio::Tokio;

use crate::{
    app::{
        DiscordApp, chat,
        display_name::{self, Surface},
        messages::embeds,
        titlebar,
    },
    media::images::{self, ImageSpec},
    store::{Attachment, Media, Message},
    theme::{self, WithoutTextSelection, rpx},
};

const MARGIN: f32 = 80.;
const FADE: Duration = Duration::from_millis(200);
const BAR_HEIGHT: f32 = 44.;
const SURFACE: u32 = 0x1A1A1E;
const ZOOM: f32 = 2.;
const ZOOM_DURATION: Duration = Duration::from_millis(240);
const DRAG_THRESHOLD: f32 = 3.;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Menu {
    #[default]
    Closed,
    Open,
    Details,
}

#[derive(Clone)]
pub(crate) struct Lightbox {
    spec: ImageSpec,
    animated: bool,
    poster: Option<ImageSpec>,
    width: u32,
    height: u32,
    /// Pixel budget the chat thumbnail was requested at, for the loading fallback.
    preview: u32,
    url: String,
    /// Key passed to `embeds::animated`; the source URL, as the chat thumbnail uses.
    key: String,
    zoomed: bool,
    menu: Menu,
    context_menu: Option<Point<Pixels>>,
    /// Fetched when the details open, for media Discord doesn't report a size for.
    size: Option<u64>,
    scroll: ScrollHandle,
    drag: Option<Drag>,
    motion: Option<ZoomMotion>,
    gallery: Option<(Rc<[LightboxEntry]>, usize)>,
}

#[derive(Clone)]
pub(crate) struct LightboxEntry {
    pub(crate) spec: ImageSpec,
    pub(crate) animated: bool,
    pub(crate) size: (u32, u32),
    pub(crate) preview: u32,
    pub(crate) url: String,
}

#[derive(Clone, Copy)]
struct ZoomMotion {
    /// Unique per zoom, so each one replays its animation.
    id: u64,
    from: Bounds<Pixels>,
    to: Bounds<Pixels>,
}

#[derive(Clone, Copy)]
struct Drag {
    start: Point<Pixels>,
    offset: Point<Pixels>,
    moved: bool,
}

/// Where the open image came from. Attachments also give the file itself; embeds give the
/// proxied copy, which stays on Discord's CDN whatever host the link points at.
#[derive(Clone, Copy)]
struct Origin<'a> {
    message: &'a Message,
    channel_id: u64,
    attachment: Option<&'a Attachment>,
    media: Option<&'a Media>,
}

impl Lightbox {
    fn gifv(still: &Media, video: Option<&Media>, preview: u32) -> Self {
        let dimensions = video
            .filter(|video| video.width > 0 && video.height > 0)
            .unwrap_or(still);
        Self {
            spec: video.map_or_else(|| still.spec(), Media::video_spec),
            animated: video.is_none() && embeds::animated_media(still),
            poster: video.map(|_| still.spec()),
            width: dimensions.width.max(1),
            height: dimensions.height.max(1),
            preview,
            url: video.unwrap_or(still).source.clone(),
            key: still.url.clone(),
            zoomed: false,
            menu: Menu::Closed,
            context_menu: None,
            size: None,
            scroll: ScrollHandle::new(),
            drag: None,
            motion: None,
            gallery: None,
        }
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
        animated: bool,
        (width, height): (u32, u32),
        preview: u32,
        url: String,
        cx: &mut Context<Self>,
    ) {
        let entry = LightboxEntry {
            spec,
            animated,
            size: (width, height),
            preview,
            url,
        };
        self.show_lightbox_entry(entry, None, cx);
    }

    pub(crate) fn open_lightbox_gallery(
        &mut self,
        entries: Rc<[LightboxEntry]>,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = entries.get(index).cloned() else {
            return;
        };
        self.show_lightbox_entry(entry, Some((entries, index)), cx);
    }

    fn show_lightbox_entry(
        &mut self,
        entry: LightboxEntry,
        gallery: Option<(Rc<[LightboxEntry]>, usize)>,
        cx: &mut Context<Self>,
    ) {
        self.lightbox = Some(Lightbox {
            spec: entry.spec,
            animated: entry.animated,
            poster: None,
            width: entry.size.0.max(1),
            height: entry.size.1.max(1),
            preview: entry.preview,
            key: entry.url.clone(),
            url: entry.url,
            zoomed: false,
            menu: Menu::Closed,
            context_menu: None,
            size: None,
            scroll: ScrollHandle::new(),
            drag: None,
            motion: None,
            gallery,
        });
        cx.notify();
    }

    fn step_lightbox(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some((entries, index)) = self.lightbox.as_ref().and_then(|box_| box_.gallery.clone())
        else {
            return;
        };
        let count = entries.len();
        if count < 2 {
            return;
        }
        let next = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        self.open_lightbox_gallery(entries, next, cx);
    }

    pub(crate) fn close_lightbox(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.lightbox.take().is_some();
        if open {
            self.stream.hovered_control = None;
            cx.notify();
        }
        open
    }

    fn set_lightbox_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
        if let Some(lightbox) = self.lightbox.as_mut().filter(|box_| box_.menu != menu) {
            lightbox.menu = menu;
            cx.notify();
        }
        if menu == Menu::Details {
            self.load_lightbox_size(cx);
        }
    }

    fn open_lightbox_context_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(lightbox) = self.lightbox.as_mut() {
            lightbox.menu = Menu::Closed;
            lightbox.context_menu = Some(position);
            cx.notify();
        }
    }

    pub(crate) fn close_lightbox_context_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self
            .lightbox
            .as_mut()
            .and_then(|lightbox| lightbox.context_menu.take())
            .is_some();
        if open {
            cx.notify();
        }
        open
    }

    fn load_lightbox_size(&self, cx: &mut Context<Self>) {
        let Some(lightbox) = self.lightbox.as_ref().filter(|box_| box_.size.is_none()) else {
            return;
        };
        let found = origin(self, lightbox);
        let attached = found.is_some_and(|origin| origin.attachment.is_some());
        let Some(service) = images::download_service(cx).filter(|_| !attached) else {
            return;
        };
        let url = lightbox.url.clone();
        let Some(target) = self.lightbox_original() else {
            return;
        };
        let request = Tokio::spawn(cx, async move { service.fetch_size(&target).await });
        cx.spawn(async move |this, cx| {
            let size = request.await.ok().flatten();
            this.update(cx, |app, cx| {
                if let Some(lightbox) = app.lightbox.as_mut().filter(|box_| box_.url == url) {
                    lightbox.size = size;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Zooms around `focus`, a window position over the image, or around its centre.
    fn toggle_lightbox_zoom(
        &mut self,
        focus: Option<Point<Pixels>>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let animate = !cx.reduce_motion();
        let Some(lightbox) = self.lightbox.as_mut().filter(|box_| box_.motion.is_none()) else {
            return;
        };
        let (width, height) = fitted_size(lightbox, window);
        let unit = rpx(1.).to_pixels(window.rem_size());
        let viewport = window.viewport_size();
        let center = point(viewport.width / 2., viewport.height / 2.);
        let fitted_size = size(unit * width, unit * height);
        let fitted = Bounds::new(
            center - point(fitted_size.width / 2., fitted_size.height / 2.),
            fitted_size,
        );
        let zoomed_size = size(fitted_size.width * ZOOM, fitted_size.height * ZOOM);
        let margin = unit * MARGIN;
        let zoomed = |offset| {
            Bounds::new(
                zoomed_origin(zoomed_size, margin, offset, viewport),
                zoomed_size,
            )
        };
        let (from, to) = if lightbox.zoomed {
            (zoomed(lightbox.scroll.offset()), fitted)
        } else {
            let (image_x, image_y) = focus.map_or((0.5, 0.5), |focus| {
                (
                    0.5 + (focus.x - center.x) / fitted_size.width,
                    0.5 + (focus.y - center.y) / fitted_size.height,
                )
            });
            let wanted = point(
                center.x - margin - zoomed_size.width * image_x,
                center.y - margin - zoomed_size.height * image_y,
            );
            let offset = clamp_offset(wanted, zoomed_size, margin, viewport);
            lightbox.scroll.set_offset(offset);
            (fitted, zoomed(offset))
        };
        lightbox.zoomed = !lightbox.zoomed;
        lightbox.drag = None;
        if animate {
            static MOTIONS: AtomicU64 = AtomicU64::new(0);
            let id = MOTIONS.fetch_add(1, Ordering::Relaxed) + 1;
            lightbox.motion = Some(ZoomMotion { id, from, to });
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(ZOOM_DURATION).await;
                this.update(cx, |app, cx| {
                    if let Some(lightbox) = app
                        .lightbox
                        .as_mut()
                        .filter(|box_| box_.motion.is_some_and(|motion| motion.id == id))
                    {
                        lightbox.motion = None;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        cx.notify();
    }

    fn start_lightbox_drag(&mut self, start: Point<Pixels>) {
        if let Some(lightbox) = self
            .lightbox
            .as_mut()
            .filter(|box_| box_.zoomed && box_.motion.is_none())
        {
            lightbox.drag = Some(Drag {
                start,
                offset: lightbox.scroll.offset(),
                moved: false,
            });
        }
    }

    fn drag_lightbox(&mut self, position: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        let threshold = rpx(DRAG_THRESHOLD).to_pixels(window.rem_size());
        let Some(lightbox) = &mut self.lightbox else {
            return;
        };
        let Some(drag) = &mut lightbox.drag else {
            return;
        };
        let delta = position - drag.start;
        drag.moved |= delta.x.abs() > threshold || delta.y.abs() > threshold;
        lightbox.scroll.set_offset(drag.offset + delta);
        cx.notify();
    }

    fn click_lightbox_image(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let dragged = self
            .lightbox
            .as_mut()
            .and_then(|lightbox| lightbox.drag.take())
            .is_some_and(|drag| drag.moved);
        if !dragged {
            self.toggle_lightbox_zoom(Some(position), window, cx);
        }
    }

    /// The URL holding the original file, on a host zcord is allowed to fetch from.
    fn lightbox_original(&self) -> Option<String> {
        let lightbox = self.lightbox.as_ref()?;
        let media = origin(self, lightbox).and_then(|origin| origin.media);
        Some(media.map_or_else(|| lightbox.url.clone(), |media| media.url.clone()))
    }

    /// The original file when it can be fetched, otherwise the full-size cached copy.
    fn lightbox_bytes(&self, cx: &mut Context<Self>) -> Option<Task<Option<Vec<u8>>>> {
        let lightbox = self.lightbox.as_ref()?;
        let service = images::download_service(cx)?;
        let url = self.lightbox_original()?;
        let spec = lightbox.spec.clone();
        let size = lightbox.width.max(lightbox.height);
        let request = Tokio::spawn(cx, async move {
            match service.fetch_original(&url).await {
                Some(bytes) => Some(bytes),
                None => {
                    let path = service.load(&spec, size).await?;
                    tokio::fs::read(path).await.ok()
                }
            }
        });
        Some(cx.background_spawn(async move { request.await.ok().flatten() }))
    }

    fn save_lightbox_image(&mut self, filename: String, cx: &mut Context<Self>) {
        let Some(bytes) = self.lightbox_bytes(cx) else {
            return;
        };
        let directory = dirs::download_dir().unwrap_or_default();
        let target = cx.prompt_for_new_path(&directory, Some(&filename));
        cx.background_spawn(async move {
            let Ok(Ok(Some(target))) = target.await else {
                return;
            };
            let Some(bytes) = bytes.await else {
                log::warn!("Could not load the image to save it");
                return;
            };
            if let Err(error) = std::fs::write(target, bytes) {
                log::warn!("Could not save the image: {error}");
            }
        })
        .detach();
    }

    fn copy_lightbox_image(&mut self, cx: &mut Context<Self>) {
        let Some(bytes) = self.lightbox_bytes(cx) else {
            return;
        };
        cx.spawn(async move |_, cx| {
            let image = cx
                .background_spawn(async move { clipboard_image(bytes.await?) })
                .await;
            let Some(image) = image else {
                log::warn!("Could not copy the image");
                return;
            };
            cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_image(&image)));
        })
        .detach();
    }
}

/// GIFs stay GIFs so they paste animated; the clipboard adds its own PNG beside them.
fn clipboard_image(bytes: Vec<u8>) -> Option<Image> {
    if image::guess_format(&bytes).ok()? == image::ImageFormat::Gif {
        return Some(Image::from_bytes(ImageFormat::Gif, bytes));
    }
    let image = image::load_from_memory(&bytes).ok()?;
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(Image::from_bytes(ImageFormat::Png, png.into_inner()))
}

fn zoomed_origin(
    image: Size<Pixels>,
    margin: Pixels,
    offset: Point<Pixels>,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let axis = |image: Pixels, offset: Pixels, view: Pixels| {
        if image + margin * 2. <= view {
            (view - image) / 2.
        } else {
            offset + margin
        }
    };
    point(
        axis(image.width, offset.x, viewport.width),
        axis(image.height, offset.y, viewport.height),
    )
}

fn clamp_offset(
    offset: Point<Pixels>,
    image: Size<Pixels>,
    margin: Pixels,
    viewport: Size<Pixels>,
) -> Point<Pixels> {
    let axis = |offset: Pixels, image: Pixels, view: Pixels| {
        let lowest = view - image - margin * 2.;
        if lowest >= px(0.) {
            px(0.)
        } else if offset < lowest {
            lowest
        } else if offset > px(0.) {
            px(0.)
        } else {
            offset
        }
    };
    point(
        axis(offset.x, image.width, viewport.width),
        axis(offset.y, image.height, viewport.height),
    )
}

fn fitted_size(lightbox: &Lightbox, window: &Window) -> (f32, f32) {
    let css = |pixels: Pixels| pixels / window.rem_size() * theme::BASE_FONT_SIZE;
    let viewport = window.viewport_size();
    let max_width = (css(viewport.width) - MARGIN * 2.).max(1.);
    let max_height = (css(viewport.height) - MARGIN * 2.).max(1.);
    let (width, height) = (lightbox.width as f32, lightbox.height as f32);
    let scale = (max_width / width).min(max_height / height).min(1.);
    (
        (width * scale).floor().max(1.),
        (height * scale).floor().max(1.),
    )
}

fn origin<'a>(app: &'a DiscordApp, lightbox: &Lightbox) -> Option<Origin<'a>> {
    let channels = app.channel().into_iter().chain(app.thread_pane_channel());
    channels.into_iter().find_map(|channel| {
        channel.messages.iter().find_map(|message| {
            let attachment = message
                .attachments
                .iter()
                .find(|attachment| attachment.url == lightbox.url);
            let media = message.embeds.iter().find_map(|embed| {
                embed
                    .images
                    .iter()
                    .chain(&embed.thumbnail)
                    .chain(&embed.video)
                    .find(|media| media.source == lightbox.url)
            });
            (attachment.is_some() || media.is_some()).then_some(Origin {
                message,
                channel_id: channel.id,
                attachment,
                media,
            })
        })
    })
}

fn file_name(lightbox: &Lightbox, attachment: Option<&Attachment>) -> String {
    attachment.map_or_else(
        || {
            lightbox
                .url
                .split(['?', '#'])
                .next()
                .and_then(|path| path.rsplit('/').next())
                .filter(|name| !name.is_empty())
                .unwrap_or("image")
                .to_owned()
        },
        |attachment| attachment.filename.clone(),
    )
}

pub(super) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    let lightbox = app.lightbox.as_ref()?;
    let (width, height) = fitted_size(lightbox, window);
    // Moving either way, the picture keeps the zoomed resolution, so it doesn't swap mid-way.
    let (width, height) = if lightbox.zoomed || lightbox.motion.is_some() {
        (width * ZOOM, height * ZOOM)
    } else {
        (width, height)
    };
    let frames = if matches!(
        lightbox.spec,
        ImageSpec::Video { .. } | ImageSpec::LocalVideo { .. }
    ) {
        crate::media::video::resolve(&lightbox.spec, theme::image_px(width.max(height)), cx)
            .or_else(|| crate::media::video::resolve(&lightbox.spec, lightbox.preview, cx))
    } else if lightbox.animated {
        crate::media::animated::resolve(&lightbox.spec, theme::image_px(width.max(height)), cx)
    } else {
        None
    };
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
            let fallback = if lightbox.animated {
                let pixels = theme::image_px(width.max(height));
                if let Some(image) = crate::media::animated::poster(spec, pixels, cx) {
                    return Some(
                        image
                            .size_full()
                            .object_fit(ObjectFit::Contain)
                            .rounded(rpx(4.))
                            .id("lightbox-still")
                            .into_any_element(),
                    );
                }
                crate::media::animated::fallback(spec, pixels, cx)?
            } else {
                spec.clone()
            };
            images::resolve(&fallback, lightbox.width.max(lightbox.height), cx)
                .or_else(|| images::resolve(&fallback, lightbox.preview, cx))
                .map(|path| {
                    embeds::animated(path, &lightbox.key)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .rounded(rpx(4.))
                        .into_any_element()
                })
        });
    let dismiss = app.entity.clone();
    let found = origin(app, lightbox);
    let press = app.entity.clone();
    let click = app.entity.clone();
    let context = app.entity.clone();
    let pan = app.entity.clone();
    let dragging = lightbox.drag.is_some_and(|drag| drag.moved);
    let picture = div()
        .id("lightbox-image")
        .flex_none()
        .w(rpx(width))
        .h(rpx(height))
        .rounded(rpx(4.))
        .overflow_hidden()
        .bg(rgb(theme::elevated()))
        .without_text_selection()
        .cursor(match (lightbox.zoomed, dragging) {
            (true, true) => CursorStyle::ClosedHand,
            (true, false) => CursorStyle::OpenHand,
            (false, _) => CursorStyle::PointingHand,
        })
        .on_mouse_down(MouseButton::Left, move |event, _, cx| {
            let start = event.position;
            press
                .update(cx, |app, _| app.start_lightbox_drag(start))
                .ok();
        })
        .on_mouse_down(MouseButton::Right, move |event, _, cx| {
            cx.stop_propagation();
            let position = event.position;
            context
                .update(cx, |app, cx| app.open_lightbox_context_menu(position, cx))
                .ok();
        })
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            let position = event.position();
            click
                .update(cx, |app, cx| app.click_lightbox_image(position, window, cx))
                .ok();
        })
        .children(image);
    let picture = match lightbox.motion {
        Some(ZoomMotion { id, from, to }) => picture
            .with_animation(
                ("lightbox-zoom", id),
                Animation::new(ZOOM_DURATION).with_easing(ease_out_quint()),
                move |this, delta| {
                    let lerp = |from: Pixels, to: Pixels| from + (to - from) * delta;
                    this.absolute()
                        .left(lerp(from.origin.x, to.origin.x))
                        .top(lerp(from.origin.y, to.origin.y))
                        .w(lerp(from.size.width, to.size.width))
                        .h(lerp(from.size.height, to.size.height))
                },
            )
            .into_any_element(),
        None if lightbox.zoomed => div()
            .id("lightbox-zoomed")
            .absolute()
            .inset_0()
            .flex()
            .overflow_scroll()
            .track_scroll(&lightbox.scroll)
            .on_mouse_move(move |event, window, cx| {
                if event.dragging() {
                    let position = event.position;
                    pan.update(cx, |app, cx| app.drag_lightbox(position, window, cx))
                        .ok();
                }
            })
            .child(div().flex_none().m_auto().p(rpx(MARGIN)).child(picture))
            .into_any_element(),
        None => picture.into_any_element(),
    };
    let paged = lightbox
        .gallery
        .as_ref()
        .is_some_and(|(entries, _)| entries.len() > 1);
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
                dismiss
                    .update(cx, |app, cx| {
                        let menu_open = app
                            .lightbox
                            .as_ref()
                            .is_some_and(|lightbox| lightbox.menu != Menu::Closed);
                        if menu_open {
                            app.set_lightbox_menu(Menu::Closed, cx);
                        } else {
                            app.close_lightbox(cx);
                        }
                    })
                    .ok();
            })
            .with_animation(("lightbox-backdrop", 0u64), animation, |this, delta| {
                this.bg(rgba(theme::LIGHTBOX_SCRIM).opacity(delta))
            })
            .child(picture)
            .when(paged, |this| {
                this.child(gallery_button(app, false))
                    .child(gallery_button(app, true))
            })
            .children(found.map(|origin| author_chip(app, origin.message, cx)))
            .child(toolbar(app, lightbox, found))
            .children(lightbox.context_menu.map(|position| {
                context_menu(
                    app,
                    lightbox,
                    position,
                    found.and_then(|origin| origin.attachment),
                )
            }))
            .into_any_element(),
    )
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

    #[test]
    fn lightboxes_only_stream_flagged_animations_or_gif_urls() {
        for extension in ["jpg", "png", "avif"] {
            let still = media(
                &format!("https://media.discordapp.net/photo.{extension}?size=8192"),
                8192,
                6144,
            );
            let lightbox = Lightbox::gifv(&still, None, 512);
            assert!(!lightbox.animated);
            assert_eq!(lightbox.spec, still.spec());
            assert_eq!((lightbox.width, lightbox.height), (8192, 6144));
        }
        let gif = media("https://media.discordapp.net/clip.GIF?token=keep", 400, 300);
        assert!(Lightbox::gifv(&gif, None, 512).animated);
        let mut webp = media("https://media.discordapp.net/clip.webp", 400, 300);
        webp.animated = true;
        assert!(Lightbox::gifv(&webp, None, 512).animated);
        let video = media("https://media.discordapp.net/clip.mp4", 400, 300);
        assert!(!Lightbox::gifv(&gif, Some(&video), 512).animated);
    }
}
