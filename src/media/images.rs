use gpui::{App, Global};
use gpui_tokio::Tokio;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
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

const MAX_DOWNLOAD: u64 = 25 * 1024 * 1024;
const MAX_CACHE: u64 = 512 * 1024 * 1024;
const MAX_VIDEO_CACHE: u64 = 256 * 1024 * 1024;
const PRUNE_THRESHOLD: u64 = 64 * 1024 * 1024;
/// Cache hits older than this get their mtime refreshed so pruning stays LRU.
const TOUCH_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Leave freshly written files alone; a reader may be about to open them.
const PRUNE_MIN_AGE: Duration = Duration::from_secs(60);

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

const COLLECTIBLES_SHOP: &str = "https://cdn.discordapp.com/media/v1/collectibles-shop/";
const COLLECTIBLE_PATHS: [&str; 2] = ["/media/v1/collectibles-shop/", "/assets/content/"];

fn valid_hash(hash: &str) -> bool {
    !hash.is_empty()
        && hash.len() <= 128
        && hash.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn valid_nameplate(asset: &str) -> bool {
    asset.len() <= 256
        && asset.starts_with("nameplates/")
        && asset.ends_with('/')
        && !asset.contains("//")
        && asset
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'/'))
}

pub fn profile_frame_layer_url(product_sku: u64, layer_id: u64) -> String {
    format!("{COLLECTIBLES_SHOP}{product_sku}/{layer_id}/static")
}

pub(crate) fn rounded_size(size: u32) -> u32 {
    size.clamp(16, 4096).next_power_of_two()
}

pub(crate) fn allowed_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            matches!(host, "cdn.discordapp.com" | "media.discordapp.net")
                || host
                    .strip_prefix("images-ext-")
                    .and_then(|rest| rest.strip_suffix(".discordapp.net"))
                    .is_some_and(|digits| {
                        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                    })
        })
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

/// Spotify album art comes from a host outside Discord's CDN; only images may load it.
fn spotify_image_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("i.scdn.co")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

/// GIF picker media is served by the providers' CDNs, outside Discord's proxy.
fn gif_provider_media_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            matches!(
                host,
                "static.klipy.com"
                    | "media.tenor.com"
                    | "media.giphy.com"
                    | "i.giphy.com"
                    | "media0.giphy.com"
                    | "media1.giphy.com"
                    | "media2.giphy.com"
                    | "media3.giphy.com"
                    | "media4.giphy.com"
            )
        })
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

pub(crate) fn trusted_video_url(raw: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(raw).ok()?;
    let extension = url
        .path()
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase());
    let video_file = matches!(extension.as_deref(), Some("mp4" | "mov" | "webm" | "m4v"));
    // Discord's external proxy only serves media it already resolved, and redirect-style
    // sources (FxTwitter's `/2/go?url=…mp4`) end up with an extensionless proxy path.
    let proxied = url
        .host_str()
        .is_some_and(|host| host.starts_with("images-ext-"));
    let discord = allowed_url(&url) && (video_file || proxied);
    // Official-client favorite GIFs can be videos on the providers' CDNs.
    let provider =
        matches!(extension.as_deref(), Some("mp4" | "webm")) && gif_provider_media_url(&url);
    if !discord && !provider {
        return None;
    }
    url.set_fragment(None);
    Some(url.into())
}

fn image_url(spec: &ImageSpec, size: u32) -> Option<reqwest::Url> {
    if let ImageSpec::Animated { still } = spec {
        still.animated()?;
        let mut url = image_url(still, size)?;
        let path = url
            .path()
            .strip_suffix(".webp")
            .or_else(|| url.path().strip_suffix(".png"))?
            .to_owned();
        url.set_path(&format!("{path}.webp"));
        url.query_pairs_mut().append_pair("animated", "true");
        return Some(url);
    }
    let size = rounded_size(size);
    let raw = match spec {
        ImageSpec::GuildAvatar {
            guild_id,
            user_id,
            hash,
        } => {
            if !valid_hash(hash) {
                return None;
            }
            format!(
                "https://cdn.discordapp.com/guilds/{guild_id}/users/{user_id}/avatars/{hash}.png?size={size}"
            )
        }
        // WebP also covers uploaded AVIF/WebP emoji. https://docs.discord.food/resources/emoji#emoji-formats
        ImageSpec::Emoji { id, animated } => {
            format!("https://cdn.discordapp.com/emojis/{id}.webp?size={size}&animated={animated}")
        }
        ImageSpec::Sticker { id, format } => {
            // APNG stickers are served from the same `.png` URL as static PNGs;
            // the full file animates once decoded.
            let extension = match format {
                1 | 2 => "png",
                4 => "gif",
                _ => return None,
            };
            format!("https://media.discordapp.net/stickers/{id}.{extension}?size={size}")
        }
        ImageSpec::GuildIcon { guild_id, hash }
        | ImageSpec::GuildTagBadge { guild_id, hash }
        | ImageSpec::ChannelIcon {
            channel_id: guild_id,
            hash,
        }
        | ImageSpec::Avatar {
            user_id: guild_id,
            hash,
        } => {
            if !valid_hash(hash) {
                return None;
            }
            let route = match spec {
                ImageSpec::GuildIcon { .. } => "icons",
                ImageSpec::GuildTagBadge { .. } => "guild-tag-badges",
                ImageSpec::ChannelIcon { .. } => "channel-icons",
                _ => "avatars",
            };
            format!("https://cdn.discordapp.com/{route}/{guild_id}/{hash}.png?size={size}")
        }
        ImageSpec::DefaultAvatar { index } => {
            if *index > 5 {
                return None;
            }
            format!("https://cdn.discordapp.com/embed/avatars/{index}.png")
        }
        ImageSpec::AvatarDecoration { asset, animated } => {
            if !valid_hash(asset) {
                return None;
            }
            // Only passthrough serves the animated APNG, and only `a_` presets have one.
            let passthrough = *animated && asset.starts_with("a_");
            format!(
                "https://cdn.discordapp.com/avatar-decoration-presets/{asset}.png?size={size}&passthrough={passthrough}"
            )
        }
        // `img.png` is the APNG twin of the webm Discord's client plays.
        ImageSpec::Nameplate { asset, animated } => {
            if !valid_nameplate(asset) {
                return None;
            }
            let file = if *animated { "img" } else { "static" };
            format!("https://cdn.discordapp.com/assets/collectibles/{asset}{file}.png")
        }
        ImageSpec::Banner { user_id, hash } => {
            if !valid_hash(hash) {
                return None;
            }
            format!("https://cdn.discordapp.com/banners/{user_id}/{hash}.png?size={size}")
        }
        ImageSpec::GuildBanner { guild_id, hash } => {
            if !valid_hash(hash) {
                return None;
            }
            format!("https://cdn.discordapp.com/banners/{guild_id}/{hash}.webp?size={size}")
        }
        ImageSpec::GuildSplash { guild_id, hash } => {
            if !valid_hash(hash) {
                return None;
            }
            format!("https://cdn.discordapp.com/splashes/{guild_id}/{hash}.png?size={size}")
        }
        ImageSpec::DiscoverySplash { guild_id, hash } => {
            if !valid_hash(hash) {
                return None;
            }
            format!(
                "https://cdn.discordapp.com/discovery-splashes/{guild_id}/{hash}.webp?size={size}"
            )
        }
        ImageSpec::GuildMemberBanner {
            guild_id,
            user_id,
            hash,
        } => {
            if !valid_hash(hash) {
                return None;
            }
            format!(
                "https://cdn.discordapp.com/guilds/{guild_id}/users/{user_id}/banners/{hash}.png?size={size}"
            )
        }
        ImageSpec::AppImage {
            application_id,
            hash,
            uploaded,
            ..
        } => {
            if !valid_hash(hash) {
                return None;
            }
            let (route, extension) = if *uploaded {
                ("app-assets", "webp")
            } else {
                ("app-icons", "png")
            };
            format!(
                "https://cdn.discordapp.com/{route}/{application_id}/{hash}.{extension}?size={size}"
            )
        }
        ImageSpec::AppCover {
            application_id,
            hash,
        } => {
            if !valid_hash(hash) {
                return None;
            }
            format!(
                "https://cdn.discordapp.com/app-icons/{application_id}/{hash}.png?size={size}&keep_aspect_ratio=true"
            )
        }
        ImageSpec::StoreAsset {
            application_id,
            asset_id,
        } => format!(
            "https://cdn.discordapp.com/app-assets/{application_id}/store/{asset_id}.webp?size={size}"
        ),
        ImageSpec::Badge { icon } => {
            if !valid_hash(icon) {
                return None;
            }
            format!("https://cdn.discordapp.com/badge-icons/{icon}.png")
        }
        ImageSpec::Attachment { url, .. }
        | ImageSpec::Media { url, .. }
        | ImageSpec::Video { url, .. }
        | ImageSpec::VideoPoster { url, .. }
        | ImageSpec::Collectible { url } => url.clone(),
        ImageSpec::Animated { .. } => return None,
    };
    let mut url = reqwest::Url::parse(&raw).ok()?;
    if !allowed_url(&url) && !spotify_image_url(&url) && !gif_provider_media_url(&url) {
        return None;
    }
    if matches!(spec, ImageSpec::Collectible { .. })
        && (url.host_str() != Some("cdn.discordapp.com")
            || !COLLECTIBLE_PATHS
                .iter()
                .any(|prefix| url.path().starts_with(prefix)))
    {
        return None;
    }
    url.set_fragment(None);
    if let ImageSpec::Attachment { width, height, .. }
    | ImageSpec::Media { width, height, .. }
    | ImageSpec::VideoPoster { width, height, .. } = spec
    {
        let poster = matches!(spec, ImageSpec::VideoPoster { .. });
        let longest = (*width).max(*height);
        if *width == 0 || *height == 0 {
            if matches!(
                spec,
                ImageSpec::Attachment { .. } | ImageSpec::VideoPoster { .. }
            ) {
                return None;
            }
            let params: Vec<_> = url
                .query_pairs()
                .filter(|(key, _)| !matches!(key.as_ref(), "width" | "height" | "size"))
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
            url.set_query(None);
            if !params.is_empty() {
                url.query_pairs_mut().extend_pairs(params);
            }
            if matches!(spec, ImageSpec::Media { animated: true, .. }) {
                url.query_pairs_mut().append_pair("animated", "true");
            }
            return Some(url);
        }
        let target = size.min(longest);
        let width = (u64::from(*width) * u64::from(target) / u64::from(longest)).max(1);
        let height = (u64::from(*height) * u64::from(target) / u64::from(longest)).max(1);
        let params: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| {
                !matches!(key.as_ref(), "width" | "height" | "size") && !(poster && key == "format")
            })
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        // The media proxy keeps the original format (and GIF animation); WebP conversion isn't
        // documented as guaranteed. https://docs.discord.food/reference#media-proxy-parameters
        url.set_query(None);
        url.query_pairs_mut()
            .extend_pairs(params)
            .append_pair("width", &width.to_string())
            .append_pair("height", &height.to_string());
        if matches!(spec, ImageSpec::Media { animated: true, .. }) {
            url.query_pairs_mut().append_pair("animated", "true");
        }
        if poster {
            url.query_pairs_mut().append_pair("format", "webp");
        }
    }
    Some(url)
}

fn file_name(spec: &ImageSpec, size: u32) -> String {
    let mut hasher = DefaultHasher::new();
    spec.hash(&mut hasher);
    format!("{:016x}-{}.img", hasher.finish(), rounded_size(size))
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
}
struct ImageCache(Rc<RefCell<Cache>>);
impl Global for ImageCache {}

pub(crate) struct Service {
    client: reqwest::Client,
    directory: PathBuf,
    video_directory: PathBuf,
    permits: Semaphore,
    video_permits: Semaphore,
    initialized: OnceCell<bool>,
    image_written: AtomicU64,
    video_written: AtomicU64,
    pruning: Arc<AtomicBool>,
}

pub fn init(cx: &mut App) {
    if cx.try_global::<ImageCache>().is_some() {
        return;
    }
    let directories = dirs::cache_dir().map(|dir| {
        let root = dir.join("zcord");
        (root.join("images"), root.join("videos"))
    });
    let client = reqwest::Client::builder()
        .user_agent(crate::discord::USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(45))
        .build()
        .ok();
    let service = directories
        .zip(client)
        .map(|((directory, video_directory), client)| {
            Arc::new(Service {
                client,
                directory,
                video_directory,
                permits: Semaphore::new(6),
                video_permits: Semaphore::new(2),
                initialized: OnceCell::new(),
                image_written: AtomicU64::new(0),
                video_written: AtomicU64::new(0),
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
    }))));
    #[cfg(windows)]
    crate::media::video::init(cx);
}

pub(crate) fn download_service(cx: &App) -> Option<Arc<Service>> {
    cx.try_global::<ImageCache>()?.0.borrow().service.clone()
}

pub fn resolve(spec: &ImageSpec, size: u32, cx: &App) -> Option<Arc<Path>> {
    let cache = cx.try_global::<ImageCache>()?.0.clone();
    let size = rounded_size(size);
    let service = {
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
        service
    }?;
    let spec = spec.clone();
    let request = spec.clone();
    let task = Tokio::spawn(cx, async move { service.load(&request, size).await });
    cx.spawn(async move |cx| {
        let path = task.await.ok().flatten();
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

const BLUR_SOURCE: u32 = 64;
const BLUR_SIGMA: f32 = 5.;
const BLUR_CACHE_LIMIT: usize = 64;
type BlurKey = (Arc<Path>, u32, u32);

#[derive(Default)]
struct BlurCache {
    entries: HashMap<BlurKey, Option<Arc<gpui::RenderImage>>>,
    order: VecDeque<BlurKey>,
}

pub fn blurred(spec: &ImageSpec, cx: &App) -> Option<Arc<gpui::RenderImage>> {
    blurred_with(spec, BLUR_SOURCE, BLUR_SIGMA, 1., cx)
}

pub fn blurred_with(
    spec: &ImageSpec,
    source: u32,
    sigma: f32,
    saturation: f32,
    cx: &App,
) -> Option<Arc<gpui::RenderImage>> {
    thread_local! {
        static BLURRED: RefCell<BlurCache> = RefCell::new(BlurCache::default());
    }
    let path = resolve(spec, source, cx)?;
    let key = (path, sigma.to_bits(), saturation.to_bits());
    BLURRED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(image) = cache.entries.get(&key) {
            return image.clone();
        }
        // Live render elements keep their own Arc, so evicting here only drops the cache's
        // reference; dropping the atlas tiles of a still-painted image would force re-uploads.
        if cache.entries.len() >= BLUR_CACHE_LIMIT
            && let Some(oldest) = cache.order.pop_front()
        {
            cache.entries.remove(&oldest);
        }
        let image = blur_file(&key.0, sigma, saturation);
        cache.order.push_back(key.clone());
        cache.entries.insert(key, image.clone());
        image
    })
}

fn blur_file(path: &Path, sigma: f32, saturation: f32) -> Option<Arc<gpui::RenderImage>> {
    let bytes = std::fs::read(path).ok()?;
    let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let mut blurred = image::imageops::fast_blur(&image, sigma);
    for pixel in blurred.pixels_mut() {
        let [r, g, b, _] = pixel.0.map(f32::from);
        let gray = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let saturate = |channel: f32| (gray + (channel - gray) * saturation).round() as u8;
        pixel.0 = [saturate(b), saturate(g), saturate(r), 255];
    }
    Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
        blurred,
    )])))
}

fn cache_directory<'a>(images: &'a Path, videos: &'a Path, spec: &ImageSpec) -> &'a Path {
    if matches!(spec, ImageSpec::Video { .. }) {
        videos
    } else {
        images
    }
}

fn touch_due(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified)
        .is_ok_and(|age| age >= TOUCH_INTERVAL)
}

fn touch_modified(path: &Path) -> std::io::Result<()> {
    std::fs::File::options()
        .write(true)
        .open(path)?
        .set_modified(SystemTime::now())
}

impl Service {
    async fn initialize(&self) -> bool {
        *self
            .initialized
            .get_or_init(|| async {
                let directory = self.directory.clone();
                let video_directory = self.video_directory.clone();
                let success = tokio::task::spawn_blocking(move || {
                    std::fs::create_dir_all(&directory)?;
                    std::fs::create_dir_all(&video_directory)?;
                    // Run both even if the first fails, so neither directory
                    // can grow unbounded because the other one errored.
                    let images = prune(&directory, MAX_CACHE);
                    let videos = prune(&video_directory, MAX_VIDEO_CACHE);
                    images?;
                    videos
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

    pub(crate) async fn load(&self, spec: &ImageSpec, size: u32) -> Option<PathBuf> {
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
        let total = counter
            .fetch_add(bytes, Ordering::Relaxed)
            .saturating_add(bytes);
        if total < PRUNE_THRESHOLD || self.pruning.swap(true, Ordering::AcqRel) {
            return;
        }
        counter.store(0, Ordering::Relaxed);
        let directory = directory.clone();
        let pruning = self.pruning.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(error) = prune(&directory, limit) {
                log::warn!(
                    "Image cache could not be pruned at {}: {error}",
                    directory.display()
                );
            }
            pruning.store(false, Ordering::Release);
        });
    }
}

fn download_bytes(spec: &ImageSpec, bytes: &[u8]) -> bool {
    if matches!(spec, ImageSpec::Video { .. }) {
        bytes.get(4..8) == Some(b"ftyp")
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

async fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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

fn prune(directory: &Path, limit: u64) -> std::io::Result<()> {
    prune_with(directory, limit, |path| std::fs::remove_file(path))
}

fn prune_with(
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
        // A file modified just now may still be opened by the UI that requested
        // it; leave it for a later prune.
        if now
            .duration_since(modified)
            .map_or(true, |age| age < PRUNE_MIN_AGE)
        {
            continue;
        }
        // A reader (or the platform) can prevent removal. Skip it and keep
        // pruning the rest instead of abandoning the whole pass.
        if remove(&path).is_err() {
            continue;
        }
        total = total.saturating_sub(length);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "zcord-image-{name}-test-{:016x}",
            rand::random::<u64>()
        ))
    }

    fn set_age(path: &Path, age: Duration) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }

    #[test]
    fn video_specs_resolve_into_videos_dir() {
        let images = Path::new("images");
        let videos = Path::new("videos");
        let video = ImageSpec::Video {
            url: "https://cdn.discordapp.com/attachments/1/2/video.mp4".into(),
            width: 640,
            height: 360,
        };
        assert_eq!(cache_directory(images, videos, &video), videos);
        let poster = ImageSpec::VideoPoster {
            id: 1,
            url: "https://cdn.discordapp.com/attachments/1/2/poster.jpg".into(),
            width: 640,
            height: 360,
        };
        assert_eq!(cache_directory(images, videos, &poster), images);
    }

    #[tokio::test]
    async fn video_cache_hit_uses_videos_dir_and_refreshes_mtime() {
        let root = temp_directory("video-hit");
        let videos = root.join("videos");
        std::fs::create_dir_all(&videos).unwrap();
        let video = ImageSpec::Video {
            url: "https://cdn.discordapp.com/attachments/1/2/video.mp4".into(),
            width: 640,
            height: 360,
        };
        let path = videos.join(file_name(&video, 64));
        std::fs::write(&path, b"\0\0\0\x18ftypisom\0\0\0\0").unwrap();
        set_age(&path, Duration::from_secs(2 * 60 * 60));

        let service = Service {
            client: reqwest::Client::new(),
            directory: root.join("images"),
            video_directory: videos,
            permits: tokio::sync::Semaphore::new(6),
            video_permits: tokio::sync::Semaphore::new(2),
            initialized: tokio::sync::OnceCell::new(),
            image_written: AtomicU64::new(0),
            video_written: AtomicU64::new(0),
            pruning: Arc::new(AtomicBool::new(false)),
        };
        let loaded = service.load(&video, 64).await.unwrap();
        assert_eq!(&loaded, &path);
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert!(
            modified
                .elapsed()
                .is_ok_and(|age| age < Duration::from_secs(5))
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn prune_skips_unremovable_and_recent_files_and_continues() {
        let directory = temp_directory("skip");
        std::fs::create_dir_all(&directory).unwrap();
        let locked = directory.join("locked.img");
        let removable = directory.join("removable.img");
        let recent = directory.join("recent.img");
        for path in [&locked, &removable, &recent] {
            std::fs::write(path, b"12345").unwrap();
        }
        set_age(&locked, Duration::from_secs(3 * 60 * 60));
        set_age(&removable, Duration::from_secs(2 * 60 * 60));
        set_age(&recent, Duration::from_secs(30));

        prune_with(&directory, 0, |path| {
            if path == locked.as_path() {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "in use",
                ))
            } else {
                std::fs::remove_file(path)
            }
        })
        .unwrap();

        assert!(locked.exists());
        assert!(!removable.exists());
        assert!(recent.exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn lru_touch_protects_a_cache_hit_from_pruning() {
        let directory = temp_directory("lru");
        std::fs::create_dir_all(&directory).unwrap();
        let touched = directory.join("touched.img");
        let untouched = directory.join("untouched.img");
        std::fs::write(&touched, b"12345").unwrap();
        std::fs::write(&untouched, b"12345").unwrap();
        set_age(&touched, Duration::from_secs(2 * 60 * 60));
        set_age(&untouched, Duration::from_secs(60 * 60));

        let now = SystemTime::now();
        assert!(touch_due(now - Duration::from_secs(2 * 60 * 60), now));
        assert!(!touch_due(now - Duration::from_secs(30 * 60), now));
        touch_modified(&touched).unwrap();

        prune(&directory, 8).unwrap();

        assert!(touched.exists());
        assert!(!untouched.exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn profile_cosmetic_urls() {
        let url = |spec: ImageSpec, size| image_url(&spec, size).map(String::from);
        let decoration = |asset: &str, animated| ImageSpec::AvatarDecoration {
            asset: asset.into(),
            animated,
        };
        assert_eq!(
            url(decoration("a_f0000000000000000000000000000016", true), 96).as_deref(),
            Some(
                "https://cdn.discordapp.com/avatar-decoration-presets/a_f0000000000000000000000000000016.png?size=128&passthrough=true"
            )
        );
        assert_eq!(
            url(decoration("a_f0000000000000000000000000000016", false), 96).as_deref(),
            Some(
                "https://cdn.discordapp.com/avatar-decoration-presets/a_f0000000000000000000000000000016.png?size=128&passthrough=false"
            )
        );
        assert_eq!(
            url(decoration("f000000000000015", true), 64).as_deref(),
            Some(
                "https://cdn.discordapp.com/avatar-decoration-presets/f000000000000015.png?size=64&passthrough=false"
            )
        );
        let nameplate = "nameplates/sample_plate_one/1191169245577216079/";
        for (animated, file) in [(false, "static"), (true, "img")] {
            let spec = ImageSpec::Nameplate {
                asset: nameplate.into(),
                animated,
            };
            assert_eq!(
                url(spec, 0),
                Some(format!(
                    "https://cdn.discordapp.com/assets/collectibles/nameplates/sample_plate_one/1191169245577216079/{file}.png"
                ))
            );
        }
        assert_eq!(
            url(
                ImageSpec::Banner {
                    user_id: 1,
                    hash: "a_ff".into()
                },
                600
            )
            .as_deref(),
            Some("https://cdn.discordapp.com/banners/1/a_ff.png?size=1024")
        );
        assert_eq!(
            url(
                ImageSpec::GuildMemberBanner {
                    guild_id: 2,
                    user_id: 1,
                    hash: "ff".into()
                },
                300
            )
            .as_deref(),
            Some("https://cdn.discordapp.com/guilds/2/users/1/banners/ff.png?size=512")
        );
        assert_eq!(
            url(
                ImageSpec::GuildBanner {
                    guild_id: 2,
                    hash: "ff".into()
                },
                300
            )
            .as_deref(),
            Some("https://cdn.discordapp.com/banners/2/ff.webp?size=512")
        );
        let guild_icon = ImageSpec::GuildIcon {
            guild_id: 2,
            hash: "a_ff".into(),
        };
        assert_eq!(
            url(guild_icon.animated().unwrap(), 48).as_deref(),
            Some("https://cdn.discordapp.com/icons/2/a_ff.webp?size=64&animated=true")
        );
        let guild_banner = ImageSpec::GuildBanner {
            guild_id: 2,
            hash: "a_ff".into(),
        };
        assert_eq!(
            url(guild_banner.animated().unwrap(), 300).as_deref(),
            Some("https://cdn.discordapp.com/banners/2/a_ff.webp?size=512&animated=true")
        );
        assert!(
            ImageSpec::GuildBanner {
                guild_id: 2,
                hash: "not/a/hash".into()
            }
            .animated()
            .is_none()
        );
        assert_eq!(
            url(
                ImageSpec::GuildSplash {
                    guild_id: 2,
                    hash: "ff".into()
                },
                512
            )
            .as_deref(),
            Some("https://cdn.discordapp.com/splashes/2/ff.png?size=512")
        );
        assert_eq!(
            url(
                ImageSpec::Badge {
                    icon: "f0000000000000000000000000000001".into()
                },
                16
            )
            .as_deref(),
            Some("https://cdn.discordapp.com/badge-icons/f0000000000000000000000000000001.png")
        );
        let frame = profile_frame_layer_url(1191169253965824081, 1191169279131648087);
        assert_eq!(
            frame,
            "https://cdn.discordapp.com/media/v1/collectibles-shop/1191169253965824081/1191169279131648087/static"
        );
        assert_eq!(
            url(ImageSpec::Collectible { url: frame.clone() }, 0),
            Some(frame)
        );
        let banner = ImageSpec::Banner {
            user_id: 1,
            hash: "a_ff".into(),
        };
        assert_eq!(
            url(banner.animated().unwrap(), 480).as_deref(),
            Some("https://cdn.discordapp.com/banners/1/a_ff.webp?size=512&animated=true")
        );
        let avatar = ImageSpec::GuildAvatar {
            guild_id: 2,
            user_id: 1,
            hash: "a_ee".into(),
        };
        assert_eq!(
            url(avatar.animated().unwrap(), 80).as_deref(),
            Some(
                "https://cdn.discordapp.com/guilds/2/users/1/avatars/a_ee.webp?size=128&animated=true"
            )
        );
        let still = ImageSpec::Avatar {
            user_id: 1,
            hash: "ff".into(),
        };
        assert!(still.animated().is_none());
        assert!(
            image_url(
                &ImageSpec::Animated {
                    still: Box::new(still)
                },
                80
            )
            .is_none()
        );
        let effect = "https://cdn.discordapp.com/assets/content/f0000000000000000000000000000000000000000000002b";
        assert_eq!(
            url(ImageSpec::Collectible { url: effect.into() }, 0).as_deref(),
            Some(effect)
        );
    }

    #[test]
    fn profile_cosmetic_urls_reject_untrusted_input() {
        for asset in ["", "a_../x", "a_x?size=1", "a_x#y", "a/b", &"a".repeat(129)] {
            let spec = ImageSpec::AvatarDecoration {
                asset: asset.into(),
                animated: true,
            };
            assert!(image_url(&spec, 96).is_none(), "{asset}");
            assert!(
                image_url(&ImageSpec::Badge { icon: asset.into() }, 16).is_none(),
                "{asset}"
            );
        }
        for asset in [
            "nameplates/../x/",
            "nameplates/a/b",
            "harvest/a/",
            "nameplates//a/",
            "nameplates/a?b/",
            "nameplates/a.b/",
        ] {
            for animated in [false, true] {
                let spec = ImageSpec::Nameplate {
                    asset: asset.into(),
                    animated,
                };
                assert!(image_url(&spec, 0).is_none(), "{asset}");
            }
        }
        for url in [
            "http://cdn.discordapp.com/media/v1/collectibles-shop/a",
            "https://media.discordapp.net/media/v1/collectibles-shop/a",
            "https://cdn.discordapp.com/attachments/1/2/a.png",
            "https://cdn.discordapp.com/media/v1/collectibles-shop/../../attachments/a",
            "https://evil.example/media/v1/collectibles-shop/a",
            "https://cdn.discordapp.com.evil.example/media/v1/collectibles-shop/a",
        ] {
            let spec = ImageSpec::Collectible { url: url.into() };
            assert!(image_url(&spec, 0).is_none(), "{url}");
        }
    }

    #[test]
    fn sticker_urls_follow_format_and_size_buckets() {
        let url = |format, size| {
            image_url(&ImageSpec::Sticker { id: 123, format }, size).map(String::from)
        };
        assert_eq!(
            url(1, 160).as_deref(),
            Some("https://media.discordapp.net/stickers/123.png?size=256")
        );
        assert_eq!(
            url(2, 160).as_deref(),
            Some("https://media.discordapp.net/stickers/123.png?size=256")
        );
        assert_eq!(
            url(4, 160).as_deref(),
            Some("https://media.discordapp.net/stickers/123.gif?size=256")
        );
        assert!(url(3, 160).is_none());
        assert!(url(0, 160).is_none());
        assert!(url(5, 160).is_none());
    }

    #[test]
    fn video_urls_preserve_parameters_and_reject_untrusted_sources() {
        for host in [
            "cdn.discordapp.com",
            "media.discordapp.net",
            "images-ext-1.discordapp.net",
            "images-ext-123.discordapp.net",
        ] {
            for dimensions in [(0, 0), (400, 200)] {
                let raw = format!(
                    "https://{host}/external/x.mp4?width=1&height=2&size=3&format=mp4&token=a%2Fb#fragment"
                );
                let spec = ImageSpec::Video {
                    url: raw.clone(),
                    width: dimensions.0,
                    height: dimensions.1,
                };
                for size in [0, 64, 4096] {
                    assert_eq!(
                        image_url(&spec, size).unwrap().as_str(),
                        raw.split('#').next().unwrap()
                    );
                }
                assert!(file_name(&spec, 64).ends_with("-64.img"));
            }
        }
        for url in [
            "https://media.tenor.com.evil.test/a.mp4",
            "https://i.giphy.com.evil.test/a.mp4",
            "http://cdn.discordapp.com/a.mp4",
            "https://user:pass@cdn.discordapp.com/a.mp4",
            "https://media.discordapp.net:444/a.mp4",
            "https://images-ext-.discordapp.net/a.mp4",
            "https://images-ext-x.discordapp.net/a.mp4",
            "https://images-ext-1.discordapp.net.evil.test/a.mp4",
            "not a URL",
        ] {
            assert!(
                image_url(
                    &ImageSpec::Video {
                        url: url.into(),
                        width: 1,
                        height: 1,
                    },
                    64
                )
                .is_none()
            );
        }
    }

    #[test]
    fn video_posters_use_the_proxy_webp_format() {
        let raw = "https://media.discordapp.net/attachments/1/2/clip.mp4?ex=abc&width=9&height=9&size=9&format=mp4#fragment";
        let spec = ImageSpec::VideoPoster {
            id: 1,
            url: raw.into(),
            width: 640,
            height: 360,
        };
        assert_eq!(
            image_url(&spec, 128).unwrap().as_str(),
            "https://media.discordapp.net/attachments/1/2/clip.mp4?ex=abc&width=128&height=72&format=webp"
        );
        assert!(
            image_url(
                &ImageSpec::VideoPoster {
                    id: 1,
                    url: raw.into(),
                    width: 0,
                    height: 360,
                },
                128,
            )
            .is_none()
        );
        assert_eq!(
            trusted_video_url(raw).as_deref(),
            Some(
                "https://media.discordapp.net/attachments/1/2/clip.mp4?ex=abc&width=9&height=9&size=9&format=mp4"
            )
        );
        let redirect = "https://images-ext-1.discordapp.net/external/abc/%3Furl%3Dhttps%253A%252F%252Fvideo.twimg.com%252Fa.mp4/https/api.fxtwitter.com/2/go";
        assert_eq!(trusted_video_url(redirect).as_deref(), Some(redirect));
        for url in [
            "https://media.tenor.com.evil.test/a.mp4",
            "https://cdn.discordapp.com/a.m3u8",
            "https://media.discordapp.net/external/abc/https/example.com/go",
            "http://cdn.discordapp.com/a.mp4",
            "https://cdn.discordapp.com:444/a.mp4",
        ] {
            assert!(trusted_video_url(url).is_none(), "{url}");
        }
    }

    #[test]
    fn video_and_image_download_formats_are_separate() {
        let video = ImageSpec::Video {
            url: String::new(),
            width: 1,
            height: 1,
        };
        let image = ImageSpec::DefaultAvatar { index: 0 };
        for bytes in [b"\0\0\0\x18ftypisom".as_slice(), b"\0\0\0\x20ftypmp42"] {
            assert!(download_bytes(&video, bytes));
            assert!(!download_bytes(&image, bytes));
        }
        for bytes in [
            b"".as_slice(),
            b"ftyp",
            b"\0\0\0\x18fty",
            b"\0\0\0\x18moov",
            b"<html>",
        ] {
            assert!(!download_bytes(&video, bytes));
            assert!(!download_bytes(&image, bytes));
        }
        for bytes in [
            b"\x89PNG\r\n\x1a\n".as_slice(),
            &[0xff, 0xd8, 0xff],
            b"GIF87a",
            b"GIF89a",
            b"RIFF\0\0\0\0WEBP",
        ] {
            assert!(download_bytes(&image, bytes));
            assert!(!download_bytes(&video, bytes));
        }
    }

    #[test]
    fn external_proxy_hosts_and_unknown_media_dimensions() {
        for host in [
            "images-ext-1.discordapp.net",
            "images-ext-2.discordapp.net",
            "images-ext-123.discordapp.net",
        ] {
            let url = format!(
                "https://{host}/external/image.png?token=keep&width=1&height=2&size=3#fragment"
            );
            for (width, height) in [(0, 0), (0, 200), (100, 0)] {
                let spec = ImageSpec::Media {
                    url: url.clone(),
                    width,
                    height,
                    animated: false,
                };
                assert_eq!(
                    image_url(&spec, 64).unwrap().as_str(),
                    format!("https://{host}/external/image.png?token=keep")
                );
                assert!(
                    image_url(
                        &ImageSpec::Attachment {
                            id: 1,
                            url: url.clone(),
                            width,
                            height
                        },
                        64
                    )
                    .is_none()
                );
            }
            let spec = ImageSpec::Media {
                url: url.clone(),
                width: 400,
                height: 200,
                animated: false,
            };
            assert_eq!(
                image_url(&spec, 100).unwrap().as_str(),
                format!("https://{host}/external/image.png?token=keep&width=128&height=64")
            );
            let animated = ImageSpec::Media {
                url,
                width: 400,
                height: 200,
                animated: true,
            };
            assert_eq!(
                image_url(&animated, 100).unwrap().as_str(),
                format!(
                    "https://{host}/external/image.png?token=keep&width=128&height=64&animated=true"
                )
            );
        }
        for host in [
            "images-ext-.discordapp.net",
            "images-ext-x.discordapp.net",
            "images-ext-1.discordapp.net.evil.test",
            "images-ext-1.evil.test",
            "images-ext-1x.discordapp.net",
        ] {
            assert!(!allowed_url(
                &reqwest::Url::parse(&format!("https://{host}/a")).unwrap()
            ));
        }
        assert_eq!(
            image_url(
                &ImageSpec::Media {
                    url: "https://images-ext-1.discordapp.net/a".into(),
                    width: 0,
                    height: 0,
                    animated: false,
                },
                64
            )
            .unwrap()
            .as_str(),
            "https://images-ext-1.discordapp.net/a"
        );
    }

    #[test]
    fn app_images_request_a_sized_copy() {
        let url = |uploaded| {
            image_url(
                &ImageSpec::AppImage {
                    application_id: 7,
                    hash: "abc123".into(),
                    uploaded,
                    animated: false,
                },
                300,
            )
            .unwrap()
            .to_string()
        };
        assert_eq!(
            url(false),
            "https://cdn.discordapp.com/app-icons/7/abc123.png?size=512"
        );
        assert_eq!(
            url(true),
            "https://cdn.discordapp.com/app-assets/7/abc123.webp?size=512"
        );
        let still = ImageSpec::AppImage {
            application_id: 7,
            hash: "abc123".into(),
            uploaded: true,
            animated: true,
        };
        assert_eq!(
            image_url(&still.animated().unwrap(), 300)
                .unwrap()
                .to_string(),
            "https://cdn.discordapp.com/app-assets/7/abc123.webp?size=512&animated=true"
        );
        let cover = ImageSpec::AppCover {
            application_id: 7,
            hash: "abc123".into(),
        };
        assert_eq!(
            image_url(&cover, 300).unwrap().to_string(),
            "https://cdn.discordapp.com/app-icons/7/abc123.png?size=512&keep_aspect_ratio=true"
        );
    }

    #[test]
    fn spotify_album_art_loads_as_an_image_only() {
        let url = "https://i.scdn.co/image/synthetic-album-art";
        assert_eq!(
            image_url(
                &ImageSpec::Media {
                    url: url.into(),
                    width: 300,
                    height: 300,
                    animated: false,
                },
                128
            )
            .unwrap()
            .as_str(),
            "https://i.scdn.co/image/synthetic-album-art?width=128&height=128"
        );
        assert!(trusted_video_url("https://i.scdn.co/image/synthetic.mp4").is_none());
        assert!(!allowed_url(&reqwest::Url::parse(url).unwrap()));
        for url in [
            "http://i.scdn.co/image/synthetic-album-art",
            "https://i.scdn.co.evil.test/image/synthetic-album-art",
            "https://i.scdn.co:8443/image/synthetic-album-art",
            "https://user:pass@i.scdn.co/image/synthetic-album-art",
        ] {
            assert!(
                image_url(
                    &ImageSpec::Media {
                        url: url.into(),
                        width: 0,
                        height: 0,
                        animated: false,
                    },
                    64
                )
                .is_none(),
                "{url}"
            );
        }
    }

    #[test]
    fn gif_picker_media_loads_from_provider_hosts_only() {
        for host in [
            "static.klipy.com",
            "media.tenor.com",
            "media.giphy.com",
            "i.giphy.com",
            "media0.giphy.com",
            "media1.giphy.com",
            "media2.giphy.com",
            "media3.giphy.com",
            "media4.giphy.com",
        ] {
            let url = format!("https://{host}/gif.webp");
            assert_eq!(
                image_url(
                    &ImageSpec::Media {
                        url: url.clone(),
                        width: 200,
                        height: 200,
                        animated: true,
                    },
                    128
                )
                .unwrap()
                .as_str(),
                format!("https://{host}/gif.webp?width=128&height=128&animated=true")
            );
            assert!(!allowed_url(&reqwest::Url::parse(&url).unwrap()), "{url}");
        }
        for url in [
            "https://media.tenor.com/clip.mp4",
            "https://media4.giphy.com/clip.mp4",
        ] {
            assert!(
                image_url(
                    &ImageSpec::Video {
                        url: url.into(),
                        width: 200,
                        height: 200,
                    },
                    64
                )
                .is_some(),
                "{url}"
            );
        }
        for url in [
            "http://static.klipy.com/gif.webp",
            "https://static.klipy.com.evil.test/gif.webp",
            "https://media.tenor.com.evil.test/gif.webp",
            "https://media1.giphy.com.evil.test/gif.webp",
            "https://static.klipy.com:8443/gif.webp",
            "https://user:pass@static.klipy.com/gif.webp",
            "https://media.gfycat.com/gif.webp",
        ] {
            assert!(
                image_url(
                    &ImageSpec::Media {
                        url: url.into(),
                        width: 0,
                        height: 0,
                        animated: false,
                    },
                    64
                )
                .is_none(),
                "{url}"
            );
        }
    }

    #[test]
    fn gif_provider_videos_are_trusted_only_for_mp4_and_webm() {
        for (url, trusted) in [
            (
                "https://media.tenor.com/abc/clip.mp4",
                "https://media.tenor.com/abc/clip.mp4",
            ),
            (
                "https://media.tenor.com/abc/clip.webm",
                "https://media.tenor.com/abc/clip.webm",
            ),
            (
                "https://media.tenor.com/abc/clip.mp4?ex=1&size=2#fragment",
                "https://media.tenor.com/abc/clip.mp4?ex=1&size=2",
            ),
            (
                "https://static.klipy.com/ii/aa/bb/mp4/1.mp4",
                "https://static.klipy.com/ii/aa/bb/mp4/1.mp4",
            ),
            (
                "https://media1.giphy.com/media/x/clip.MP4",
                "https://media1.giphy.com/media/x/clip.MP4",
            ),
            (
                "https://media.giphy.com/media/x/clip.webm",
                "https://media.giphy.com/media/x/clip.webm",
            ),
            (
                "https://i.giphy.com/clip.mp4",
                "https://i.giphy.com/clip.mp4",
            ),
        ] {
            assert_eq!(trusted_video_url(url).as_deref(), Some(trusted), "{url}");
        }
        for url in [
            "http://media.tenor.com/clip.mp4",
            "https://media.tenor.com.evil.test/clip.mp4",
            "https://media.tenor.com:8443/clip.mp4",
            "https://user:pass@media.tenor.com/clip.mp4",
            "https://static.klipy.com/clip.mov",
            "https://static.klipy.com/clip.m4v",
            "https://static.klipy.com/clip.gif",
            "https://static.klipy.com/clip",
            "https://static.klipy.com/clip?file=1.mp4",
            "https://media.gfycat.com/clip.mp4",
        ] {
            assert!(trusted_video_url(url).is_none(), "{url}");
        }
    }

    #[test]
    fn cdn_urls_and_size_buckets() {
        assert_eq!(
            image_url(
                &ImageSpec::GuildTagBadge {
                    guild_id: 9,
                    hash: "badge".into()
                },
                20
            )
            .unwrap()
            .as_str(),
            "https://cdn.discordapp.com/guild-tag-badges/9/badge.png?size=32"
        );
        let avatar = ImageSpec::GuildAvatar {
            guild_id: 1,
            user_id: 2,
            hash: "a_hash".into(),
        };
        assert_eq!(
            image_url(&avatar, 32).unwrap().as_str(),
            "https://cdn.discordapp.com/guilds/1/users/2/avatars/a_hash.png?size=32"
        );
        for animated in [true, false] {
            assert_eq!(
                image_url(&ImageSpec::Emoji { id: 123, animated }, 100)
                    .unwrap()
                    .as_str(),
                format!("https://cdn.discordapp.com/emojis/123.webp?size=128&animated={animated}")
            );
        }
        assert_eq!(
            [0, 16, 17, 32, 100, 4097, u32::MAX].map(rounded_size),
            [16, 16, 32, 32, 128, 4096, 4096]
        );
        let icon = ImageSpec::GuildIcon {
            guild_id: 1,
            hash: "a_hash".into(),
        };
        assert_eq!(
            image_url(&icon, 100).unwrap().as_str(),
            "https://cdn.discordapp.com/icons/1/a_hash.png?size=128"
        );
        let avatar = ImageSpec::Avatar {
            user_id: 2,
            hash: "hash".into(),
        };
        assert_eq!(
            image_url(&avatar, 32).unwrap().as_str(),
            "https://cdn.discordapp.com/avatars/2/hash.png?size=32"
        );
        assert_eq!(
            image_url(&ImageSpec::DefaultAvatar { index: 5 }, 64)
                .unwrap()
                .as_str(),
            "https://cdn.discordapp.com/embed/avatars/5.png"
        );
        let attachment = ImageSpec::Attachment {
            id: 3,
            url: "https://media.discordapp.net/attachments/1/2/a.gif?ex=signature&width=1&height=1"
                .into(),
            width: 400,
            height: 200,
        };
        assert_eq!(
            image_url(&attachment, 100).unwrap().as_str(),
            "https://media.discordapp.net/attachments/1/2/a.gif?ex=signature&width=128&height=64"
        );
        assert!(
            image_url(&attachment, 4096)
                .unwrap()
                .as_str()
                .ends_with("width=400&height=200")
        );
    }

    #[test]
    fn hostile_urls_and_cache_names() {
        for url in [
            "https://evil.example/x.png",
            "http://cdn.discordapp.com/a.png",
            "https://cdn.discordapp.com.evil.example/a",
            "https://user@cdn.discordapp.com/a",
            "https://media.discordapp.net:444/a",
        ] {
            let spec = ImageSpec::Attachment {
                id: 1,
                url: url.into(),
                width: 100,
                height: 100,
            };
            assert!(image_url(&spec, 64).is_none());
            let name = file_name(&spec, 64);
            assert!(!name.contains('?') && !name.contains('/') && !name.contains(".."));
            assert_eq!(name, file_name(&spec, 63));
        }
        assert!(
            image_url(
                &ImageSpec::Avatar {
                    user_id: 1,
                    hash: "../escape".into()
                },
                32
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn atomic_files_and_pruning() {
        let directory = temp_directory("atomic");
        tokio::fs::create_dir(&directory).await.unwrap();
        let path = directory.join("test.img");
        atomic_write(&path, b"image fixture").await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"image fixture");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        set_age(&path, Duration::from_secs(2 * 60));
        prune(&directory, 0).unwrap();
        assert!(!path.exists());
        tokio::fs::remove_dir(directory).await.unwrap();
    }
}
