mod blur;
mod download;
mod stills;
mod urls;

use blur::BLURRED;
pub use blur::{blurred, blurred_at_width, blurred_with};
pub(crate) use download::{touch_due, touch_modified};
pub use urls::profile_frame_layer_url;
pub(crate) use urls::{allowed_url, avatar_sample_url, rounded_size, trusted_video_url};

use gpui::{App, Global, Task};
use gpui_tokio::Tokio;
use image::{AnimationDecoder, ImageEncoder, ImageFormat};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    io::Cursor,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{OnceCell, Semaphore},
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ImageSpec {
    GuildIcon {
        guild_id: u64,
        hash: String,
    },
    ChannelIcon {
        channel_id: u64,
        hash: String,
    },
    Avatar {
        user_id: u64,
        hash: String,
    },
    DefaultAvatar {
        index: u8,
    },
    /// One of your earlier uploads, from `/users/@me/avatars`.
    ArchivedAvatar {
        user_id: u64,
        avatar_id: u64,
        hash: String,
    },
    Attachment {
        id: u64,
        url: String,
        width: u32,
        height: u32,
    },
    GuildAvatar {
        guild_id: u64,
        user_id: u64,
        hash: String,
    },
    Emoji {
        id: u64,
        animated: bool,
    },
    Sticker {
        id: u64,
        /// Discord's `format_type`: 1 = PNG, 2 = APNG, 3 = Lottie, 4 = GIF.
        format: u8,
    },
    GuildTagBadge {
        guild_id: u64,
        hash: String,
    },
    RoleIcon {
        role_id: u64,
        hash: String,
    },
    Media {
        url: String,
        width: u32,
        height: u32,
        animated: bool,
    },
    Video {
        url: String,
        width: u32,
        height: u32,
    },
    VideoPoster {
        id: u64,
        url: String,
        width: u32,
        height: u32,
    },
    LocalGif {
        path: PathBuf,
    },
    LocalVideo {
        path: PathBuf,
    },
    AvatarDecoration {
        asset: String,
        animated: bool,
    },
    Nameplate {
        asset: String,
        animated: bool,
    },
    Banner {
        user_id: u64,
        hash: String,
    },
    GuildBanner {
        guild_id: u64,
        hash: String,
    },
    /// A guild's invite splash (`splashes/{guild_id}/{hash}.png`).
    GuildSplash {
        guild_id: u64,
        hash: String,
    },
    /// Also where an invite profile's `custom_banner_hash` lives.
    DiscoverySplash {
        guild_id: u64,
        hash: String,
    },
    GuildMemberBanner {
        guild_id: u64,
        user_id: u64,
        hash: String,
    },
    Badge {
        icon: String,
    },
    Collectible {
        url: String,
    },
    /// Application icons and covers (`app-icons/`) or uploaded widget assets (`app-assets/`).
    AppImage {
        application_id: u64,
        hash: String,
        uploaded: bool,
        animated: bool,
    },
    /// A game's portrait cover (`app-icons/{application_id}/{hash}`). The CDN crops these to a
    /// square unless asked to keep their aspect ratio.
    AppCover {
        application_id: u64,
        hash: String,
    },
    /// SKU store art (`app-assets/{application_id}/store/{asset_id}`), e.g. wishlist cards.
    StoreAsset {
        application_id: u64,
        asset_id: u64,
    },
    Animated {
        still: Box<ImageSpec>,
    },
}

impl ImageSpec {
    pub fn animated(&self) -> Option<ImageSpec> {
        let hash = match self {
            ImageSpec::GuildIcon { hash, .. }
            | ImageSpec::Avatar { hash, .. }
            | ImageSpec::ArchivedAvatar { hash, .. }
            | ImageSpec::GuildAvatar { hash, .. }
            | ImageSpec::Banner { hash, .. }
            | ImageSpec::GuildBanner { hash, .. }
            | ImageSpec::GuildMemberBanner { hash, .. } => hash,
            // Uploaded app assets carry no `a_` prefix; the widget config flags animated ones.
            ImageSpec::AppImage { animated: true, .. } => {
                return Some(ImageSpec::Animated {
                    still: Box::new(self.clone()),
                });
            }
            _ => return None,
        };
        hash.starts_with("a_").then(|| ImageSpec::Animated {
            still: Box::new(self.clone()),
        })
    }
}

enum Entry {
    Pending,
    Ready(Arc<Path>),
    Failed,
}
struct Cache {
    entries: HashMap<ImageSpec, HashMap<u32, Entry>>,
    service: Option<Arc<Service>>,
    refresh_pending: bool,
    generation: u64,
}
struct ImageCache(Rc<RefCell<Cache>>);
impl Global for ImageCache {}

impl Cache {
    fn clear(&mut self) {
        self.entries.clear();
        self.generation = self.generation.wrapping_add(1);
    }
}

pub(crate) struct Service {
    client: reqwest::Client,
    directory: PathBuf,
    video_directory: PathBuf,
    frames_directory: PathBuf,
    permits: Semaphore,
    video_permits: Semaphore,
    initialized: OnceCell<bool>,
    image_written: AtomicU64,
    video_written: AtomicU64,
    frames_written: AtomicU64,
    pruning: Arc<AtomicBool>,
}

pub fn init(cx: &mut App) {
    if cx.try_global::<ImageCache>().is_some() {
        return;
    }
    let directories = dirs::cache_dir().map(|dir| {
        let root = dir.join("zcord");
        (
            root.join("images"),
            root.join("videos"),
            root.join("frames"),
        )
    });
    let client = reqwest::Client::builder()
        .user_agent(crate::discord::user_agent())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(45))
        .build()
        .ok();
    let service =
        directories
            .zip(client)
            .map(|((directory, video_directory, frames_directory), client)| {
                Arc::new(Service {
                    client,
                    directory,
                    video_directory,
                    frames_directory,
                    permits: Semaphore::new(6),
                    video_permits: Semaphore::new(2),
                    initialized: OnceCell::new(),
                    image_written: AtomicU64::new(0),
                    video_written: AtomicU64::new(0),
                    frames_written: AtomicU64::new(0),
                    pruning: Arc::new(AtomicBool::new(false)),
                })
            });
    if let Some(service) = service.clone() {
        Tokio::spawn(cx, async move {
            service.initialize().await;
        })
        .detach();
    } else {
        log::warn!("Image cache could not initialize");
    }
    cx.set_global(ImageCache(Rc::new(RefCell::new(Cache {
        entries: HashMap::new(),
        service,
        refresh_pending: false,
        generation: 0,
    }))));
    #[cfg(windows)]
    crate::media::video::init(cx);
}

pub(crate) fn download_service(cx: &App) -> Option<Arc<Service>> {
    cx.try_global::<ImageCache>()?.0.borrow().service.clone()
}

fn cache_directories() -> Option<(PathBuf, PathBuf, PathBuf)> {
    dirs::cache_dir().map(|dir| {
        let root = dir.join("zcord");
        (
            root.join("images"),
            root.join("videos"),
            root.join("frames"),
        )
    })
}

fn clear_cache_dirs(images: &Path, videos: &Path, frames: &Path) -> Result<u64, String> {
    let mut freed = 0u64;
    for directory in [images, videos, frames] {
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("Could not read {}: {error}", directory.display()));
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    log::warn!("Could not read a cache entry: {error}");
                    continue;
                }
            };
            let path = entry.path();
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let size = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            match std::fs::remove_file(&path) {
                Ok(()) => freed = freed.saturating_add(size),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    log::warn!("Could not delete cached file {}: {error}", path.display());
                }
            }
        }
    }
    Ok(freed)
}

pub fn clear_cache(cx: &mut App) -> Task<Result<u64, String>> {
    let cache = cx.try_global::<ImageCache>().map(|cache| cache.0.clone());
    let directories = cache_directories();
    cx.spawn(async move |cx| {
        let result = match directories {
            Some((images, videos, frames)) => {
                cx.background_executor()
                    .spawn(async move { clear_cache_dirs(&images, &videos, &frames) })
                    .await
            }
            None => Ok(0),
        };
        cx.update(move |cx| {
            if let Some(cache) = &cache {
                cache.borrow_mut().clear();
            }
            BLURRED.with(|cache| cache.borrow_mut().clear());
            cx.refresh_windows();
        });
        result
    })
}

/// True once a load of `spec` at `size` has given up, so callers stop showing a spinner.
pub fn failed(spec: &ImageSpec, size: u32, cx: &App) -> bool {
    let Some(cache) = cx.try_global::<ImageCache>() else {
        return true;
    };
    let state = cache.0.borrow();
    state.service.is_none()
        || state
            .entries
            .get(spec)
            .and_then(|sizes| sizes.get(&rounded_size(size)))
            .is_some_and(|entry| matches!(entry, Entry::Failed))
}

pub fn resolve(spec: &ImageSpec, size: u32, cx: &App) -> Option<Arc<Path>> {
    let cache = cx.try_global::<ImageCache>()?.0.clone();
    let size = rounded_size(size);
    let (service, generation) = {
        let mut state = cache.borrow_mut();
        if let Some(entry) = state.entries.get(spec).and_then(|sizes| sizes.get(&size)) {
            return match entry {
                Entry::Ready(path) => Some(path.clone()),
                _ => None,
            };
        }
        let service = state.service.clone();
        state.entries.entry(spec.clone()).or_default().insert(
            size,
            if service.is_some() {
                Entry::Pending
            } else {
                Entry::Failed
            },
        );
        (service, state.generation)
    };
    let service = service?;
    let spec = spec.clone();
    let request = spec.clone();
    let task = Tokio::spawn(cx, async move { service.load(&request, size).await });
    cx.spawn(async move |cx| {
        let path = task.await.ok().flatten();
        if cache.borrow().generation != generation {
            return;
        }
        let refresh = {
            let mut state = cache.borrow_mut();
            let success = path.is_some();
            let entry = path.map_or(Entry::Failed, |path| Entry::Ready(Arc::from(path)));
            state.entries.entry(spec).or_default().insert(size, entry);
            if success && !state.refresh_pending {
                state.refresh_pending = true;
                true
            } else {
                false
            }
        };
        if refresh {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            cache.borrow_mut().refresh_pending = false;
            cx.update(|cx| cx.refresh_windows());
        }
    })
    .detach();
    None
}

#[cfg(test)]
mod tests;
