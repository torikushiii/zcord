use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_tokio::Tokio;

use crate::{
    app::{
        DiscordApp, chat,
        display_name::{self, Surface},
        embeds, titlebar,
    },
    media::images::{self, ImageSpec},
    store::{Attachment, Media, Message},
    theme::{self, WithoutTextSelection, rpx},
};

const MARGIN: f32 = 80.;
const FADE: Duration = Duration::from_millis(200);
const BAR_INSET: f32 = 8.;
const BAR_HEIGHT: f32 = 44.;
const BAR_ICON: f32 = 20.;
const BAR_BUTTON: f32 = 34.;
const BAR_RADIUS: f32 = 12.;
const BAR_GAP: f32 = 12.;
const MENU_WIDTH: f32 = 208.;
const MENU_ROW: f32 = 40.;
const MENU_GAP: f32 = 8.;
const DETAILS_WIDTH: f32 = 212.;
const SURFACE: u32 = 0x1A1A1E;
const ZOOM: f32 = 2.;
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
    /// Fetched when the details open, for media Discord doesn't report a size for.
    size: Option<u64>,
    scroll: ScrollHandle,
    drag: Option<Drag>,
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
            size: None,
            scroll: ScrollHandle::new(),
            drag: None,
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
        self.lightbox = Some(Lightbox {
            spec,
            animated,
            poster: None,
            width: width.max(1),
            height: height.max(1),
            preview,
            key: url.clone(),
            url,
            zoomed: false,
            menu: Menu::Closed,
            size: None,
            scroll: ScrollHandle::new(),
            drag: None,
        });
        cx.notify();
    }

    pub(crate) fn close_lightbox(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.lightbox.take().is_some();
        if open {
            self.hovered_stage_control = None;
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
        let Some(lightbox) = &mut self.lightbox else {
            return;
        };
        lightbox.zoomed = !lightbox.zoomed;
        lightbox.drag = None;
        if lightbox.zoomed {
            let (width, height) = fitted_size(lightbox, window);
            let unit = rpx(1.).to_pixels(window.rem_size());
            let viewport = window.viewport_size();
            let center = point(viewport.width / 2., viewport.height / 2.);
            let (image_x, image_y) = focus.map_or((0.5, 0.5), |focus| {
                (
                    0.5 + (focus.x - center.x) / (unit * width),
                    0.5 + (focus.y - center.y) / (unit * height),
                )
            });
            lightbox.scroll.set_offset(point(
                center.x - unit * (MARGIN + width * ZOOM * image_x),
                center.y - unit * (MARGIN + height * ZOOM * image_y),
            ));
        }
        cx.notify();
    }

    fn start_lightbox_drag(&mut self, start: Point<Pixels>) {
        if let Some(lightbox) = self.lightbox.as_mut().filter(|box_| box_.zoomed) {
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

fn bar_button(
    app: &DiscordApp,
    id: &'static str,
    tip: &'static str,
    icon: &str,
    active: bool,
) -> Stateful<Div> {
    let hover = app.entity.clone();
    div()
        .id(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(rpx(BAR_BUTTON))
        .rounded(rpx(8.))
        .cursor_pointer()
        .when(active, |this| this.bg(rgba(theme::selected())))
        .hover(|style| style.bg(rgba(theme::selected())))
        .on_hover(move |hovered, _, cx| {
            hover
                .update(cx, |app, cx| {
                    if *hovered {
                        app.hovered_stage_control = Some(id);
                    } else if app.hovered_stage_control == Some(id) {
                        app.hovered_stage_control = None;
                    }
                    app.views.overlays.update(cx, |_, cx| cx.notify());
                })
                .ok();
        })
        .child(theme::icon(icon, BAR_ICON, theme::strong()))
        .when(app.hovered_stage_control == Some(id), |this| {
            this.child(theme::tooltip_below(tip))
        })
}

fn panel() -> Div {
    div()
        .bg(rgb(SURFACE))
        .border_1()
        .border_color(rgba(theme::border()))
        .rounded(rpx(BAR_RADIUS))
        .shadow_lg()
}

fn menu_row(
    app: &DiscordApp,
    id: &'static str,
    label: &'static str,
    icon: Option<&str>,
    menu: Menu,
) -> Stateful<Div> {
    let hover = app.entity.clone();
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(rpx(8.))
        .h(rpx(MENU_ROW))
        .px(rpx(8.))
        .rounded(rpx(8.))
        .cursor_pointer()
        .text_size(rpx(14.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme::strong()))
        .hover(|style| style.bg(rgba(theme::selected())))
        .on_hover(move |hovered, _, cx| {
            if *hovered {
                hover
                    .update(cx, |app, cx| app.set_lightbox_menu(menu, cx))
                    .ok();
            }
        })
        .children(icon.map(|icon| theme::icon(icon, 20., theme::strong())))
        .child(div().flex_1().min_w_0().truncate().child(label))
}

fn details_field(title: &'static str, value: String) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(rpx(4.))
        .child(
            div()
                .text_size(rpx(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme::strong()))
                .child(title),
        )
        .child(
            div()
                .truncate()
                .text_size(rpx(13.))
                .text_color(rgb(theme::muted()))
                .child(value),
        )
}

fn more_menu(app: &DiscordApp, lightbox: &Lightbox, attachment: Option<&Attachment>) -> Div {
    let copy = app.entity.clone();
    let close_link = app.entity.clone();
    let close_id = app.entity.clone();
    let url = lightbox.url.clone();
    let has_url = !url.is_empty();
    let attachment_id = attachment.map(|attachment| attachment.id);
    let size = format!("{}x{}", lightbox.width, lightbox.height);
    let bytes = attachment
        .map(|attachment| attachment.size)
        .or(lightbox.size);
    let size = match bytes {
        Some(bytes) => format!("{size} ({})", chat::file_size(bytes)),
        None => size,
    };
    div()
        .absolute()
        .top(rpx(BAR_HEIGHT + MENU_GAP))
        .right_0()
        .child(
            panel()
                .relative()
                .flex()
                .flex_col()
                .w(rpx(MENU_WIDTH))
                .p(rpx(8.))
                .child(
                    menu_row(
                        app,
                        "lightbox-copy-image",
                        "Copy Image",
                        Some("image"),
                        Menu::Open,
                    )
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        copy.update(cx, |app, cx| {
                            app.copy_lightbox_image(cx);
                            app.set_lightbox_menu(Menu::Closed, cx);
                        })
                        .ok();
                    }),
                )
                .when(has_url, |this| {
                    this.child(
                        menu_row(
                            app,
                            "lightbox-copy-link",
                            "Copy Media Link",
                            Some("link"),
                            Menu::Open,
                        )
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                            close_link
                                .update(cx, |app, cx| app.set_lightbox_menu(Menu::Closed, cx))
                                .ok();
                        }),
                    )
                })
                .child(
                    menu_row(app, "lightbox-details", "View Details", None, Menu::Details)
                        .when(lightbox.menu == Menu::Details, |this| {
                            this.bg(rgba(theme::selected()))
                        })
                        .child(theme::icon("chevron-right", 16., theme::strong())),
                )
                .children(attachment_id.map(|id| {
                    menu_row(
                        app,
                        "lightbox-copy-id",
                        "Copy Attachment ID",
                        Some("id-badge"),
                        Menu::Open,
                    )
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()));
                        close_id
                            .update(cx, |app, cx| app.set_lightbox_menu(Menu::Closed, cx))
                            .ok();
                    })
                }))
                .when(lightbox.menu == Menu::Details, |this| {
                    this.child(
                        panel()
                            .absolute()
                            .top(rpx(MENU_ROW))
                            .right(rpx(MENU_WIDTH + MENU_GAP))
                            .flex()
                            .flex_col()
                            .gap(rpx(16.))
                            .w(rpx(DETAILS_WIDTH))
                            .p(rpx(16.))
                            .child(details_field("Filename", file_name(lightbox, attachment)))
                            .child(details_field("Size", size)),
                    )
                }),
        )
}

fn toolbar(app: &DiscordApp, lightbox: &Lightbox, found: Option<Origin<'_>>) -> Stateful<Div> {
    let attachment = found.and_then(|origin| origin.attachment);
    let forward_target = found
        .filter(|origin| app.live && origin.message.forwardable)
        .map(|origin| (origin.message.id, origin.channel_id));
    let forward = app.entity.clone();
    let zoom = app.entity.clone();
    let save = app.entity.clone();
    let more = app.entity.clone();
    let close = app.entity.clone();
    let filename = file_name(lightbox, attachment);
    let url = lightbox.url.clone();
    let has_url = !url.is_empty();
    let menu_open = lightbox.menu != Menu::Closed;
    div()
        .id("lightbox-toolbar")
        .absolute()
        .top(rpx(titlebar::HEIGHT + BAR_INSET))
        .right(rpx(BAR_INSET))
        .flex()
        .items_center()
        .gap(rpx(BAR_GAP))
        .occlude()
        .without_text_selection()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(
            panel()
                .relative()
                .flex()
                .items_center()
                .gap(rpx(2.))
                .h(rpx(BAR_HEIGHT))
                .px(rpx(5.))
                .child(
                    bar_button(
                        app,
                        "lightbox-zoom",
                        if lightbox.zoomed {
                            "Zoom Out"
                        } else {
                            "Zoom In"
                        },
                        if lightbox.zoomed {
                            "zoom-out"
                        } else {
                            "zoom-in"
                        },
                        false,
                    )
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        zoom.update(cx, |app, cx| app.toggle_lightbox_zoom(None, window, cx))
                            .ok();
                    }),
                )
                .children(forward_target.map(|(message_id, channel_id)| {
                    bar_button(app, "lightbox-forward", "Forward", "forward", false).on_click(
                        move |_, window, cx| {
                            cx.stop_propagation();
                            forward
                                .update(cx, |app, cx| {
                                    app.open_forward_dialog(message_id, channel_id, window, cx)
                                })
                                .ok();
                        },
                    )
                }))
                .child(
                    bar_button(app, "lightbox-download", "Download", "download", false).on_click(
                        move |_, _, cx| {
                            cx.stop_propagation();
                            save.update(cx, |app, cx| {
                                app.save_lightbox_image(filename.clone(), cx)
                            })
                            .ok();
                        },
                    ),
                )
                .when(has_url, |this| {
                    this.child(
                        bar_button(
                            app,
                            "lightbox-open-browser",
                            "Open in Browser",
                            "external-link",
                            false,
                        )
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            cx.open_url(&url);
                        }),
                    )
                })
                .child(
                    bar_button(app, "lightbox-more", "More", "ellipsis", menu_open).on_click(
                        move |_, _, cx| {
                            cx.stop_propagation();
                            let menu = if menu_open { Menu::Closed } else { Menu::Open };
                            more.update(cx, |app, cx| app.set_lightbox_menu(menu, cx))
                                .ok();
                        },
                    ),
                )
                .when(menu_open, |this| {
                    this.child(more_menu(app, lightbox, attachment))
                }),
        )
        .child(
            panel()
                .flex()
                .items_center()
                .justify_center()
                .size(rpx(BAR_HEIGHT))
                .child(
                    bar_button(app, "lightbox-close", "Close", "x", false).on_click(
                        move |_, _, cx| {
                            cx.stop_propagation();
                            close.update(cx, |app, cx| app.close_lightbox(cx)).ok();
                        },
                    ),
                ),
        )
}

fn author_chip(app: &DiscordApp, message: &Message, cx: &App) -> Div {
    let guild_id = app.selected_guild_id;
    let tag = app
        .store
        .user(message.author_id)
        .and_then(|user| user.tag.as_ref());
    let style = app.store.member_name_style(guild_id, message.author_id);
    let name = app
        .store
        .member_name(guild_id, message.author_id)
        .map(str::to_owned)
        .or_else(|| {
            app.store
                .user(message.author_id)
                .map(|user| user.name.clone())
        })
        .unwrap_or_default();
    div()
        .absolute()
        .top(rpx(titlebar::HEIGHT + BAR_INSET))
        .left(rpx(16.))
        .flex()
        .items_center()
        .gap(rpx(12.))
        .max_w(relative(0.4))
        .without_text_selection()
        .child(theme::undecorated_member_avatar(
            &app.store,
            guild_id,
            message.author_id,
            40.,
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rpx(4.))
                        .child(
                            div()
                                .truncate()
                                .text_size(rpx(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme::strong()))
                                .child(display_name::render(
                                    ("lightbox-author", message.id),
                                    &name,
                                    style,
                                    Surface::FontOnly,
                                    cx,
                                )),
                        )
                        .children(tag.map(|tag| theme::guild_tag(tag, cx))),
                )
                .child(
                    div()
                        .text_size(rpx(12.))
                        .text_color(rgb(theme::muted()))
                        .child(message.timestamp.short()),
                ),
        )
}

pub(super) fn render(app: &DiscordApp, window: &Window, cx: &App) -> Option<AnyElement> {
    let lightbox = app.lightbox.as_ref()?;
    let (width, height) = fitted_size(lightbox, window);
    let (width, height) = if lightbox.zoomed {
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
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            let position = event.position();
            click
                .update(cx, |app, cx| app.click_lightbox_image(position, window, cx))
                .ok();
        })
        .children(image);
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
            .map(|this| {
                if lightbox.zoomed {
                    this.child(
                        div()
                            .id("lightbox-zoomed")
                            .absolute()
                            .inset_0()
                            .flex()
                            .overflow_scroll()
                            .track_scroll(&lightbox.scroll)
                            .on_mouse_move(move |event, window, cx| {
                                if event.dragging() {
                                    let position = event.position;
                                    pan.update(cx, |app, cx| {
                                        app.drag_lightbox(position, window, cx)
                                    })
                                    .ok();
                                }
                            })
                            .child(div().flex_none().m_auto().p(rpx(MARGIN)).child(picture)),
                    )
                } else {
                    this.child(picture)
                }
            })
            .children(found.map(|origin| author_chip(app, origin.message, cx)))
            .child(toolbar(app, lightbox, found))
            .into_any_element(),
    )
}
