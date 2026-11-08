use super::super::*;
use crate::media::images::ImageSpec;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

pub(crate) const MAX_FILES: usize = 10;
pub(crate) const STAGED_PREVIEW: f32 = 200.;
const VIDEO_LIGHTBOX_SIDE: u32 = 1920;
const DROP_SCRIM: u32 = 0x000000D9;
const DROP_ENTER: Duration = Duration::from_millis(160);

#[derive(Clone)]
pub(crate) struct StagedFile {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub content_type: String,
    pub spoiler: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UploadButton {
    Attach,
    Spoiler(u64),
    Remove(u64),
    Cancel(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    Invalid,
    TooMany,
    TooLarge(u64),
}

fn refusal(paths: &[PathBuf], staged: usize, limit: u64) -> Result<(), Refusal> {
    if paths.iter().any(|path| !path.is_file()) {
        return Err(Refusal::Invalid);
    }
    if staged + paths.len() > MAX_FILES {
        return Err(Refusal::TooMany);
    }
    for path in paths {
        let size = path.metadata().map_err(|_| Refusal::Invalid)?.len();
        if size > limit {
            return Err(Refusal::TooLarge(limit));
        }
    }
    Ok(())
}

pub(super) fn content_type(name: &str) -> &'static str {
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "txt" => "text/plain",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "json" => "application/json",
        _ => "",
    }
}

fn pasted_root() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("zcord").join("pasted"))
}

/// Picked files belong to the user, so only folders under `root` are ever removed.
fn pasted_folders(files: &[StagedFile], root: &Path) -> Vec<PathBuf> {
    files
        .iter()
        .filter(|file| file.path.starts_with(root))
        .filter_map(|file| file.path.parent().map(Path::to_owned))
        .collect()
}

pub(crate) fn remove_pasted(files: &[StagedFile], cx: &App) {
    let Some(root) = pasted_root() else {
        return;
    };
    let folders = pasted_folders(files, &root);
    if folders.is_empty() {
        return;
    }
    cx.background_spawn(async move {
        for folder in folders {
            let _ = std::fs::remove_dir_all(folder);
        }
    })
    .detach();
}

/// Staged files don't survive a restart, so anything left here is unreachable.
pub(crate) fn clear_pasted(cx: &App) {
    let Some(root) = pasted_root() else {
        return;
    };
    cx.background_spawn(async move {
        let _ = std::fs::remove_dir_all(root);
    })
    .detach();
}

/// Windows hands screenshots over as bitmaps; Discord's client turns them into `image.png`.
fn clipboard_png(bytes: &[u8], format: image::ImageFormat) -> Option<Vec<u8>> {
    let mut pixels = image::load_from_memory_with_format(bytes, format)
        .ok()?
        .into_rgba8();
    // 32-bit clipboard bitmaps usually leave alpha at zero.
    if pixels.pixels().all(|pixel| pixel.0[3] == 0) {
        for pixel in pixels.pixels_mut() {
            pixel.0[3] = 255;
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

fn clipboard_image_extension(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Png => Some("png"),
        ImageFormat::Jpeg => Some("jpeg"),
        ImageFormat::Webp => Some("webp"),
        ImageFormat::Gif => Some("gif"),
        ImageFormat::Bmp => Some("bmp"),
        ImageFormat::Tiff => Some("tiff"),
        _ => None,
    }
}

fn drop_sheet(glyph: &str, angle: f32, left: f32, top: f32) -> impl IntoElement + use<> {
    let layer = |name: &str, color: u32| {
        svg()
            .absolute()
            .inset_0()
            .size_full()
            .path(SharedString::from(format!("app/{name}.svg")))
            .text_color(rgb(color))
            .with_transformation(Transformation::rotate(radians(angle)))
    };
    div()
        .absolute()
        .left(rpx(left))
        .top(rpx(top))
        .w(rpx(84.))
        .h(rpx(98.))
        .child(layer("upload-sheet", theme::upload_sheet()))
        .child(layer("upload-fold", theme::upload_fold()))
        .child(layer(glyph, theme::BLURPLE))
}

pub(crate) fn drop_overlay(app: &DiscordApp, cx: &App) -> Div {
    let name = app.channel().map_or("", |channel| channel.name.as_str());
    let target = match app.store.private_channel(app.selected_channel_id) {
        Some(dm) if dm.group => name.to_owned(),
        Some(_) => format!("@{name}"),
        None => format!("#{name}"),
    };
    let card = div()
        .relative()
        .flex()
        .flex_col()
        .items_center()
        .justify_end()
        .w(rpx(370.))
        .h(rpx(214.))
        .pb(rpx(46.))
        .rounded(rpx(16.))
        .bg(rgb(theme::BLURPLE))
        .text_color(rgb(0xFFFFFF))
        .child(
            div()
                .absolute()
                .inset(rpx(12.))
                .rounded(rpx(8.))
                .border_2()
                .border_dashed()
                .border_color(rgba(theme::accent_border())),
        )
        .child(
            div()
                .absolute()
                .top(rpx(-40.))
                .left(rpx(60.))
                .w(rpx(250.))
                .h(rpx(120.))
                .child(drop_sheet("upload-glyph-text", -0.42, 12., 14.))
                .child(drop_sheet("upload-glyph-code", 0.42, 154., 14.))
                .child(drop_sheet("upload-glyph-image", 0., 83., 0.)),
        )
        .child(
            div()
                .text_size(rpx(24.))
                .line_height(rpx(30.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .child(format!("Upload to {target}")),
        )
        .child(
            div()
                .mt(rpx(4.))
                .text_size(rpx(13.))
                .line_height(rpx(18.))
                .text_center()
                .child("You can add comments before uploading.")
                .child("Hold shift to upload directly."),
        );
    let card = if cx.reduce_motion() {
        card.into_any_element()
    } else {
        card.with_animation(
            "upload-drop-card",
            Animation::new(DROP_ENTER).with_easing(ease_out_quint()),
            |this, delta| this.opacity(delta).top(rpx(12. * (1. - delta))),
        )
        .into_any_element()
    };
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(DROP_SCRIM))
        .child(card)
}

impl DiscordApp {
    pub(crate) fn drop_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stage_paths(paths, cx);
        if window.modifiers().shift && self.uploads.refusal.is_none() {
            self.send_message(window, cx);
        }
    }

    pub(crate) fn stage_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.stage_paths_in(self.selected_channel_id, paths, cx);
    }

    pub(crate) fn stage_paths_in(
        &mut self,
        channel_id: u64,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let staged = self.uploads.staged.get(&channel_id).map_or(0, Vec::len);
        let limit = self.store.upload_limit(channel_id);
        if let Err(refusal) = refusal(&paths, staged, limit) {
            self.uploads.refusal = Some(refusal);
            cx.notify();
            return;
        }
        self.uploads.refusal = None;
        let staged = self.uploads.staged.entry(channel_id).or_default();
        for path in paths {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let size = path.metadata().map_or(0, |metadata| metadata.len());
            let id = self.uploads.next_id;
            self.uploads.next_id += 1;
            staged.push(StagedFile {
                id,
                path,
                content_type: content_type(&name).to_owned(),
                name,
                size,
                spoiler: false,
            });
        }
        cx.notify();
    }

    pub(crate) fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_files_for(self.selected_channel_id, window, cx);
    }

    pub(crate) fn pick_files_for(
        &mut self,
        channel_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                this.update_in(cx, |app, _, cx| app.stage_paths_in(channel_id, paths, cx))
                    .ok();
            }
        })
        .detach();
    }

    pub(crate) fn paste_clipboard_files(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        // Word and browsers put text and a rendering of it on the clipboard together.
        let has_text = item
            .entries
            .iter()
            .any(|entry| matches!(entry, ClipboardEntry::String(_)));
        let mut handled = false;
        let mut paths = Vec::new();
        for entry in item.entries {
            match entry {
                ClipboardEntry::ExternalPaths(external) => {
                    paths.extend(external.paths().iter().cloned());
                    handled = true;
                }
                ClipboardEntry::Image(image) if !has_text => {
                    handled = true;
                    self.stage_clipboard_image(image, cx);
                }
                ClipboardEntry::Image(_) | ClipboardEntry::String(_) => {}
            }
        }
        if !paths.is_empty() {
            self.stage_paths(paths, cx);
        }
        handled
    }

    fn stage_clipboard_image(&mut self, image: Image, cx: &mut Context<Self>) {
        let Some(extension) = clipboard_image_extension(image.format) else {
            return;
        };
        let Some(root) = pasted_root() else {
            return;
        };
        let id = self.uploads.next_id;
        self.uploads.next_id += 1;
        let folder = root.join(id.to_string());
        let write = Tokio::spawn(cx, async move {
            let bitmap = match image.format {
                ImageFormat::Bmp => Some(image::ImageFormat::Bmp),
                ImageFormat::Tiff => Some(image::ImageFormat::Tiff),
                _ => None,
            };
            let source = image.bytes;
            let (bytes, extension) = tokio::task::spawn_blocking(move || {
                match bitmap.and_then(|format| clipboard_png(&source, format)) {
                    Some(png) => (png, "png"),
                    None => (source, extension),
                }
            })
            .await?;
            tokio::fs::create_dir_all(&folder).await?;
            let path = folder.join(format!("image.{extension}"));
            tokio::fs::write(&path, bytes).await?;
            Ok::<_, std::io::Error>(path)
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(path)) = write.await {
                this.update(cx, |app, cx| app.stage_paths(vec![path], cx))
                    .ok();
            }
        })
        .detach();
    }

    /// Staged ids are unique across channels, so the main and thread pane composers share this.
    fn staged_channel(&self, id: u64) -> Option<u64> {
        self.uploads
            .staged
            .iter()
            .find(|(_, files)| files.iter().any(|file| file.id == id))
            .map(|(channel_id, _)| *channel_id)
    }

    pub(crate) fn remove_staged(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(channel_id) = self.staged_channel(id) else {
            return;
        };
        let empty = match self.uploads.staged.get_mut(&channel_id) {
            Some(files) => {
                let removed: Vec<_> = files.iter().filter(|file| file.id == id).cloned().collect();
                remove_pasted(&removed, cx);
                files.retain(|file| file.id != id);
                files.is_empty()
            }
            None => return,
        };
        if empty {
            self.uploads.staged.remove(&channel_id);
        }
        cx.notify();
    }

    pub(crate) fn clear_focused_staged(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let channel_id = if self.composer_focused(window, cx) {
            self.selected_channel_id
        } else if self.thread_pane_focused(window, cx)
            && let Some(thread) = self.thread_pane_channel()
        {
            thread.id
        } else {
            return false;
        };
        let Some(removed) = self.uploads.staged.remove(&channel_id) else {
            return false;
        };
        remove_pasted(&removed, cx);
        cx.notify();
        true
    }

    pub(crate) fn toggle_staged_spoiler(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(channel_id) = self.staged_channel(id) else {
            return;
        };
        if let Some(file) = self
            .uploads
            .staged
            .get_mut(&channel_id)
            .and_then(|files| files.iter_mut().find(|file| file.id == id))
        {
            file.spoiler = !file.spoiler;
            cx.notify();
        }
    }

    pub(crate) fn open_staged_lightbox(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(file) = self.staged_files().iter().find(|file| file.id == id) else {
            return;
        };
        let path = file.path.clone();
        let animated = file.content_type == "image/gif";
        let pixels = theme::image_px(STAGED_PREVIEW);
        let (spec, dimensions) = if file.content_type.starts_with("video/") {
            let spec = ImageSpec::LocalVideo { path };
            // Video dimensions aren't known locally, so keep the poster's shape at a large size.
            let dimensions = crate::media::video::resolve(&spec, pixels, cx).map(|poster| {
                let size = poster.size(0);
                let (width, height) = (size.width.0.max(1) as u32, size.height.0.max(1) as u32);
                let longest = width.max(height);
                (
                    width * VIDEO_LIGHTBOX_SIDE / longest,
                    height * VIDEO_LIGHTBOX_SIDE / longest,
                )
            });
            (spec, dimensions)
        } else {
            let dimensions = image::ImageReader::open(&path)
                .and_then(|reader| reader.with_guessed_format())
                .ok()
                .and_then(|reader| reader.into_dimensions().ok());
            (ImageSpec::LocalGif { path }, dimensions)
        };
        let Some(dimensions) = dimensions else {
            return;
        };
        self.open_lightbox(spec, animated, dimensions, pixels, String::new(), cx);
    }

    pub(crate) fn staged_files(&self) -> &[StagedFile] {
        self.staged_files_in(self.selected_channel_id)
    }

    pub(crate) fn staged_files_in(&self, channel_id: u64) -> &[StagedFile] {
        self.uploads
            .staged
            .get(&channel_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn dismiss_upload_refusal(&mut self, cx: &mut Context<Self>) {
        if self.uploads.refusal.take().is_some() {
            cx.notify();
        }
    }

    pub(crate) fn cancel_upload(&self, nonce: u64) {
        if let Some(progress) = self.uploads.progress.get(&nonce) {
            progress.cancelled.store(true, Ordering::Release);
        }
    }

    pub(crate) fn upload_fraction(&self, nonce: u64) -> Option<f32> {
        self.uploads.progress.get(&nonce).map(|progress| {
            if progress.total == 0 {
                1.0
            } else {
                progress.sent.load(Ordering::Acquire) as f32 / progress.total as f32
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_FILES, Refusal, StagedFile, content_type, pasted_folders, refusal};
    use std::fs;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("zcord-attachments-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn staged(name: &str, spoiler: bool) -> StagedFile {
        StagedFile {
            id: 1,
            path: PathBuf::from(name),
            name: name.to_owned(),
            size: 0,
            content_type: content_type(name).to_owned(),
            spoiler,
        }
    }

    #[test]
    fn clipboard_bitmaps_become_opaque_pngs() {
        let bitmap = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 0]));
        let mut bmp = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(bitmap)
            .write_to(&mut bmp, image::ImageFormat::Bmp)
            .unwrap();
        let png = super::clipboard_png(bmp.get_ref(), image::ImageFormat::Bmp).unwrap();
        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        assert_eq!(decoded.get_pixel(1, 1).0, [10, 20, 30, 255]);
        assert!(super::clipboard_png(b"not a bitmap", image::ImageFormat::Bmp).is_none());
    }

    #[test]
    fn only_files_inside_the_pasted_folder_are_removed() {
        let root = PathBuf::from("cache/zcord/pasted");
        let pasted = StagedFile {
            path: root.join("7").join("image.png"),
            ..staged("image.png", false)
        };
        let picked = staged("documents/report.pdf", false);
        assert_eq!(pasted_folders(&[picked, pasted], &root), [root.join("7")]);
    }

    #[test]
    fn refusal_applies_discord_batch_rules() {
        let dir = scratch("refusal");
        let small = dir.join("small.txt");
        fs::write(&small, b"abc").unwrap();
        let large = dir.join("large.txt");
        fs::write(&large, b"abcd").unwrap();
        assert_eq!(
            refusal(&[dir.join("missing.txt")], 0, 10),
            Err(Refusal::Invalid)
        );
        assert_eq!(refusal(&[dir.clone()], 0, 10), Err(Refusal::Invalid));
        assert_eq!(
            refusal(&[small.clone(), large.clone()], MAX_FILES - 1, 10),
            Err(Refusal::TooMany)
        );
        assert_eq!(refusal(&[small.clone()], 0, 2), Err(Refusal::TooLarge(2)));
        assert_eq!(refusal(&[small, large], 8, 4), Ok(()));
        assert_eq!(refusal(&[], 0, 10), Ok(()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn content_types_follow_extensions() {
        assert_eq!(content_type("image.png"), "image/png");
        assert_eq!(content_type("IMAGE.PNG"), "image/png");
        assert_eq!(content_type("photo.jpg"), "image/jpeg");
        assert_eq!(content_type("photo.jpeg"), "image/jpeg");
        assert_eq!(content_type("anim.gif"), "image/gif");
        assert_eq!(content_type("still.webp"), "image/webp");
        assert_eq!(content_type("next.avif"), "image/avif");
        assert_eq!(content_type("old.bmp"), "image/bmp");
        assert_eq!(content_type("clip.mp4"), "video/mp4");
        assert_eq!(content_type("clip.webm"), "video/webm");
        assert_eq!(content_type("clip.mov"), "video/quicktime");
        assert_eq!(content_type("song.mp3"), "audio/mpeg");
        assert_eq!(content_type("song.ogg"), "audio/ogg");
        assert_eq!(content_type("song.wav"), "audio/wav");
        assert_eq!(content_type("notes.txt"), "text/plain");
        assert_eq!(content_type("paper.pdf"), "application/pdf");
        assert_eq!(content_type("bundle.zip"), "application/zip");
        assert_eq!(content_type("data.json"), "application/json");
        assert_eq!(content_type("binary.bin"), "");
        assert_eq!(content_type("README"), "");
    }
}
