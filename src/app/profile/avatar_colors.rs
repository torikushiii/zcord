use std::{
    io::{BufRead, Cursor, Seek},
    path::Path,
};

use color_thief::{ColorFormat, get_palette};
use image::{ImageReader, Limits, RgbaImage};

pub(crate) fn load(path: &Path) -> Option<[u32; 2]> {
    palette(&decode(ImageReader::open(path).ok()?)?)
}

pub(crate) fn suggestions(bytes: &[u8]) -> Option<[u32; 5]> {
    let [primary, secondary] = palette(&decode(ImageReader::new(Cursor::new(bytes)))?)?;
    let [a, b, c] = tetrad(primary);
    Some([primary, secondary, a, b, c])
}

fn tetrad(primary: u32) -> [u32; 3] {
    let [hue, saturation, lightness] = rgb_to_hsl(primary);
    let (saturation, lightness) = ((saturation + 0.3).min(1.), (lightness + 0.15).min(1.));
    [270., 180., 90.].map(|turn| hsl_to_rgb([(hue + turn) % 360., saturation, lightness]))
}

fn decode<R: BufRead + Seek>(reader: ImageReader<R>) -> Option<RgbaImage> {
    let mut reader = reader.with_guessed_format().ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    Some(reader.decode().ok()?.to_rgba8())
}

fn rgb_to_hsl(color: u32) -> [f64; 3] {
    let [r, g, b] = [16, 8, 0].map(|shift| f64::from((color >> shift) & 0xFF) / 255.);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let lightness = (max + min) / 2.;
    if max == min {
        return [0., 0., lightness];
    }
    let delta = max - min;
    let saturation = if lightness > 0.5 {
        delta / (2. - max - min)
    } else {
        delta / (max + min)
    };
    let hue = if max == r {
        (g - b) / delta + if g < b { 6. } else { 0. }
    } else if max == g {
        (b - r) / delta + 2.
    } else {
        (r - g) / delta + 4.
    };
    [hue * 60., saturation, lightness]
}

fn hsl_to_rgb([hue, saturation, lightness]: [f64; 3]) -> u32 {
    let hue = hue / 360.;
    let channel = |p: f64, q: f64, mut t: f64| {
        if t < 0. {
            t += 1.;
        }
        if t > 1. {
            t -= 1.;
        }
        if t < 1. / 6. {
            p + (q - p) * 6. * t
        } else if t < 0.5 {
            q
        } else if t < 2. / 3. {
            p + (q - p) * (2. / 3. - t) * 6.
        } else {
            p
        }
    };
    let [r, g, b] = if saturation == 0. {
        [lightness; 3]
    } else {
        let q = if lightness < 0.5 {
            lightness * (1. + saturation)
        } else {
            lightness + saturation - lightness * saturation
        };
        let p = 2. * lightness - q;
        [
            channel(p, q, hue + 1. / 3.),
            channel(p, q, hue),
            channel(p, q, hue - 1. / 3.),
        ]
    };
    [r, g, b]
        .map(|value| (value * 255.).round() as u32)
        .into_iter()
        .fold(0, |color, value| (color << 8) | value)
}

fn palette(image: &RgbaImage) -> Option<[u32; 2]> {
    let mut samples = Vec::new();
    for pixel in image.pixels().step_by(10) {
        let [r, g, b, a] = pixel.0;
        if a < 125 || (r > 250 && g > 250 && b > 250) {
            continue;
        }
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
    fn suggestions_brighten_the_main_color_around_a_tetrad() {
        assert_eq!(tetrad(0x21333C), [0x338723, 0x874423, 0x762387]);
        for color in [0x21333C, 0xFFFFFF, 0x000000, 0x5865F2] {
            assert_eq!(hsl_to_rgb(rgb_to_hsl(color)), color);
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
