use super::*;

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

fn twitch_preview_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("static-cdn.jtvnw.net")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().starts_with("/previews-ttv/live_user_")
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
pub(super) fn gif_provider_media_url(url: &reqwest::Url) -> bool {
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

pub(super) fn avif_proxy_image(spec: &ImageSpec, url: &reqwest::Url) -> bool {
    matches!(spec, ImageSpec::Attachment { .. } | ImageSpec::Media { .. })
        && allowed_url(url)
        && url
            .path()
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("avif"))
}

/// The 96px WebP of an avatar that Discord's color picker samples its suggestions from, which
/// `image_url`'s power-of-two sizes can't request. Animated avatars sample their first frame.
pub(crate) fn avatar_sample_url(spec: &ImageSpec) -> Option<reqwest::Url> {
    if !matches!(
        spec,
        ImageSpec::Avatar { .. } | ImageSpec::GuildAvatar { .. } | ImageSpec::ArchivedAvatar { .. }
    ) {
        return None;
    }
    let mut url = image_url(spec, 96)?;
    let path = url.path().strip_suffix(".png")?.to_owned();
    url.set_path(&format!("{path}.webp"));
    url.query_pairs_mut().clear().append_pair("size", "96");
    if spec.animated().is_some() {
        url.query_pairs_mut().append_pair("animated", "true");
    }
    Some(url)
}

pub(super) fn image_url(spec: &ImageSpec, size: u32) -> Option<reqwest::Url> {
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
        | ImageSpec::RoleIcon {
            role_id: guild_id,
            hash,
        }
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
                ImageSpec::RoleIcon { .. } => "role-icons",
                ImageSpec::ChannelIcon { .. } => "channel-icons",
                _ => "avatars",
            };
            format!("https://cdn.discordapp.com/{route}/{guild_id}/{hash}.png?size={size}")
        }
        ImageSpec::ArchivedAvatar {
            user_id,
            avatar_id,
            hash,
        } => {
            if !valid_hash(hash) {
                return None;
            }
            format!(
                "https://cdn.discordapp.com/avatars/{user_id}/archived/{avatar_id}/{hash}.png?size={size}"
            )
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
        ImageSpec::LocalGif { .. } | ImageSpec::LocalVideo { .. } => return None,
    };
    let mut url = reqwest::Url::parse(&raw).ok()?;
    if !allowed_url(&url)
        && !spotify_image_url(&url)
        && !twitch_preview_url(&url)
        && !gif_provider_media_url(&url)
    {
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
    let avif = avif_proxy_image(spec, &url);
    if avif && url.host_str() == Some("cdn.discordapp.com") {
        url.set_host(Some("media.discordapp.net")).ok()?;
    }
    if let ImageSpec::Attachment { width, height, .. }
    | ImageSpec::Media { width, height, .. }
    | ImageSpec::VideoPoster { width, height, .. } = spec
    {
        let poster = matches!(spec, ImageSpec::VideoPoster { .. });
        let longest = (*width).max(*height);
        let dimensions = if *width == 0 || *height == 0 {
            if matches!(
                spec,
                ImageSpec::Attachment { .. } | ImageSpec::VideoPoster { .. }
            ) {
                return None;
            }
            None
        } else {
            let target = size.min(longest);
            let width = (u64::from(*width) * u64::from(target) / u64::from(longest)).max(1);
            let height = (u64::from(*height) * u64::from(target) / u64::from(longest)).max(1);
            Some((width, height))
        };
        let params: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| {
                !matches!(key.as_ref(), "width" | "height" | "size")
                    && !((poster || avif) && key == "format")
                    && !(avif && key == "animated")
            })
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        url.set_query(None);
        if !params.is_empty() {
            url.query_pairs_mut().extend_pairs(params);
        }
        if let Some((width, height)) = dimensions {
            url.query_pairs_mut()
                .append_pair("width", &width.to_string())
                .append_pair("height", &height.to_string());
        }
        if avif {
            // The media proxy keeps the original format (and GIF animation) unless `format` is set.
            // AVIF isn't decoded locally, so it is requested as WebP with `animated` explicit.
            // https://docs.discord.food/reference#media-proxy-parameters
            let animated = !matches!(
                spec,
                ImageSpec::Media {
                    animated: false,
                    ..
                }
            );
            url.query_pairs_mut()
                .append_pair("format", "webp")
                .append_pair("animated", if animated { "true" } else { "false" });
        } else if matches!(spec, ImageSpec::Media { animated: true, .. }) {
            url.query_pairs_mut().append_pair("animated", "true");
        }
        if poster {
            url.query_pairs_mut().append_pair("format", "webp");
        }
    }
    Some(url)
}
