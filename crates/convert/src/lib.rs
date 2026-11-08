/// BT.709 limited range.
pub fn nv12_to_bgra(y: &[u8], uv: &[u8], stride: usize, width: usize, height: usize) -> Vec<u8> {
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

pub fn scale_bgra(
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

pub fn fit_dimensions(src_w: usize, src_h: usize, max_w: usize, max_h: usize) -> (usize, usize) {
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

/// BT.709 limited range, the source fitted and centred inside `dst_w` × `dst_h` on black.
pub fn bgra_to_nv12(
    bgra: &[u8],
    stride: usize,
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> Vec<u8> {
    let mut output = vec![0; dst_w.saturating_mul(dst_h).saturating_mul(3) / 2];
    if dst_w == 0 || dst_h == 0 || !dst_w.is_multiple_of(2) || !dst_h.is_multiple_of(2) {
        return output;
    }
    let luma_len = dst_w * dst_h;
    let (luma, chroma) = output.split_at_mut(luma_len);
    luma.fill(16);
    chroma.fill(128);
    if src_w == 0
        || src_h == 0
        || stride < src_w.saturating_mul(4)
        || bgra.len() < stride.saturating_mul(src_h)
    {
        return output;
    }
    // An odd last column or row is dropped, as a window's height often is odd; the rest is
    // fitted to the output, read in place when it already fits exactly.
    let (even_w, even_h) = (src_w & !1, src_h & !1);
    if even_w < 2 || even_h < 2 {
        return output;
    }
    let (scaled_w, scaled_h) = fit_even(even_w, even_h, dst_w, dst_h);
    let unscaled = (scaled_w, scaled_h) == (even_w, even_h);
    // Even offsets keep each 2×2 block on one chroma sample.
    let left = ((dst_w - scaled_w) / 2) & !1;
    let top = ((dst_h - scaled_h) / 2) & !1;
    let scaled;
    let (source, source_stride) = if unscaled {
        (bgra, stride)
    } else {
        scaled = if scaled_w > even_w {
            resize_bilinear(bgra, stride, even_w, even_h, scaled_w, scaled_h)
        } else {
            scale_bgra(bgra, stride, src_w, src_h, scaled_w, scaled_h)
        };
        (scaled.as_slice(), scaled_w * 4)
    };
    // Rows of 2×2 blocks are independent, so bands of them convert on separate threads; a
    // 1080p frame on one core takes long enough to cap the frame rate.
    let pairs = scaled_h / 2;
    let threads = std::thread::available_parallelism()
        .map_or(1, |threads| threads.get())
        .clamp(1, MAX_THREADS)
        .min(pairs.div_ceil(MIN_PAIRS_PER_THREAD));
    let band = pairs.div_ceil(threads);
    let luma_area = &mut luma[top * dst_w..(top + scaled_h) * dst_w];
    let chroma_area = &mut chroma[(top / 2) * dst_w..(top / 2 + pairs) * dst_w];
    let bands = luma_area
        .chunks_mut(band * 2 * dst_w)
        .zip(chroma_area.chunks_mut(band * dst_w))
        .enumerate();
    let rows = Rows {
        source,
        stride: source_stride,
        width: scaled_w,
        dst_w,
        left,
    };
    if threads == 1 {
        for (index, (luma, chroma)) in bands {
            rows.convert(index * band, luma, chroma);
        }
    } else {
        std::thread::scope(|scope| {
            for (index, (luma, chroma)) in bands {
                let rows = &rows;
                scope.spawn(move || rows.convert(index * band, luma, chroma));
            }
        });
    }
    output
}

/// The largest even size with the shape of `width` × `height` that fits inside `max_w` ×
/// `max_h`, scaling up as well as down.
pub fn fit_even(width: usize, height: usize, max_w: usize, max_h: usize) -> (usize, usize) {
    if width == 0 || height == 0 || max_w < 2 || max_h < 2 {
        return (0, 0);
    }
    let scale = (max_w as f64 / width as f64).min(max_h as f64 / height as f64);
    let side = |length: usize, max: usize| {
        (((length as f64 * scale).round() as usize).clamp(2, max) & !1).max(2)
    };
    (side(width, max_w), side(height, max_h))
}

/// Bilinear resize, for scaling a picture up: the box scaler only averages, which would
/// turn each source pixel into a visible block. Bands of rows run on separate threads.
pub fn resize_bilinear(
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
    let columns: Vec<_> = (0..dst_w).map(|x| sample(x, dst_w, src_w)).collect();
    let threads = std::thread::available_parallelism()
        .map_or(1, |threads| threads.get())
        .clamp(1, MAX_THREADS)
        .min(dst_h.div_ceil(MIN_PAIRS_PER_THREAD * 2));
    let band = dst_h.div_ceil(threads);
    let resize_band = |first_row: usize, rows: &mut [u8]| {
        for (offset, row) in rows.chunks_exact_mut(dst_w * 4).enumerate() {
            let (y0, y1, fy) = sample(first_row + offset, dst_h, src_h);
            let (top, bottom) = (&bgra[y0 * stride..], &bgra[y1 * stride..]);
            for (pixel, &(x0, x1, fx)) in row.as_chunks_mut::<4>().0.iter_mut().zip(&columns) {
                for channel in 0..4 {
                    let blend = |line: &[u8]| {
                        u32::from(line[x0 * 4 + channel]) * (256 - fx)
                            + u32::from(line[x1 * 4 + channel]) * fx
                    };
                    let value = blend(top) * (256 - fy) + blend(bottom) * fy;
                    pixel[channel] = ((value + 32_768) >> 16) as u8;
                }
            }
        }
    };
    if threads == 1 {
        resize_band(0, &mut output);
    } else {
        std::thread::scope(|scope| {
            for (index, rows) in output.chunks_mut(band * dst_w * 4).enumerate() {
                let resize_band = &resize_band;
                scope.spawn(move || resize_band(index * band, rows));
            }
        });
    }
    output
}

/// Where output position `position` of `out_len` samples a line of `in_len`: its two
/// neighbouring source positions and the second's weight out of 256, centres aligned.
fn sample(position: usize, out_len: usize, in_len: usize) -> (usize, usize, u32) {
    let source = ((position as f64 + 0.5) * in_len as f64 / out_len as f64 - 0.5).max(0.);
    let first = (source as usize).min(in_len - 1);
    let second = (first + 1).min(in_len - 1);
    let weight = (((source - first as f64) * 256.).round() as u32).min(256);
    (first, second, weight)
}

const MAX_THREADS: usize = 8;
/// Below this many block rows a band isn't worth a thread.
const MIN_PAIRS_PER_THREAD: usize = 32;

/// The source picture and where it lands in the output, shared by every band.
struct Rows<'a> {
    source: &'a [u8],
    stride: usize,
    width: usize,
    dst_w: usize,
    left: usize,
}

impl Rows<'_> {
    /// Converts the band of block rows starting at `first_pair`, whose luma rows (two per
    /// block row) and chroma rows are `luma` and `chroma`.
    fn convert(&self, first_pair: usize, luma: &mut [u8], chroma: &mut [u8]) {
        let (width, dst_w, left) = (self.width, self.dst_w, self.left);
        let row_bytes = width * 4;
        let luma_pairs = luma.chunks_exact_mut(2 * dst_w);
        for (pair, (luma, uv)) in luma_pairs.zip(chroma.chunks_exact_mut(dst_w)).enumerate() {
            let row = (first_pair + pair) * 2;
            let upper = &self.source[row * self.stride..][..row_bytes];
            let lower = &self.source[(row + 1) * self.stride..][..row_bytes];
            let (luma_upper, luma_lower) = luma.split_at_mut(dst_w);
            let blocks = upper
                .as_chunks::<8>()
                .0
                .iter()
                .zip(lower.as_chunks::<8>().0)
                .zip(luma_upper[left..left + width].as_chunks_mut::<2>().0)
                .zip(luma_lower[left..left + width].as_chunks_mut::<2>().0)
                .zip(uv[left..left + width].as_chunks_mut::<2>().0);
            for ((((upper, lower), luma_upper), luma_lower), uv) in blocks {
                let [b0, g0, r0, _, b1, g1, r1, _] = upper.map(i32::from);
                let [b2, g2, r2, _, b3, g3, r3, _] = lower.map(i32::from);
                *luma_upper = [luma_of(r0, g0, b0), luma_of(r1, g1, b1)];
                *luma_lower = [luma_of(r2, g2, b2), luma_of(r3, g3, b3)];
                let (r, g, b) = (r0 + r1 + r2 + r3, g0 + g1 + g2 + g3, b0 + b1 + b2 + b3);
                // Sums of four pixels: the extra 2 bits of shift average them. The coefficients
                // keep every result within 16..=240, so nothing needs clamping.
                *uv = [
                    (((-26 * r - 86 * g + 112 * b + 512) >> 10) + 128) as u8,
                    (((112 * r - 102 * g - 10 * b + 512) >> 10) + 128) as u8,
                ];
            }
        }
    }
}

/// BT.709 limited-range luma in 8-bit fixed point; always within 16..=235.
#[inline(always)]
fn luma_of(r: i32, g: i32, b: i32) -> u8 {
    (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16) as u8
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
    fn bilinear_upscaling_blends_instead_of_blocking() {
        // Black then white, doubled: the middle pixels land between the two.
        let source = [[0u8, 0, 0, 255], [255, 255, 255, 255]].concat();
        let wide = resize_bilinear(&source, 8, 2, 1, 4, 1);
        let blue: Vec<u8> = wide.chunks_exact(4).map(|pixel| pixel[0]).collect();
        assert_eq!(blue.first(), Some(&0));
        assert_eq!(blue.last(), Some(&255));
        assert!(blue.windows(2).all(|pair| pair[0] < pair[1]));
        let flat = [7u8, 70, 140, 255].repeat(3 * 3);
        assert!(
            resize_bilinear(&flat, 12, 3, 3, 8, 6)
                .chunks_exact(4)
                .all(|pixel| pixel == [7, 70, 140, 255])
        );
        assert_eq!(fit_even(1922, 1118, 2560, 1440), (2476, 1440));
    }

    #[test]
    fn an_odd_last_row_is_dropped_instead_of_scaling() {
        // 4×3 red into 4×2: the third row is cut, the rest converts unscaled and unshifted.
        let mut picture = [0u8, 0, 255, 255].repeat(4 * 2);
        picture.extend([255u8; 16]);
        let nv12 = bgra_to_nv12(&picture, 16, 4, 3, 4, 2);
        assert_eq!(&nv12[..8], &[63; 8]);
        assert_eq!(&nv12[8..], &[102, 240, 102, 240]);
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
