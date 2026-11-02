use std::path::Path;

use color_thief::{ColorFormat, get_palette};
use image::{ImageReader, Limits, RgbaImage};

pub(crate) fn load(path: &Path) -> Option<[u32; 2]> {
    let mut reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    palette(&reader.decode().ok()?.to_rgba8())
}

fn palette(image: &RgbaImage) -> Option<[u32; 2]> {
    // Discord samples every tenth pixel and quantizes five colors, using the first two.
    let mut samples = Vec::new();
    for pixel in image.pixels().step_by(10) {
        let [r, g, b, a] = pixel.0;
        if a < 125 || (r > 250 && g > 250 && b > 250) {
            continue;
        }
        // This port strides by the channel count too; repeat samples to retain each one.
        samples.extend_from_slice(&[r, g, b, r, g, b, r, g, b]);
    }
    if samples.is_empty() {
        return None;
    }
    let colors = get_palette(&samples, ColorFormat::Rgb, 1, 5).ok()?;
    let first = colors.first()?;
    let second = colors.get(1).unwrap_or(first);
    let rgb = |color: &color_thief::Color| {
        (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
    };
    Some([rgb(first), rgb(second)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn avatar_colors_follow_the_image_not_a_fixed_dark_theme() {
        for (source, light) in [([220, 210, 175, 255], true), ([25, 35, 60, 255], false)] {
            let image = RgbaImage::from_pixel(128, 128, Rgba(source));
            let colors = palette(&image).unwrap();
            for color in colors {
                for (shift, expected) in [(16, source[0]), (8, source[1]), (0, source[2])] {
                    assert!((((color >> shift) & 0xFF) as i16 - i16::from(expected)).abs() <= 8);
                }
            }
            assert_eq!(
                super::super::ProfileTheme {
                    primary: colors[0],
                    secondary: colors[1]
                }
                .is_light(),
                light
            );
        }
    }

    #[test]
    fn transparent_pixels_do_not_choose_the_theme() {
        let image = RgbaImage::from_fn(128, 128, |x, _| {
            if x < 96 {
                Rgba([255, 0, 255, 0])
            } else {
                Rgba([220, 210, 175, 255])
            }
        });
        let colors = palette(&image).unwrap();
        assert!(((colors[0] >> 8) & 0xFF) > 180);
        assert!(palette(&RgbaImage::new(0, 0)).is_none());
        assert!(palette(&RgbaImage::from_pixel(128, 128, Rgba([255, 0, 255, 0]))).is_none());
        assert!(palette(&RgbaImage::from_pixel(128, 128, Rgba([255; 4]))).is_none());
    }
}
