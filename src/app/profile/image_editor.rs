use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use gpui::{App, Context, Pixels, Point, RenderImage, SharedString};
use gpui_tokio::Tokio;

use super::styles::{
    ImageChange, ImageTarget, StyleCrop, StyleDraft, decode_style_still, encode_style_bytes,
    image_limits, read_style_file, style_data_uri,
};
use super::styles_panel::StyleDialog;
use crate::{
    app::DiscordApp,
    media::images::{self, ImageSpec},
};

pub(super) const STAGE: [f32; 2] = [432., 350.];
const FIT: [f32; 2] = [432., 315.];
pub(super) const MAX_ZOOM: f32 = 2.;

#[derive(Clone)]
pub(crate) struct ImageEdit {
    pub(crate) target: ImageTarget,
    path: PathBuf,
    bytes: Arc<[u8]>,
    natural: [u32; 2],
    pub(crate) preview: Arc<RenderImage>,
    preview_frames: Arc<Vec<image::Frame>>,
    pub(crate) quarter_turns: u8,
    pub(crate) zoom: f32,
    pub(crate) offset: [f32; 2],
    drag: Option<(Point<Pixels>, [f32; 2])>,
    pub(crate) applying: bool,
    pub(crate) error: Option<SharedString>,
    recent: Option<(u64, SharedString)>,
}

impl ImageEdit {
    pub(super) fn load(
        path: PathBuf,
        target: ImageTarget,
        preview_px: u32,
    ) -> Result<Self, String> {
        let bytes = read_style_file(&path)?;
        Self::from_bytes(path, bytes, target, preview_px)
    }

    pub(super) fn load_recent(
        path: &Path,
        (id, storage_hash): (u64, SharedString),
        preview_px: u32,
    ) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|_| "Could not load this avatar.".to_owned())?;
        let mut edit = Self::from_bytes(
            PathBuf::from(format!("{id}.png")),
            bytes,
            ImageTarget::Avatar,
            preview_px,
        )?;
        edit.recent = Some((id, storage_hash));
        Ok(edit)
    }

    fn from_bytes(
        path: PathBuf,
        bytes: Vec<u8>,
        target: ImageTarget,
        preview_px: u32,
    ) -> Result<Self, String> {
        let image = decode_style_still(&bytes)?;
        let natural = [image.width(), image.height()];
        let frames = animation_preview(&bytes, (preview_px / 2).max(1)).unwrap_or_else(|| {
            vec![preview_frame(
                image,
                preview_px,
                image::Delay::from_numer_denom_ms(0, 1),
            )]
        });
        Ok(Self {
            target,
            path,
            bytes: bytes.into(),
            natural,
            preview: render_image(&frames),
            preview_frames: Arc::new(frames),
            quarter_turns: 0,
            zoom: 1.,
            offset: [0., 0.],
            drag: None,
            applying: false,
            error: None,
            recent: None,
        })
    }

    fn base(&self) -> [f32; 2] {
        let [width, height] = self.natural.map(|side| side as f32);
        let scale = (FIT[0] / width).min(FIT[1] / height);
        let base = [width * scale, height * scale];
        if self.quarter_turns % 2 == 1 {
            [base[1], base[0]]
        } else {
            base
        }
    }

    pub(crate) fn image_size(&self) -> [f32; 2] {
        self.base().map(|side| side * self.zoom)
    }

    pub(crate) fn frame_size(&self) -> [f32; 2] {
        let [width, height] = self.base();
        let (ratio_width, ratio_height) = self.target.ratio();
        let ratio = ratio_width as f32 / ratio_height as f32;
        if width / height > ratio {
            [height * ratio, height]
        } else {
            [width, width / ratio]
        }
    }

    fn clamped(&self, offset: [f32; 2]) -> [f32; 2] {
        let (image, frame) = (self.image_size(), self.frame_size());
        [0, 1].map(|axis| {
            let limit = ((image[axis] - frame[axis]) / 2.).max(0.);
            offset[axis].clamp(-limit, limit)
        })
    }

    pub(crate) fn edited(&self) -> bool {
        self.quarter_turns != 0 || self.zoom != 1. || self.offset != [0., 0.]
    }

    pub(crate) fn crop(&self) -> StyleCrop {
        let turned = if self.quarter_turns % 2 == 1 {
            [self.natural[1], self.natural[0]]
        } else {
            self.natural
        };
        let (image, frame) = (self.image_size(), self.frame_size());
        let scale = turned[0] as f32 / image[0];
        let [(x, width), (y, height)] = [0, 1].map(|axis| {
            let side = turned[axis] as f32;
            let size = (frame[axis] * scale).round().clamp(1., side);
            let start = (side / 2. - self.offset[axis] * scale - size / 2.)
                .round()
                .clamp(0., side - size);
            (start as u32, size as u32)
        });
        StyleCrop {
            quarter_turns: self.quarter_turns,
            rect: [x, y, width, height],
        }
    }

    fn rotate(&mut self) -> Arc<RenderImage> {
        let frames: Vec<_> = self
            .preview_frames
            .iter()
            .map(|frame| {
                let turned = image::imageops::rotate90(frame.buffer());
                image::Frame::from_parts(turned, 0, 0, frame.delay())
            })
            .collect();
        let replaced = std::mem::replace(&mut self.preview, render_image(&frames));
        self.preview_frames = Arc::new(frames);
        self.quarter_turns = (self.quarter_turns + 1) % 4;
        let [x, y] = self.offset;
        self.offset = self.clamped([-y, x]);
        replaced
    }
}

fn render_image(frames: &[image::Frame]) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(frames.to_vec()))
}

/// Frees `image` from every window's atlas. gpui lifts the window out of the app while one of
/// its handlers runs, so dropping right away would miss the window that shows the preview.
pub(super) fn drop_image_later(image: Arc<RenderImage>, cx: &mut App) {
    cx.defer(move |cx| cx.drop_image(image, None));
}

const ANIMATION_PREVIEW_BUDGET: usize = 192 * 1024 * 1024;

fn preview_frame(image: image::DynamicImage, side: u32, delay: image::Delay) -> image::Frame {
    let image = if image.width().max(image.height()) > side {
        image.thumbnail(side, side)
    } else {
        image
    };
    let mut pixels = image.into_rgba8();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    image::Frame::from_parts(pixels, 0, 0, delay)
}

fn animation_preview(bytes: &[u8], side: u32) -> Option<Vec<image::Frame>> {
    use image::{AnimationDecoder, ImageDecoder, ImageFormat};
    let cursor = std::io::Cursor::new(bytes);
    let frames = match image::guess_format(bytes).ok()? {
        ImageFormat::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(cursor).ok()?;
            decoder.set_limits(image_limits()).ok()?;
            decoder.into_frames()
        }
        ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(cursor).ok()?;
            if !decoder.has_animation() {
                return None;
            }
            decoder.set_limits(image_limits()).ok()?;
            decoder.into_frames()
        }
        _ => return None,
    };
    let mut preview = Vec::new();
    let mut held = 0;
    for frame in frames {
        let Ok(frame) = frame else { break };
        let delay = frame.delay();
        let frame = preview_frame(
            image::DynamicImage::ImageRgba8(frame.into_buffer()),
            side,
            delay,
        );
        held += frame.buffer().len();
        preview.push(frame);
        if held > ANIMATION_PREVIEW_BUDGET || preview.len() >= 1024 {
            break;
        }
    }
    (preview.len() > 1).then_some(preview)
}

fn preview_directory() -> PathBuf {
    std::env::temp_dir().join("zcord-style-uploads")
}

fn write_preview(extension: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let directory = preview_directory();
    std::fs::create_dir_all(&directory).map_err(|_| "Could not prepare the image.".to_owned())?;
    // A fresh name each time: gpui caches images by path.
    let path = directory.join(format!("{:016x}.{extension}", rand::random::<u64>()));
    std::fs::write(&path, bytes).map_err(|_| "Could not prepare the image.".to_owned())?;
    Ok(path)
}

/// How long a fresh preview is spared, since an Apply that is still finishing has written it
/// but not yet put it in the draft.
const PREVIEW_GRACE: std::time::Duration = std::time::Duration::from_secs(60);

fn prune_previews(directory: &Path, keep: &[&Path], spare_recent: bool) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if keep.contains(&path.as_path()) {
            continue;
        }
        let recent = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| {
                now.duration_since(modified)
                    .is_ok_and(|age| age < PREVIEW_GRACE)
            });
        if !(spare_recent && recent) {
            // A preview gpui still has open can't go on Windows; the next prune retries it.
            let _ = std::fs::remove_file(path);
        }
    }
}

pub(super) fn draft_previews(draft: &StyleDraft) -> Vec<&Path> {
    let directory = preview_directory();
    [&draft.avatar, &draft.banner]
        .into_iter()
        .filter_map(|change| match change {
            Some(ImageChange::Upload { preview, .. }) => Some(preview.as_path()),
            _ => None,
        })
        .filter(|preview| preview.parent() == Some(directory.as_path()))
        .collect()
}

impl DiscordApp {
    pub(crate) fn edit_recent_avatar(
        &mut self,
        id: u64,
        storage_hash: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.profile_styles.as_ref().filter(|editor| !editor.saving) else {
            return;
        };
        let identity = editor.identity();
        let Some(service) = images::download_service(cx) else {
            return;
        };
        let still = ImageSpec::ArchivedAvatar {
            user_id: self.store.current_user_id,
            avatar_id: id,
            hash: storage_hash.to_string(),
        };
        let animated = still.animated();
        let preview_px = crate::theme::image_px(STAGE[0] * MAX_ZOOM);
        let recent = (id, storage_hash);
        let request = Tokio::spawn(cx, async move {
            let path = match &animated {
                Some(spec) => service.load(spec, 1024).await,
                None => None,
            };
            let path = match path {
                Some(path) => path,
                None => service
                    .load(&still, 1024)
                    .await
                    .ok_or_else(|| "Could not download this avatar.".to_owned())?,
            };
            tokio::task::spawn_blocking(move || ImageEdit::load_recent(&path, recent, preview_px))
                .await
                .map_err(|_| "Could not load this avatar.".to_owned())?
        });
        cx.spawn(async move |this, cx| {
            let result = request
                .await
                .unwrap_or_else(|_| Err("Could not download this avatar.".to_owned()));
            this.update(cx, |app, cx| {
                let Some(editor) = app
                    .profile_styles
                    .as_mut()
                    .filter(|editor| Arc::ptr_eq(&editor.identity(), &identity))
                else {
                    return;
                };
                match result {
                    Ok(edit) => app.open_style_dialog(StyleDialog::ImageEdit(edit), cx),
                    Err(error) => {
                        editor.error = Some(error.into());
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn prune_style_previews(&self, spare_recent: bool) {
        let keep = self
            .profile_styles
            .as_ref()
            .map(|editor| draft_previews(&editor.draft))
            .unwrap_or_default();
        prune_previews(&preview_directory(), &keep, spare_recent);
    }

    fn image_edit_mut(&mut self) -> Option<&mut ImageEdit> {
        let ui = &mut self.profile_styles.as_mut()?.ui;
        if ui.dialog_closing.is_some() {
            return None;
        }
        match &mut ui.dialog {
            Some(StyleDialog::ImageEdit(edit)) => Some(edit),
            _ => None,
        }
    }

    fn update_image_edit(&mut self, update: impl FnOnce(&mut ImageEdit), cx: &mut Context<Self>) {
        if let Some(edit) = self.image_edit_mut().filter(|edit| !edit.applying) {
            update(edit);
            cx.notify();
        }
    }

    pub(crate) fn start_image_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.update_image_edit(|edit| edit.drag = Some((position, edit.offset)), cx);
    }

    /// `position` and the drag's start are window pixels, which are the stage's CSS pixels
    /// scaled by `rem_scale`.
    pub(crate) fn drag_image(
        &mut self,
        position: Point<Pixels>,
        rem_scale: f32,
        cx: &mut Context<Self>,
    ) {
        self.update_image_edit(
            |edit| {
                let Some((start, offset)) = edit.drag else {
                    return;
                };
                let delta = position - start;
                edit.offset = edit.clamped([
                    offset[0] + f32::from(delta.x) / rem_scale,
                    offset[1] + f32::from(delta.y) / rem_scale,
                ]);
            },
            cx,
        );
    }

    pub(crate) fn end_image_drag(&mut self, cx: &mut Context<Self>) {
        self.update_image_edit(|edit| edit.drag = None, cx);
    }

    pub(crate) fn set_image_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        self.update_image_edit(
            |edit| {
                let zoom = zoom.clamp(1., MAX_ZOOM);
                let scale = zoom / edit.zoom;
                edit.zoom = zoom;
                edit.offset = edit.clamped(edit.offset.map(|side| side * scale));
            },
            cx,
        );
    }

    pub(crate) fn rotate_image_edit(&mut self, cx: &mut Context<Self>) {
        let mut replaced = None;
        self.update_image_edit(|edit| replaced = Some(edit.rotate()), cx);
        if let Some(image) = replaced {
            drop_image_later(image, cx);
        }
    }

    pub(crate) fn reset_image_edit(&mut self, cx: &mut Context<Self>) {
        let mut replaced = Vec::new();
        self.update_image_edit(
            |edit| {
                while edit.quarter_turns != 0 {
                    replaced.push(edit.rotate());
                }
                edit.zoom = 1.;
                edit.offset = [0., 0.];
            },
            cx,
        );
        for image in replaced {
            drop_image_later(image, cx);
        }
    }

    pub(crate) fn apply_image_edit(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.profile_styles.as_ref() else {
            return;
        };
        let (identity, nitro) = (editor.identity(), editor.nitro);
        let Some(edit) = self.image_edit_mut().filter(|edit| !edit.applying) else {
            return;
        };
        if let Some((id, storage_hash)) = edit.recent.clone().filter(|_| !edit.edited()) {
            self.close_style_dialog(cx);
            self.update_style_draft(
                |draft| draft.avatar = Some(ImageChange::Recent { id, storage_hash }),
                cx,
            );
            return;
        }
        edit.applying = true;
        edit.error = None;
        let shown = edit.preview.clone();
        let (target, crop, path, bytes) = (
            edit.target,
            edit.crop(),
            edit.path.clone(),
            edit.bytes.clone(),
        );
        cx.notify();
        let encode = cx.background_executor().spawn(async move {
            let (extension, encoded) = encode_style_bytes(&bytes, target, nitro, Some(crop))?;
            let preview = write_preview(extension, &encoded)?;
            Ok::<_, String>(ImageChange::Upload {
                path,
                preview,
                data_uri: style_data_uri(extension, &encoded).into(),
            })
        });
        cx.spawn(async move |this, cx| {
            let result = encode.await;
            this.update(cx, |app, cx| {
                let still_open = app
                    .profile_styles
                    .as_ref()
                    .is_some_and(|editor| Arc::ptr_eq(&editor.identity(), &identity))
                    && app
                        .image_edit_mut()
                        .is_some_and(|edit| Arc::ptr_eq(&edit.preview, &shown));
                if !still_open {
                    return;
                }
                match result {
                    Ok(image) => {
                        app.close_style_dialog(cx);
                        app.update_style_draft(
                            |draft| match target {
                                ImageTarget::Avatar => draft.avatar = Some(image),
                                ImageTarget::Banner => draft.banner = Some(image),
                            },
                            cx,
                        );
                    }
                    Err(error) => {
                        if let Some(edit) = app.image_edit_mut() {
                            edit.applying = false;
                            edit.error = Some(error.into());
                        }
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(natural: [u32; 2], target: ImageTarget) -> ImageEdit {
        let pixels = image::RgbaImage::new(4, 4);
        ImageEdit {
            target,
            path: PathBuf::new(),
            bytes: Arc::from([]),
            natural,
            preview: render_image(&[image::Frame::new(pixels.clone())]),
            preview_frames: Arc::new(vec![image::Frame::new(pixels)]),
            quarter_turns: 0,
            zoom: 1.,
            offset: [0., 0.],
            drag: None,
            applying: false,
            error: None,
            recent: None,
        }
    }

    fn close(left: [f32; 2], right: [f32; 2]) -> bool {
        (left[0] - right[0]).abs() < 0.01 && (left[1] - right[1]).abs() < 0.01
    }

    #[test]
    fn image_editor_prunes_previews_the_draft_dropped() {
        let directory = std::env::temp_dir().join(format!(
            "zcord-style-prune-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let file = |name: &str, age: u64| {
            let path = directory.join(name);
            std::fs::write(&path, b"x").unwrap();
            let modified = SystemTime::now() - std::time::Duration::from_secs(age);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(modified)
                .unwrap();
            path
        };
        let (kept, stale, fresh) = (
            file("kept.png", 600),
            file("stale.png", 600),
            file("fresh.png", 0),
        );

        prune_previews(&directory, &[kept.as_path()], true);
        assert!(kept.exists() && fresh.exists() && !stale.exists());
        prune_previews(&directory, &[], false);
        assert!(!kept.exists() && !fresh.exists());
        std::fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn image_editor_fits_like_discord() {
        let avatar = edit([742, 1000], ImageTarget::Avatar);
        assert!(close(avatar.image_size(), [233.73, 315.]));
        assert!(close(avatar.frame_size(), [233.73, 233.73]));
        let banner = edit([1000, 576], ImageTarget::Banner);
        assert!(close(banner.image_size(), [432., 248.83]));
        assert!(close(banner.frame_size(), [432., 152.47]));
    }

    #[test]
    fn image_editor_crop_is_centered_until_moved() {
        let avatar = edit([742, 1000], ImageTarget::Avatar);
        assert_eq!(avatar.crop().rect, [0, 129, 742, 742]);
        let mut banner = edit([1700, 1000], ImageTarget::Banner);
        assert_eq!(
            banner.crop(),
            StyleCrop::centered(1700, 1000, ImageTarget::Banner)
        );
        banner.zoom = 2.;
        let [_, _, width, height] = banner.crop().rect;
        assert_eq!((width, height), (850, 300));
    }

    #[test]
    fn image_editor_keeps_the_frame_on_the_image() {
        let mut avatar = edit([1000, 1000], ImageTarget::Avatar);
        assert_eq!(avatar.clamped([50., -50.]), [0., 0.]);
        avatar.zoom = 2.;
        let [limit, _] = avatar.clamped([1000., 0.]);
        assert!((limit - 157.5).abs() < 0.01);
        avatar.offset = [limit, 0.];
        assert_eq!(avatar.crop().rect[0], 0);
    }

    #[test]
    fn image_editor_rotation_turns_the_crop_and_the_pan() {
        let mut banner = edit([1000, 2000], ImageTarget::Banner);
        let upright = banner.frame_size();
        banner.zoom = 2.;
        banner.offset = banner.clamped([0., 100.]);
        let _ = banner.rotate();
        assert_eq!(banner.quarter_turns, 1);
        assert!(banner.frame_size()[0] > upright[0]);
        assert!(banner.offset[0] < 0. && banner.offset[1] == 0.);
        let [x, y, width, height] = banner.crop().rect;
        assert!(x + width <= 2000 && y + height <= 1000);
    }
}
