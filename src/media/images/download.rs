use super::stills::{StillConversion, downscaled_still, file_name, first_frame_png, still_spec};
use super::urls::{allowed_url, gif_provider_media_url, image_url, rounded_size};
use super::*;

const MAX_DOWNLOAD: u64 = 25 * 1024 * 1024;
const MAX_CACHE: u64 = 512 * 1024 * 1024;
const MAX_VIDEO_CACHE: u64 = 256 * 1024 * 1024;
const MAX_FRAMES_CACHE: u64 = 1536 * 1024 * 1024;
const PRUNE_THRESHOLD: u64 = 64 * 1024 * 1024;
const TOUCH_INTERVAL: Duration = Duration::from_secs(60 * 60);
const PRUNE_MIN_AGE: Duration = Duration::from_secs(60);

pub(super) fn cache_directory<'a>(
    images: &'a Path,
    videos: &'a Path,
    spec: &ImageSpec,
) -> &'a Path {
    if matches!(spec, ImageSpec::Video { .. }) {
        videos
    } else {
        images
    }
}

pub(crate) fn touch_due(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified)
        .is_ok_and(|age| age >= TOUCH_INTERVAL)
}

pub(crate) fn touch_modified(path: &Path) -> std::io::Result<()> {
    std::fs::File::options()
        .write(true)
        .open(path)?
        .set_modified(SystemTime::now())
}

impl Service {
    pub(super) async fn initialize(&self) -> bool {
        *self
            .initialized
            .get_or_init(|| async {
                let directory = self.directory.clone();
                let video_directory = self.video_directory.clone();
                let frames_directory = self.frames_directory.clone();
                let success = tokio::task::spawn_blocking(move || {
                    std::fs::create_dir_all(&directory)?;
                    std::fs::create_dir_all(&video_directory)?;
                    std::fs::create_dir_all(&frames_directory)?;
                    let images = prune(&directory, MAX_CACHE);
                    let videos = prune(&video_directory, MAX_VIDEO_CACHE);
                    let frames = prune(&frames_directory, MAX_FRAMES_CACHE);
                    images?;
                    videos?;
                    frames
                })
                .await
                .is_ok_and(|result| result.is_ok());
                if !success {
                    log::warn!("Image cache directories could not be prepared");
                }
                success
            })
            .await
    }

    pub(crate) fn frames_path(&self, spec: &ImageSpec, size: u32) -> PathBuf {
        self.frames_directory.join(file_name(spec, size))
    }

    pub(crate) async fn load_gif_provider_media(
        &self,
        raw: &str,
        max_bytes: u64,
    ) -> Result<PathBuf, String> {
        let url = reqwest::Url::parse(raw)
            .ok()
            .filter(gif_provider_media_url)
            .ok_or_else(|| "Choose a GIF from Tenor, Giphy, or Klipy.".to_owned())?;
        if !self.initialize().await {
            return Err("Could not prepare the GIF cache.".into());
        }
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| "Could not download this GIF.".to_owned())?;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| "Could not download this GIF.".to_owned())?;
        let bytes = gif_provider_bytes(response, max_bytes).await?;
        let extension = match image::guess_format(&bytes) {
            Ok(ImageFormat::Gif) => "gif",
            Ok(ImageFormat::WebP) => "webp",
            _ => return Err("This GIF format is not supported.".into()),
        };
        let path = self.directory.join(format!(
            "profile-gif-{:016x}.{extension}",
            rand::random::<u64>()
        ));
        atomic_write(&path, &bytes)
            .await
            .map_err(|_| "Could not cache this GIF.".to_owned())?;
        self.record(
            &self.image_written,
            &self.directory,
            MAX_CACHE,
            bytes.len() as u64,
        );
        Ok(path)
    }

    pub(crate) async fn fetch_original(&self, url: &str) -> Option<Vec<u8>> {
        let url = reqwest::Url::parse(url).ok().filter(allowed_url)?;
        let response = self.client.get(url).send().await.ok()?;
        let bytes = response.error_for_status().ok()?.bytes().await.ok()?;
        Some(bytes.to_vec())
    }

    pub(crate) async fn fetch_size(&self, url: &str) -> Option<u64> {
        let url = reqwest::Url::parse(url).ok().filter(allowed_url)?;
        let response = self.client.head(url).send().await.ok()?;
        response.error_for_status().ok()?.content_length()
    }

    pub(crate) async fn load(&self, spec: &ImageSpec, size: u32) -> Option<PathBuf> {
        if let ImageSpec::LocalGif { path } | ImageSpec::LocalVideo { path } = spec {
            return if path.is_file() {
                Some(path.clone())
            } else {
                None
            };
        }
        let url = image_url(spec, size)?;
        if !self.initialize().await {
            return None;
        }
        let video = matches!(spec, ImageSpec::Video { .. });
        let _permit = if video {
            self.video_permits.acquire().await.ok()?
        } else {
            self.permits.acquire().await.ok()?
        };
        let directory = cache_directory(&self.directory, &self.video_directory, spec);
        let path = directory.join(file_name(spec, size));
        if let Ok(metadata) = tokio::fs::metadata(&path).await
            && metadata.is_file()
            && metadata.len() > 0
            && metadata.len() <= MAX_DOWNLOAD
        {
            if video {
                use tokio::io::AsyncReadExt;
                let mut file = tokio::fs::File::open(&path).await.ok()?;
                let mut header = [0; 8];
                file.read_exact(&mut header).await.ok()?;
                if !download_bytes(spec, &header) {
                    return None;
                }
            }
            if let Ok(modified) = metadata.modified()
                && touch_due(modified, SystemTime::now())
            {
                let path = path.clone();
                let _ = tokio::task::spawn_blocking(move || touch_modified(&path)).await;
            }
            return Some(path);
        }
        let mut response = self.client.get(url).send().await.ok()?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|size| size > MAX_DOWNLOAD)
        {
            return None;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if bytes.len().checked_add(chunk.len())? as u64 > MAX_DOWNLOAD {
                return None;
            }
            bytes.extend_from_slice(&chunk);
        }
        if !download_bytes(spec, &bytes) {
            return None;
        }
        let convert: Option<StillConversion> = if still_spec(spec) {
            Some(first_frame_png)
        } else if matches!(spec, ImageSpec::Collectible { .. }) {
            Some(downscaled_still)
        } else {
            None
        };
        let bytes = match convert {
            Some(convert) => {
                let target = rounded_size(size);
                tokio::task::spawn_blocking(move || convert(&bytes, target).unwrap_or(bytes))
                    .await
                    .ok()?
            }
            None => bytes,
        };
        atomic_write(&path, &bytes).await.ok()?;
        self.record_write(spec, bytes.len() as u64);
        Some(path)
    }

    fn record_write(&self, spec: &ImageSpec, bytes: u64) {
        let (counter, directory, limit) = if matches!(spec, ImageSpec::Video { .. }) {
            (&self.video_written, &self.video_directory, MAX_VIDEO_CACHE)
        } else {
            (&self.image_written, &self.directory, MAX_CACHE)
        };
        self.record(counter, directory, limit, bytes);
    }

    pub(crate) fn record_frames_write(&self, bytes: u64) {
        self.record(
            &self.frames_written,
            &self.frames_directory,
            MAX_FRAMES_CACHE,
            bytes,
        );
    }

    fn record(&self, counter: &AtomicU64, directory: &Path, limit: u64, bytes: u64) {
        let total = counter
            .fetch_add(bytes, Ordering::Relaxed)
            .saturating_add(bytes);
        if total < PRUNE_THRESHOLD || self.pruning.swap(true, Ordering::AcqRel) {
            return;
        }
        counter.store(0, Ordering::Relaxed);
        let directory = directory.to_path_buf();
        let pruning = self.pruning.clone();
        drop(tokio::task::spawn_blocking(move || {
            if let Err(error) = prune(&directory, limit) {
                log::warn!(
                    "Image cache could not be pruned at {}: {error}",
                    directory.display()
                );
            }
            pruning.store(false, Ordering::Release);
        }));
    }
}

pub(super) async fn gif_provider_bytes(
    mut response: reqwest::Response,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    if !response.status().is_success() {
        return Err("Could not download this GIF.".into());
    }
    if response
        .content_length()
        .is_some_and(|size| size >= max_bytes)
    {
        return Err("Choose a GIF smaller than 10 MB.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not download this GIF.".to_owned())?
    {
        if bytes.len().saturating_add(chunk.len()) as u64 >= max_bytes {
            return Err("Choose a GIF smaller than 10 MB.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub(super) fn download_bytes(spec: &ImageSpec, bytes: &[u8]) -> bool {
    if matches!(spec, ImageSpec::Video { .. }) {
        bytes.get(4..8) == Some(b"ftyp") || bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3])
    } else {
        image_bytes(bytes)
    }
}

fn image_bytes(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
}

pub(super) async fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension(format!(
        "tmp-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .await?;
        file.write_all(bytes).await?;
        file.flush().await?;
        drop(file);
        tokio::fs::rename(&temp, path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(temp).await;
    }
    result
}

pub(super) fn prune(directory: &Path, limit: u64) -> std::io::Result<()> {
    prune_with(directory, limit, |path| std::fs::remove_file(path))
}

pub(super) fn prune_with(
    directory: &Path,
    limit: u64,
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut files = Vec::new();
    let mut total = 0u64;
    for entry in std::fs::read_dir(directory)? {
        let Ok(entry) = entry else { continue };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        total = total.saturating_add(metadata.len());
        files.push((
            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            metadata.len(),
            entry.path(),
        ));
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    let now = SystemTime::now();
    for (modified, length, path) in files {
        if total <= limit {
            break;
        }
        if now
            .duration_since(modified)
            .map_or(true, |age| age < PRUNE_MIN_AGE)
        {
            continue;
        }
        if remove(&path).is_err() {
            continue;
        }
        total = total.saturating_sub(length);
    }
    Ok(())
}
