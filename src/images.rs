use gpui::{App, Global};
use gpui_tokio::Tokio;
use std::{
    cell::RefCell,
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{OnceCell, Semaphore},
};

const MAX_DOWNLOAD: u64 = 25 * 1024 * 1024;
const MAX_CACHE: u64 = 512 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum ImageSpec {
    GuildIcon {
        guild_id: u64,
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
    GuildTagBadge {
        guild_id: u64,
        hash: String,
    },
}

fn rounded_size(size: u32) -> u32 {
    size.clamp(16, 4096).next_power_of_two()
}

fn allowed_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some("cdn.discordapp.com" | "media.discordapp.net")
        )
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

fn image_url(spec: &ImageSpec, size: u32) -> Option<reqwest::Url> {
    let size = rounded_size(size);
    let raw = match spec {
        ImageSpec::GuildAvatar {
            guild_id,
            user_id,
            hash,
        } => {
            if hash.is_empty()
                || hash.len() > 128
                || !hash.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            {
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
        ImageSpec::GuildIcon { guild_id, hash }
        | ImageSpec::GuildTagBadge { guild_id, hash }
        | ImageSpec::Avatar {
            user_id: guild_id,
            hash,
        } => {
            if hash.is_empty()
                || hash.len() > 128
                || !hash.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            {
                return None;
            }
            let route = match spec {
                ImageSpec::GuildIcon { .. } => "icons",
                ImageSpec::GuildTagBadge { .. } => "guild-tag-badges",
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
        ImageSpec::Attachment { url, .. } => url.clone(),
    };
    let mut url = reqwest::Url::parse(&raw).ok()?;
    if !allowed_url(&url) {
        return None;
    }
    url.set_fragment(None);
    if let ImageSpec::Attachment { width, height, .. } = spec {
        let longest = (*width).max(*height);
        if *width == 0 || *height == 0 {
            return None;
        }
        let target = size.min(longest);
        let width = (u64::from(*width) * u64::from(target) / u64::from(longest)).max(1);
        let height = (u64::from(*height) * u64::from(target) / u64::from(longest)).max(1);
        let params: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| !matches!(key.as_ref(), "width" | "height" | "size"))
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        // The media proxy keeps the original format (and GIF animation); WebP conversion isn't
        // documented as guaranteed. https://docs.discord.food/reference#media-proxy-parameters
        url.set_query(None);
        url.query_pairs_mut()
            .extend_pairs(params)
            .append_pair("width", &width.to_string())
            .append_pair("height", &height.to_string());
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

struct Service {
    client: reqwest::Client,
    directory: PathBuf,
    permits: Semaphore,
    initialized: OnceCell<bool>,
}

pub fn init(cx: &mut App) {
    if cx.try_global::<ImageCache>().is_some() {
        return;
    }
    let directory = dirs::cache_dir().map(|dir| dir.join("zcord").join("images"));
    let client = reqwest::Client::builder()
        .user_agent(crate::discord::USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(45))
        .build()
        .ok();
    let service = directory.zip(client).map(|(directory, client)| {
        Arc::new(Service {
            client,
            directory,
            permits: Semaphore::new(6),
            initialized: OnceCell::new(),
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

impl Service {
    async fn initialize(&self) -> bool {
        *self
            .initialized
            .get_or_init(|| async {
                let directory = self.directory.clone();
                let success = tokio::task::spawn_blocking(move || {
                    std::fs::create_dir_all(&directory)?;
                    prune(&directory, MAX_CACHE)
                })
                .await
                .is_ok_and(|result| result.is_ok());
                if !success {
                    log::warn!("Image cache directory could not be prepared");
                }
                success
            })
            .await
    }

    async fn load(&self, spec: &ImageSpec, size: u32) -> Option<PathBuf> {
        let url = image_url(spec, size)?;
        if !self.initialize().await {
            return None;
        }
        let _permit = self.permits.acquire().await.ok()?;
        let path = self.directory.join(file_name(spec, size));
        if tokio::fs::metadata(&path)
            .await
            .is_ok_and(|m| m.is_file() && m.len() > 0 && m.len() <= MAX_DOWNLOAD)
        {
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
        if !image_bytes(&bytes) {
            return None;
        }
        atomic_write(&path, &bytes).await.ok()?;
        Some(path)
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
    let mut files = Vec::new();
    let mut total = 0u64;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let metadata = entry.metadata()?;
        total = total.saturating_add(metadata.len());
        files.push((
            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            metadata.len(),
            entry.path(),
        ));
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, length, path) in files {
        if total <= limit {
            break;
        }
        std::fs::remove_file(path)?;
        total = total.saturating_sub(length);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let directory =
            std::env::temp_dir().join(format!("zcord-image-test-{:016x}", rand::random::<u64>()));
        tokio::fs::create_dir(&directory).await.unwrap();
        let path = directory.join("test.img");
        atomic_write(&path, b"image fixture").await.unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"image fixture");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        prune(&directory, 0).unwrap();
        assert!(!path.exists());
        tokio::fs::remove_dir(directory).await.unwrap();
    }
}
