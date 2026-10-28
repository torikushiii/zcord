//! Decoding a large GIF or clip takes seconds; re-showing one reads these frames instead.
//!
//! Format (little-endian): magic `ZCFR`, version `1`, frame count `u32`, width `u32`,
//! height `u32`; then per frame the delay numerator and denominator (`u32` each)
//! followed by `width * height * 4` raw BGRA bytes, exactly as stored in
//! `RenderImage`.

use super::images::{touch_due, touch_modified};
use anyhow::{Context, ensure};
use gpui::RenderImage;
use image::{Delay, Frame, RgbaImage};
use std::{
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::Path,
    sync::Arc,
    time::SystemTime,
};

const MAGIC: &[u8; 4] = b"ZCFR";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 17;
const FRAME_HEADER_LEN: usize = 8;
const MAX_DIMENSION: u32 = 4096;
const MAX_FRAMES: u32 = 600;
const MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Callers only save images produced by the animated and video decoders, which
/// resize every frame to one target size.
fn write(image: &RenderImage, out: &mut impl Write) -> io::Result<u64> {
    let frame_count = u32::try_from(image.frame_count()).unwrap_or(u32::MAX);
    if frame_count == 0 {
        return Ok(0);
    }
    let (width, height) = dimensions(image, 0);
    out.write_all(MAGIC)?;
    out.write_all(&[VERSION])?;
    out.write_all(&frame_count.to_le_bytes())?;
    out.write_all(&width.to_le_bytes())?;
    out.write_all(&height.to_le_bytes())?;
    let mut written = HEADER_LEN as u64;
    for index in 0..frame_count as usize {
        let (numerator, denominator) = image.delay(index).numer_denom_ms();
        out.write_all(&numerator.to_le_bytes())?;
        out.write_all(&denominator.to_le_bytes())?;
        let pixels = image.as_bytes(index).unwrap_or_default();
        out.write_all(pixels)?;
        written += FRAME_HEADER_LEN as u64 + pixels.len() as u64;
    }
    Ok(written)
}

pub(crate) fn save(path: &Path, image: &RenderImage) -> io::Result<u64> {
    if image.frame_count() == 0 {
        return Ok(0);
    }
    let temp = path.with_extension(format!(
        "tmp-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let result = (|| {
        let file = File::options().write(true).create_new(true).open(&temp)?;
        let mut out = BufWriter::new(file);
        let written = write(image, &mut out)?;
        out.flush()?;
        drop(out);
        fs::rename(&temp, path)?;
        Ok(written)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub(crate) async fn load(path: &Path) -> Option<(Arc<RenderImage>, usize)> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                log::debug!("Could not read frame cache {}: {error}", path.display());
            }
            return None;
        }
    };
    match decode(&bytes) {
        Ok(decoded) => {
            if let Ok(metadata) = tokio::fs::metadata(path).await
                && let Ok(modified) = metadata.modified()
                && touch_due(modified, SystemTime::now())
            {
                let path = path.to_path_buf();
                let _ = tokio::task::spawn_blocking(move || touch_modified(&path)).await;
            }
            Some(decoded)
        }
        Err(error) => {
            log::debug!(
                "Discarding corrupt frame cache {}: {error:#}",
                path.display()
            );
            let _ = tokio::fs::remove_file(path).await;
            None
        }
    }
}

fn decode(bytes: &[u8]) -> anyhow::Result<(Arc<RenderImage>, usize)> {
    ensure!(bytes.len() >= HEADER_LEN, "frame cache header is truncated");
    ensure!(bytes.starts_with(MAGIC), "invalid frame cache magic");
    ensure!(
        bytes[4] == VERSION,
        "unsupported frame cache version {}",
        bytes[4]
    );
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "frame cache file is too large"
    );
    let frame_count = u32_at(bytes, 5)?;
    let width = u32_at(bytes, 9)?;
    let height = u32_at(bytes, 13)?;
    ensure!(
        frame_count > 0 && frame_count <= MAX_FRAMES,
        "invalid frame count"
    );
    ensure!(
        width > 0 && height > 0 && width <= MAX_DIMENSION && height <= MAX_DIMENSION,
        "invalid frame dimensions"
    );
    let frame_bytes = u64::from(width) * u64::from(height) * 4;
    let expected = u64::from(frame_count) * (FRAME_HEADER_LEN as u64 + frame_bytes);
    ensure!(
        bytes.len() as u64 == HEADER_LEN as u64 + expected,
        "frame cache length mismatch"
    );
    let frame_bytes = usize::try_from(frame_bytes).context("frame cache size overflow")?;
    let mut frames = Vec::with_capacity(frame_count as usize);
    let mut offset = HEADER_LEN;
    for _ in 0..frame_count {
        let numerator = u32_at(bytes, offset)?;
        let denominator = u32_at(bytes, offset + 4)?;
        ensure!(denominator != 0, "frame delay denominator is zero");
        offset += FRAME_HEADER_LEN;
        let pixels = bytes
            .get(offset..offset + frame_bytes)
            .context("frame cache pixel data is truncated")?;
        offset += frame_bytes;
        let pixels = RgbaImage::from_raw(width, height, pixels.to_vec())
            .context("frame cache pixel buffer has the wrong length")?;
        frames.push(Frame::from_parts(
            pixels,
            0,
            0,
            Delay::from_numer_denom_ms(numerator, denominator),
        ));
    }
    let bytes = frame_bytes
        .checked_mul(frame_count as usize)
        .context("frame cache size overflow")?;
    Ok((Arc::new(RenderImage::new(frames)), bytes))
}

fn dimensions(image: &RenderImage, index: usize) -> (u32, u32) {
    let size = image.size(index);
    (
        u32::try_from(size.width.0).unwrap_or(0),
        u32::try_from(size.height.0).unwrap_or(0),
    )
}

fn u32_at(bytes: &[u8], offset: usize) -> anyhow::Result<u32> {
    let slice = bytes
        .get(offset..offset + 4)
        .context("frame cache header is truncated")?;
    let mut value = [0; 4];
    value.copy_from_slice(slice);
    Ok(u32::from_le_bytes(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use std::time::Duration;

    fn temp_directory() -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("zcord-frames-test-{:016x}", rand::random::<u64>()));
        fs::create_dir(&directory).unwrap();
        directory
    }

    fn image() -> RenderImage {
        let frames = [(0u8, (50, 1)), (1, (100, 3)), (2, (200, 7))];
        RenderImage::new(
            frames
                .into_iter()
                .map(|(frame, (numerator, denominator))| {
                    let pixels = RgbaImage::from_fn(3, 2, |x, y| {
                        let seed = frame * 20 + (y * 3 + x) as u8 * 2;
                        Rgba([seed, seed + 1, seed + 2, seed + 3])
                    });
                    Frame::from_parts(
                        pixels,
                        0,
                        0,
                        Delay::from_numer_denom_ms(numerator, denominator),
                    )
                })
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn round_trip_preserves_frames_delays_and_pixels() {
        let image = image();
        let mut encoded = Vec::new();
        assert_eq!(write(&image, &mut encoded).unwrap(), encoded.len() as u64);
        let (decoded, bytes) = decode(&encoded).unwrap();

        assert_eq!(decoded.frame_count(), 3);
        assert_eq!(bytes, 3 * 2 * 4 * 3);
        for index in 0..3 {
            assert_eq!(decoded.size(index), image.size(index));
            assert_eq!(decoded.as_bytes(index), image.as_bytes(index));
            assert_eq!(
                decoded.delay(index).numer_denom_ms(),
                image.delay(index).numer_denom_ms()
            );
        }
    }

    #[test]
    fn truncated_file_is_an_error() {
        let mut encoded = Vec::new();
        write(&image(), &mut encoded).unwrap();
        for length in [0, 4, HEADER_LEN, encoded.len() - 1] {
            assert!(decode(&encoded[..length]).is_err(), "length {length}");
        }
    }

    #[test]
    fn wrong_magic_is_an_error() {
        let mut encoded = Vec::new();
        write(&image(), &mut encoded).unwrap();
        encoded[0] = b'X';
        assert!(decode(&encoded).is_err());
    }

    #[test]
    fn saved_file_matches_streamed_format() {
        let directory = temp_directory();
        let path = directory.join("test.img");
        let image = image();
        let mut encoded = Vec::new();
        write(&image, &mut encoded).unwrap();

        assert_eq!(save(&path, &image).unwrap(), encoded.len() as u64);
        assert_eq!(fs::read(&path).unwrap(), encoded);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn empty_image_writes_nothing_and_leaves_no_file() {
        let directory = temp_directory();
        let path = directory.join("empty.img");
        let image = RenderImage::new(Vec::<Frame>::new());
        let mut encoded = Vec::new();

        assert_eq!(write(&image, &mut encoded).unwrap(), 0);
        assert!(encoded.is_empty());
        assert_eq!(save(&path, &image).unwrap(), 0);
        assert!(!path.exists());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn write_propagates_errors() {
        let mut buffer = [0; HEADER_LEN];
        let mut out = io::Cursor::new(buffer.as_mut_slice());
        assert_eq!(
            write(&image(), &mut out).unwrap_err().kind(),
            io::ErrorKind::WriteZero
        );
    }

    #[test]
    fn failed_save_removes_temp_file() {
        let directory = temp_directory();
        let path = directory.join("test.img");
        fs::create_dir(&path).unwrap();

        assert!(save(&path, &image()).is_err());
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn frames_cache_hit_does_not_need_the_downloaded_source() {
        let directory = temp_directory();
        let path = directory.join("test.img");
        let image = RenderImage::new(
            [50, 100]
                .into_iter()
                .map(|delay| {
                    Frame::from_parts(
                        RgbaImage::new(1, 1),
                        0,
                        0,
                        Delay::from_numer_denom_ms(delay, 1),
                    )
                })
                .collect::<Vec<_>>(),
        );
        save(&path, &image).unwrap();

        let (cached, bytes) = load(&path).await.unwrap();
        assert_eq!(cached.frame_count(), 2);
        assert_eq!(cached.delay(0).numer_denom_ms(), (50, 1));
        assert_eq!(cached.delay(1).numer_denom_ms(), (100, 1));
        assert_eq!(bytes, 2 * 4);
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn cache_hits_refresh_only_old_mtimes() {
        let directory = temp_directory();
        let path = directory.join("test.img");
        save(&path, &image()).unwrap();
        let old = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();

        assert!(load(&path).await.is_some());
        let refreshed = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(refreshed > old);
        assert!(!touch_due(refreshed, SystemTime::now()));
        assert!(load(&path).await.is_some());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), refreshed);
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn missing_and_corrupt_files_are_cache_misses() {
        let directory = temp_directory();
        let path = directory.join("test.img");
        assert!(load(&path).await.is_none());
        fs::write(&path, b"corrupt").unwrap();

        assert!(load(&path).await.is_none());
        assert!(!path.exists());
        fs::remove_dir(directory).unwrap();
    }
}
