use super::urls::{avif_proxy_image, rounded_size};
use super::*;

pub(super) type StillConversion = fn(&[u8], u32) -> Option<Vec<u8>>;

pub(super) fn first_frame_png(bytes: &[u8], size: u32) -> Option<Vec<u8>> {
    let format = image::guess_format(bytes).ok()?;
    let first = match format {
        ImageFormat::Gif => {
            let mut frames = image::codecs::gif::GifDecoder::new(Cursor::new(bytes))
                .ok()?
                .into_frames();
            let first = frames.next()?.ok()?;
            let _second = frames.next()?;
            first
        }
        ImageFormat::WebP => {
            let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
            if !decoder.has_animation() {
                return None;
            }
            decoder.into_frames().next()?.ok()?
        }
        _ => return None,
    };
    let image = first.into_buffer();
    let longest = image.width().max(image.height());
    encode_png(&shrink(image, longest, size))
}

pub(super) fn downscaled_still(bytes: &[u8], size: u32) -> Option<Vec<u8>> {
    let format = image::guess_format(bytes).ok()?;
    let animated = match format {
        ImageFormat::Png => image::codecs::png::PngDecoder::new(Cursor::new(bytes))
            .ok()?
            .is_apng()
            .ok()?,
        ImageFormat::WebP => image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))
            .ok()?
            .has_animation(),
        ImageFormat::Jpeg => false,
        _ => return None,
    };
    if animated {
        return None;
    }
    let image = image::load_from_memory_with_format(bytes, format)
        .ok()?
        .into_rgba8();
    let shortest = image.width().min(image.height());
    if shortest <= size.max(1) {
        return None;
    }
    encode_png(&shrink(image, shortest, size))
}

fn shrink(image: image::RgbaImage, reference: u32, target: u32) -> image::RgbaImage {
    let target = target.max(1);
    if reference <= target {
        return image;
    }
    let scaled = |side: u32| {
        ((u64::from(side) * u64::from(target) + u64::from(reference) / 2) / u64::from(reference))
            .max(1) as u32
    };
    let (width, height) = (scaled(image.width()), scaled(image.height()));
    image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle)
}

fn encode_png(image: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some(png)
}

pub(super) fn still_spec(spec: &ImageSpec) -> bool {
    match spec {
        ImageSpec::Animated { .. }
        | ImageSpec::Video { .. }
        | ImageSpec::Attachment { .. }
        | ImageSpec::Collectible { .. } => false,
        ImageSpec::Emoji { animated, .. }
        | ImageSpec::Media { animated, .. }
        | ImageSpec::AvatarDecoration { animated, .. }
        | ImageSpec::Nameplate { animated, .. }
        | ImageSpec::AppImage { animated, .. } => !*animated,
        ImageSpec::Sticker { format, .. } => !matches!(*format, 2 | 4),
        _ => true,
    }
}

pub(super) fn file_name(spec: &ImageSpec, size: u32) -> String {
    let mut hasher = DefaultHasher::new();
    spec.hash(&mut hasher);
    if let ImageSpec::Attachment { url, .. } | ImageSpec::Media { url, .. } = spec
        && let Ok(url) = reqwest::Url::parse(url)
        && avif_proxy_image(spec, &url)
    {
        "avif-proxy-webp-v1".hash(&mut hasher);
    }
    if matches!(spec, ImageSpec::Collectible { .. }) {
        "collectible-downscaled-v2".hash(&mut hasher);
    }
    let suffix = if still_spec(spec) { "-still" } else { "" };
    format!(
        "{:016x}{suffix}-{}.img",
        hasher.finish(),
        rounded_size(size)
    )
}
