/// BT.709 limited range.
pub(crate) fn nv12_to_bgra(
    y: &[u8],
    uv: &[u8],
    stride: usize,
    width: usize,
    height: usize,
) -> Vec<u8> {
    let mut bgra = vec![0; width.saturating_mul(height).saturating_mul(4)];
    if width == 0 || height == 0 || stride < width {
        return bgra;
    }
    for row in 0..height {
        let Some(luma) = y.get(row * stride..row * stride + width) else {
            return bgra;
        };
        let chroma_row = row / 2;
        let Some(chroma) = uv.get(chroma_row * stride..chroma_row * stride + width) else {
            return bgra;
        };
        for column in 0..width {
            let luma = f32::from(luma[column]);
            let u = f32::from(chroma[column & !1]);
            let v = f32::from(chroma[(column & !1) + 1]);
            let luma = (luma - 16.) * (255. / 219.);
            let u = (u - 128.) * (255. / 224.);
            let v = (v - 128.) * (255. / 224.);
            let red = luma + 1.5748 * v;
            let green = luma - 0.1873 * u - 0.4681 * v;
            let blue = luma + 1.8556 * u;
            let at = (row * width + column) * 4;
            bgra[at] = blue.clamp(0., 255.) as u8;
            bgra[at + 1] = green.clamp(0., 255.) as u8;
            bgra[at + 2] = red.clamp(0., 255.) as u8;
            bgra[at + 3] = 255;
        }
    }
    bgra
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nv12(y: &[u8], u: u8, v: u8, width: usize, height: usize) -> (Vec<u8>, Vec<u8>) {
        let mut uv = vec![0; width * height];
        for pair in uv.chunks_exact_mut(2) {
            pair[0] = u;
            pair[1] = v;
        }
        (y.to_vec(), uv)
    }

    #[test]
    fn bt709_limited_black_white_and_grey() {
        let width = 2;
        let height = 2;
        let (y, uv) = nv12(&[16; 4], 128, 128, width, height);
        assert_eq!(
            nv12_to_bgra(&y, &uv, width, width, height),
            [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]
        );

        let (y, uv) = nv12(&[235; 4], 128, 128, width, height);
        assert_eq!(
            nv12_to_bgra(&y, &uv, width, width, height),
            [
                255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255
            ]
        );

        let (y, uv) = nv12(&[128; 4], 128, 128, width, height);
        let grey = nv12_to_bgra(&y, &uv, width, width, height);
        assert!(
            grey.chunks_exact(4)
                .all(|pixel| pixel == [130, 130, 130, 255])
        );
    }

    #[test]
    fn bgra_converts_known_colours_to_bt709_limited_nv12() {
        let red = [0u8, 0, 255, 255].repeat(4);
        let nv12 = bgra_to_nv12(&red, 8, 2, 2, 2, 2);
        assert_eq!(&nv12[..4], &[63, 63, 63, 63]);
        assert_eq!(&nv12[4..], &[102, 240]);

        let black = [0u8; 16];
        assert_eq!(
            bgra_to_nv12(&black, 8, 2, 2, 2, 2),
            [16, 16, 16, 16, 128, 128]
        );

        let white = [255u8; 16];
        let nv12 = bgra_to_nv12(&white, 8, 2, 2, 2, 2);
        assert_eq!(&nv12[..4], &[235, 235, 235, 235]);
        assert_eq!(&nv12[4..], &[128, 128]);
    }

    #[test]
    fn bgra_letterboxes_without_stretching() {
        let red = [0u8, 0, 255, 255].repeat(4);
        let nv12 = bgra_to_nv12(&red, 8, 2, 2, 6, 2);
        assert_eq!(&nv12[..6], &[16, 16, 63, 63, 16, 16]);
        assert_eq!(&nv12[6..12], &[16, 16, 63, 63, 16, 16]);
        assert_eq!(&nv12[12..], &[128, 128, 102, 240, 128, 128]);
    }

    #[test]
    fn box_scaler_downsamples_and_fits_a_thumbnail() {
        let mut source = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let at = (y * 4 + x) * 4;
                let red = x >= 2;
                source[at] = if red { 0 } else { 255 };
                source[at + 1] = 0;
                source[at + 2] = if red { 255 } else { 0 };
                source[at + 3] = 255;
            }
        }
        let scaled = scale_bgra(&source, 16, 4, 4, 2, 2);
        assert_eq!(scaled.len(), 16);
        assert_eq!(&scaled[..4], &[255, 0, 0, 255]);
        assert_eq!(&scaled[4..8], &[0, 0, 255, 255]);
        assert_eq!(&scaled[8..12], &[255, 0, 0, 255]);
        assert_eq!(&scaled[12..], &[0, 0, 255, 255]);
        assert_eq!(fit_dimensions(1920, 1080, 480, 270), (480, 270));
        assert_eq!(fit_dimensions(1080, 1920, 480, 270), (152, 270));
        assert_eq!(fit_dimensions(100, 100, 480, 270), (100, 100));
    }

    #[test]
    fn bt709_limited_red_and_strided_rows() {
        let width = 2;
        let height = 2;
        let (y, mut uv) = nv12(&[63; 4], 102, 240, width, height);
        let red = nv12_to_bgra(&y, &uv, width, width, height);
        assert!(red.chunks_exact(4).all(|pixel| pixel == [0, 0, 255, 255]));

        for pair in uv.chunks_exact_mut(2) {
            pair[0] = 128;
            pair[1] = 128;
        }
        let padded = [16, 16, 99, 99, 16, 16, 99, 99];
        let bgra = nv12_to_bgra(&padded, &uv, 4, width, height);
        assert!(bgra.chunks_exact(4).all(|pixel| pixel == [0, 0, 0, 255]));
    }
}

pub(crate) fn scale_bgra(
    bgra: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<u8> {
    let mut output = vec![0; dst_w.saturating_mul(dst_h).saturating_mul(4)];
    if dst_w == 0
        || dst_h == 0
        || src_w == 0
        || src_h == 0
        || stride < src_w.saturating_mul(4)
        || bgra.len() < stride.saturating_mul(src_h)
    {
        return output;
    }
    for dy in 0..dst_h {
        let sy0 = dy * src_h / dst_h;
        let sy1 = ((dy + 1) * src_h / dst_h).max(sy0 + 1).min(src_h);
        for dx in 0..dst_w {
            let sx0 = dx * src_w / dst_w;
            let sx1 = ((dx + 1) * src_w / dst_w).max(sx0 + 1).min(src_w);
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for y in sy0..sy1 {
                for x in sx0..sx1 {
                    let at = y * stride + x * 4;
                    for channel in 0..4 {
                        sum[channel] += u32::from(bgra[at + channel]);
                    }
                    count += 1;
                }
            }
            let count = count.max(1);
            let at = (dy * dst_w + dx) * 4;
            for channel in 0..4 {
                output[at + channel] = (sum[channel] / count) as u8;
            }
        }
    }
    output
}

pub(crate) fn fit_dimensions(
    src_w: usize,
    src_h: usize,
    max_w: usize,
    max_h: usize,
) -> (usize, usize) {
    if src_w == 0 || src_h == 0 || max_w == 0 || max_h == 0 {
        return (0, 0);
    }
    let scale = (max_w as f64 / src_w as f64)
        .min(max_h as f64 / src_h as f64)
        .min(1.);
    (
        ((src_w as f64 * scale).round() as usize).clamp(1, max_w),
        ((src_h as f64 * scale).round() as usize).clamp(1, max_h),
    )
}

/// BT.709 limited range.
pub(crate) fn bgra_to_nv12(
    bgra: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<u8> {
    let mut output = vec![0; dst_w.saturating_mul(dst_h).saturating_mul(3) / 2];
    if dst_w == 0 || dst_h == 0 || dst_w % 2 != 0 || dst_h % 2 != 0 {
        return output;
    }
    let luma_len = dst_w * dst_h;
    output[..luma_len].fill(16);
    output[luma_len..].fill(128);
    if src_w == 0
        || src_h == 0
        || stride < src_w.saturating_mul(4)
        || bgra.len() < stride.saturating_mul(src_h)
    {
        return output;
    }
    let scale = (dst_w as f64 / src_w as f64).min(dst_h as f64 / src_h as f64);
    let scaled_w = (((src_w as f64 * scale).round() as usize).clamp(1, dst_w) & !1).max(2);
    let scaled_h = (((src_h as f64 * scale).round() as usize).clamp(1, dst_h) & !1).max(2);
    let left = (dst_w - scaled_w) / 2;
    let top = (dst_h - scaled_h) / 2;
    let scaled = scale_bgra(bgra, stride, src_w, src_h, scaled_w, scaled_h);
    let mut rgb = vec![[0u8; 3]; dst_w * dst_h];
    for dy in 0..scaled_h {
        for dx in 0..scaled_w {
            let at = (dy * scaled_w + dx) * 4;
            rgb[(top + dy) * dst_w + left + dx] = [scaled[at], scaled[at + 1], scaled[at + 2]];
        }
    }
    for (index, [blue, green, red]) in rgb.iter().copied().enumerate() {
        let (red, green, blue) = (
            f32::from(red) / 255.,
            f32::from(green) / 255.,
            f32::from(blue) / 255.,
        );
        let luma = (0.2126 * red + 0.7152 * green + 0.0722 * blue).clamp(0., 1.);
        output[index] = (16. + 219. * luma).round().clamp(16., 235.) as u8;
    }
    for dy in (0..dst_h).step_by(2) {
        for dx in (0..dst_w).step_by(2) {
            let mut sum = [0u32; 3];
            for y in dy..(dy + 2).min(dst_h) {
                for x in dx..(dx + 2).min(dst_w) {
                    let [blue, green, red] = rgb[y * dst_w + x];
                    sum[0] += u32::from(red);
                    sum[1] += u32::from(green);
                    sum[2] += u32::from(blue);
                }
            }
            let [red, green, blue] = sum.map(|channel| channel as f32 / (4. * 255.));
            let luma = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
            let u = 128. + 224. * (blue - luma) / 1.8556;
            let v = 128. + 224. * (red - luma) / 1.5748;
            let at = luma_len + (dy / 2) * dst_w + dx;
            output[at] = u.round().clamp(0., 255.) as u8;
            output[at + 1] = v.round().clamp(0., 255.) as u8;
        }
    }
    output
}
