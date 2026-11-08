use super::*;

#[cfg(test)]
pub(super) fn prepare_style_image(
    path: PathBuf,
    target: ImageTarget,
    nitro: bool,
) -> Result<ImageChange, String> {
    let bytes = read_style_file(&path)?;
    let data_uri = encode_style_image(&bytes, target, nitro)?;
    Ok(ImageChange::Upload {
        preview: path.clone(),
        path,
        data_uri: data_uri.into(),
    })
}

/// Reads a picked avatar or banner, refusing other formats and anything over the upload limit.
pub(crate) fn read_style_file(path: &Path) -> Result<Vec<u8>, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") {
        return Err("Choose a PNG, JPEG, GIF, or WebP image.".into());
    }
    let file = std::fs::File::open(path).map_err(|_| "Could not read the image.".to_owned())?;
    let metadata = file
        .metadata()
        .map_err(|_| "Could not read the image.".to_owned())?;
    if !metadata.is_file() || metadata.len() >= STYLE_IMAGE_LIMIT {
        return Err("Choose an image smaller than 10 MB.".into());
    }
    let mut bytes = Vec::new();
    file.take(STYLE_IMAGE_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the image.".to_owned())?;
    if bytes.len() as u64 >= STYLE_IMAGE_LIMIT {
        return Err("Choose an image smaller than 10 MB.".into());
    }
    Ok(bytes)
}

/// Where the image editor's frame sat: the image turned `quarter_turns` clockwise, then
/// `rect` (`[x, y, width, height]`) cut from the turned image's pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StyleCrop {
    pub quarter_turns: u8,
    pub rect: [u32; 4],
}

impl StyleCrop {
    /// The largest centered cut of `target`'s shape, which uploads take without the editor.
    pub(crate) fn centered(width: u32, height: u32, target: ImageTarget) -> Self {
        let (ratio_width, ratio_height) = target.ratio();
        let (crop_width, crop_height) =
            if u64::from(width) * ratio_height > u64::from(height) * ratio_width {
                (
                    (u64::from(height) * ratio_width / ratio_height).max(1) as u32,
                    height,
                )
            } else {
                (
                    width,
                    (u64::from(width) * ratio_height / ratio_width).max(1) as u32,
                )
            };
        Self {
            quarter_turns: 0,
            rect: [
                (width - crop_width) / 2,
                (height - crop_height) / 2,
                crop_width,
                crop_height,
            ],
        }
    }

    fn is_identity(&self, width: u32, height: u32) -> bool {
        self.quarter_turns.is_multiple_of(4) && self.rect == [0, 0, width, height]
    }

    /// Turns and cuts one frame, then shrinks it to fit `target`'s upload size.
    fn apply(
        &self,
        image: image::RgbaImage,
        target: ImageTarget,
        filter: image::imageops::FilterType,
    ) -> image::RgbaImage {
        let turned = match self.quarter_turns % 4 {
            1 => image::imageops::rotate90(&image),
            2 => image::imageops::rotate180(&image),
            3 => image::imageops::rotate270(&image),
            _ => image,
        };
        let [x, y, width, height] = self.rect;
        let x = x.min(turned.width() - 1);
        let y = y.min(turned.height() - 1);
        let width = width.clamp(1, turned.width() - x);
        let height = height.clamp(1, turned.height() - y);
        let cropped = image::imageops::crop_imm(&turned, x, y, width, height).to_image();
        let (max_width, max_height) = target.max_size();
        if width <= max_width && height <= max_height {
            return cropped;
        }
        let scale = (max_width as f32 / width as f32).min(max_height as f32 / height as f32);
        let scaled = |side: u32| ((side as f32 * scale).round() as u32).max(1);
        image::imageops::resize(&cropped, scaled(width), scaled(height), filter)
    }
}

impl ImageTarget {
    /// Discord's upload shape: square avatars, 17:6 banners.
    pub(crate) fn ratio(self) -> (u64, u64) {
        match self {
            ImageTarget::Avatar => (1, 1),
            ImageTarget::Banner => (17, 6),
        }
    }

    fn max_size(self) -> (u32, u32) {
        match self {
            ImageTarget::Avatar => (1024, 1024),
            ImageTarget::Banner => (2400, 848),
        }
    }
}

pub(crate) fn image_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits
}

pub(super) struct StyleGifWriter {
    pub(super) bytes: Vec<u8>,
    pub(super) limit: usize,
    pub(super) overflowed: bool,
}

impl Write for StyleGifWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) >= self.limit {
            self.overflowed = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "GIF exceeds upload limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn gif_repeat(count: image::metadata::LoopCount) -> Repeat {
    match count {
        image::metadata::LoopCount::Infinite => Repeat::Infinite,
        image::metadata::LoopCount::Finite(count) => {
            Repeat::Finite(u16::try_from(count.get()).unwrap_or(u16::MAX))
        }
    }
}

/// Re-encodes animation frames as a GIF, cutting each one to `crop` when given. Returns the
/// GIF and how many frames it holds.
fn encode_style_gif<'a>(
    frames: image::Frames<'a>,
    (width, height): (u32, u32),
    repeat: Repeat,
    max_bytes: usize,
    crop: Option<(StyleCrop, ImageTarget)>,
) -> Result<(Vec<u8>, usize), String> {
    let mut output = StyleGifWriter {
        bytes: Vec::new(),
        limit: max_bytes,
        overflowed: false,
    };
    let encoded = (|| -> Result<usize, String> {
        let mut encoder = GifEncoder::new_with_speed(&mut output, 10);
        encoder
            .set_repeat(repeat)
            .map_err(|_| "Could not convert this GIF.".to_owned())?;
        let mut count = 0;
        let mut pixels = 0u64;
        for frame in frames {
            if count >= 1024 {
                return Err("This GIF has too many frames.".into());
            }
            pixels = pixels.saturating_add(u64::from(width) * u64::from(height));
            if pixels > 128 * 1024 * 1024 {
                return Err("This GIF is too large to process.".into());
            }
            let frame = frame.map_err(|_| "Could not decode this image.".to_owned())?;
            let frame = match crop {
                Some((crop, target)) => {
                    let delay = frame.delay();
                    let buffer = crop.apply(
                        frame.into_buffer(),
                        target,
                        image::imageops::FilterType::Triangle,
                    );
                    image::Frame::from_parts(buffer, 0, 0, delay)
                }
                None => frame,
            };
            encoder
                .encode_frame(frame)
                .map_err(|_| "Could not convert this GIF.".to_owned())?;
            count += 1;
        }
        Ok(count)
    })();
    if output.overflowed {
        return Err("This GIF exceeds the 10 MB upload limit after conversion.".into());
    }
    Ok((output.bytes, encoded?))
}

pub(super) fn webp_style_gif(
    bytes: &[u8],
    max_bytes: usize,
    crop: Option<(StyleCrop, ImageTarget)>,
) -> Result<Option<Vec<u8>>, String> {
    let mut decoder = WebPDecoder::new(Cursor::new(bytes))
        .map_err(|_| "Could not decode this image.".to_owned())?;
    if !decoder.has_animation() {
        return Ok(None);
    }
    let mut limits = image_limits();
    decoder
        .set_limits(limits.clone())
        .map_err(|_| "This GIF is too large to process.".to_owned())?;
    let dimensions = decoder.dimensions();
    // WebP's animation iterator does not enforce max_alloc itself.
    limits
        .reserve_buffer(dimensions.0, dimensions.1, image::ColorType::Rgba8)
        .map_err(|_| "This GIF is too large to process.".to_owned())?;
    let repeat = gif_repeat(decoder.loop_count());
    let (gif, frames) =
        encode_style_gif(decoder.into_frames(), dimensions, repeat, max_bytes, crop)?;
    Ok((frames > 1).then_some(gif))
}

/// A GIF's frames cut to `crop`; uncut GIFs upload as they are.
fn cropped_style_gif(
    bytes: &[u8],
    crop: StyleCrop,
    target: ImageTarget,
) -> Result<Vec<u8>, String> {
    let invalid = || "Could not decode this image.".to_owned();
    let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    let mut limits = image_limits();
    decoder.set_limits(limits.clone()).map_err(|_| invalid())?;
    let dimensions = decoder.dimensions();
    limits
        .reserve_buffer(dimensions.0, dimensions.1, image::ColorType::Rgba8)
        .map_err(|_| "This GIF is too large to process.".to_owned())?;
    let repeat = gif_repeat(decoder.loop_count());
    let (gif, _) = encode_style_gif(
        decoder.into_frames(),
        dimensions,
        repeat,
        STYLE_IMAGE_LIMIT as usize,
        Some((crop, target)),
    )?;
    Ok(gif)
}

/// The first frame of a picked image, which stills upload and the editor shows.
pub(crate) fn decode_style_still(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let invalid = || "Could not decode this image.".to_owned();
    let format = image::guess_format(bytes).map_err(|_| invalid())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP
    ) {
        return Err("Choose a PNG, JPEG, GIF, or WebP image.".into());
    }
    let image = if format == ImageFormat::Gif {
        let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|_| invalid())?;
        decoder.set_limits(image_limits()).map_err(|_| invalid())?;
        let first = decoder
            .into_frames()
            .next()
            .ok_or_else(invalid)?
            .map_err(|_| invalid())?;
        image::DynamicImage::ImageRgba8(first.into_buffer())
    } else {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(image_limits());
        reader.decode().map_err(|_| invalid())?
    };
    if image.width() == 0 || image.height() == 0 {
        return Err(invalid());
    }
    Ok(image)
}

fn gif_is_animated(bytes: &[u8]) -> Result<bool, String> {
    let invalid = || "Could not decode this image.".to_owned();
    let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    decoder.set_limits(image_limits()).map_err(|_| invalid())?;
    let mut frames = decoder.into_frames();
    frames.next().ok_or_else(invalid)?.map_err(|_| invalid())?;
    Ok(frames.next().transpose().map_err(|_| invalid())?.is_some())
}

#[cfg(test)]
pub(super) fn encode_style_image(
    bytes: &[u8],
    target: ImageTarget,
    nitro: bool,
) -> Result<String, String> {
    let (extension, encoded) = encode_style_bytes(bytes, target, nitro, None)?;
    Ok(style_data_uri(extension, &encoded))
}

/// An encoded upload as the data URI Discord's profile endpoints take.
pub(crate) fn style_data_uri(extension: &str, bytes: &[u8]) -> String {
    format!("data:image/{extension};base64,{}", STANDARD.encode(bytes))
}

/// The upload for a picked image: animated GIFs (and animated WebPs, as GIFs) for Nitro,
/// otherwise a PNG still. `crop` is the editor's frame; without it the image is cut to
/// `target`'s shape around its center. Returns the file extension and bytes.
pub(crate) fn encode_style_bytes(
    bytes: &[u8],
    target: ImageTarget,
    nitro: bool,
    crop: Option<StyleCrop>,
) -> Result<(&'static str, Vec<u8>), String> {
    if bytes.len() as u64 >= STYLE_IMAGE_LIMIT {
        return Err("Choose an image smaller than 10 MB.".into());
    }
    let format =
        image::guess_format(bytes).map_err(|_| "Could not decode this image.".to_owned())?;
    if format == ImageFormat::WebP
        && nitro
        && let Some(gif) = webp_style_gif(
            bytes,
            STYLE_IMAGE_LIMIT as usize,
            crop.map(|crop| (crop, target)),
        )?
    {
        return Ok(("gif", gif));
    }
    let image = decode_style_still(bytes)?;
    let (width, height) = (image.width(), image.height());
    if format == ImageFormat::Gif && nitro && gif_is_animated(bytes)? {
        return match crop.filter(|crop| !crop.is_identity(width, height)) {
            Some(crop) => Ok(("gif", cropped_style_gif(bytes, crop, target)?)),
            None => Ok(("gif", bytes.to_vec())),
        };
    }
    let crop = crop.unwrap_or_else(|| StyleCrop::centered(width, height, target));
    let still = crop.apply(
        image.into_rgba8(),
        target,
        image::imageops::FilterType::Lanczos3,
    );
    let mut png = Cursor::new(Vec::new());
    still
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|_| "Could not encode the image.".to_owned())?;
    Ok(("png", png.into_inner()))
}
