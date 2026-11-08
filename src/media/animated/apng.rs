//! PNG and APNG frames, composited to the APNG spec.
//!
//! `image`'s APNG compositor saves the restore point for `dispose_op = previous` before it
//! clears a background-disposed frame, so a previous-disposed frame that follows brings the
//! cleared pixels back. Discord's profile effects lean on that pairing.

use super::MAX_DIMENSION;
use anyhow::{Result, ensure};
use image::{
    Delay, Frame, Frames, GenericImage, GenericImageView, ImageError, ImageFormat, ImageResult,
    Pixel, Rgba, RgbaImage,
    error::{DecodingError, ImageFormatHint},
};
use png::{BlendOp, ColorType, DisposeOp, FrameControl, Transformations};
use std::io::{BufRead, Seek};

const MAX_ALLOC: usize = 512 << 20;

pub(super) fn frames<'a, R: BufRead + Seek + 'a>(reader: R) -> Result<((u32, u32), Frames<'a>)> {
    let mut decoder = png::Decoder::new_with_limits(reader, png::Limits { bytes: MAX_ALLOC });
    decoder.set_transformations(
        Transformations::EXPAND | Transformations::STRIP_16 | Transformations::ALPHA,
    );
    let reader = decoder.read_info()?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    ensure!(
        width <= MAX_DIMENSION && height <= MAX_DIMENSION,
        "{width}x{height} PNG is too large"
    );
    let animation = info.animation_control();
    let composer = Composer {
        remaining: animation.map_or(1, |actl| actl.num_frames),
        skip_default: animation.is_some() && info.frame_control().is_none(),
        reader,
        canvas: RgbaImage::new(width, height),
        disposal: None,
    };
    Ok(((width, height), Frames::new(Box::new(composer))))
}

struct Region {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

enum Disposal {
    Background(Region),
    Previous(Region, RgbaImage),
}

struct Composer<R: BufRead + Seek> {
    reader: png::Reader<R>,
    canvas: RgbaImage,
    remaining: u32,
    skip_default: bool,
    disposal: Option<Disposal>,
}

impl<R: BufRead + Seek> Iterator for Composer<R> {
    type Item = ImageResult<Frame>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let frame = self.compose();
        self.remaining = if frame.is_ok() { self.remaining - 1 } else { 0 };
        Some(frame)
    }
}

impl<R: BufRead + Seek> Composer<R> {
    fn compose(&mut self) -> ImageResult<Frame> {
        if std::mem::take(&mut self.skip_default) {
            self.read()?;
        }
        let (source, control) = self.read()?;
        let region = Region {
            x: control.map_or(0, |fc| fc.x_offset),
            y: control.map_or(0, |fc| fc.y_offset),
            width: source.width(),
            height: source.height(),
        };
        if region
            .x
            .checked_add(region.width)
            .is_none_or(|right| right > self.canvas.width())
            || region
                .y
                .checked_add(region.height)
                .is_none_or(|bottom| bottom > self.canvas.height())
        {
            return Err(decoding_error("APNG frame lies outside the canvas"));
        }

        match self.disposal.take() {
            None => {}
            Some(Disposal::Background(old)) => {
                for y in old.y..old.y + old.height {
                    for x in old.x..old.x + old.width {
                        self.canvas.put_pixel(x, y, Rgba([0; 4]));
                    }
                }
            }
            Some(Disposal::Previous(old, saved)) => {
                self.canvas
                    .copy_from(&saved, old.x, old.y)
                    .map_err(|_| decoding_error("APNG restore lies outside the canvas"))?;
            }
        }

        let dispose = control.map_or(DisposeOp::None, |fc| fc.dispose_op);
        let saved = (dispose == DisposeOp::Previous).then(|| {
            self.canvas
                .view(region.x, region.y, region.width, region.height)
                .to_image()
        });
        match control.map_or(BlendOp::Source, |fc| fc.blend_op) {
            BlendOp::Source => {
                self.canvas
                    .copy_from(&source, region.x, region.y)
                    .map_err(|_| decoding_error("APNG frame lies outside the canvas"))?;
            }
            BlendOp::Over => {
                for (x, y, pixel) in source.enumerate_pixels() {
                    self.canvas
                        .get_pixel_mut(region.x + x, region.y + y)
                        .blend(pixel);
                }
            }
        }
        self.disposal = match (dispose, saved) {
            (DisposeOp::Background, _) => Some(Disposal::Background(region)),
            (DisposeOp::Previous, Some(saved)) => Some(Disposal::Previous(region, saved)),
            _ => None,
        };

        Ok(Frame::from_parts(
            self.canvas.clone(),
            0,
            0,
            control.map_or(Delay::from_numer_denom_ms(0, 1), delay),
        ))
    }

    fn read(&mut self) -> ImageResult<(RgbaImage, Option<FrameControl>)> {
        let size = self
            .reader
            .output_buffer_size()
            .ok_or_else(|| decoding_error("PNG frame is too large"))?;
        let mut buffer = vec![0; size];
        let output = self.reader.next_frame(&mut buffer).map_err(png_error)?;
        buffer.truncate(output.buffer_size());
        let rgba = match output.color_type {
            ColorType::Rgba => buffer,
            ColorType::GrayscaleAlpha => buffer
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|&[gray, alpha]| [gray, gray, gray, alpha])
                .collect(),
            other => return Err(decoding_error(&format!("unexpected PNG output {other:?}"))),
        };
        let image = RgbaImage::from_raw(output.width, output.height, rgba)
            .ok_or_else(|| decoding_error("PNG frame is shorter than its size"))?;
        Ok((image, self.reader.info().frame_control().copied()))
    }
}

fn delay(control: FrameControl) -> Delay {
    let denominator = match control.delay_den {
        0 => 100,
        denominator => u32::from(denominator),
    };
    Delay::from_numer_denom_ms(u32::from(control.delay_num) * 1000, denominator)
}

fn png_error(error: png::DecodingError) -> ImageError {
    ImageError::Decoding(DecodingError::new(
        ImageFormatHint::Exact(ImageFormat::Png),
        error,
    ))
}

fn decoding_error(message: &str) -> ImageError {
    ImageError::Decoding(DecodingError::new(
        ImageFormatHint::Exact(ImageFormat::Png),
        message.to_owned(),
    ))
}
