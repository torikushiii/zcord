use super::*;
use image::{Rgba, RgbaImage};
use std::time::Duration;

fn decode_bytes(bytes: &[u8]) -> anyhow::Result<(Arc<RenderImage>, usize, Option<NonZeroU32>)> {
    decode(&mut &bytes[..], bytes.len() as u64)
}

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
    assert_eq!(
        write(&image, None, &mut encoded).unwrap(),
        encoded.len() as u64
    );
    let (decoded, bytes, plays) = decode_bytes(&encoded).unwrap();

    assert_eq!(decoded.frame_count(), 3);
    assert_eq!(bytes, 3 * 2 * 4 * 3);
    assert_eq!(plays, None);
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
fn play_count_round_trips() {
    for plays in [
        None,
        NonZeroU32::new(1),
        NonZeroU32::new(17),
        NonZeroU32::new(u32::MAX),
    ] {
        let mut encoded = Vec::new();
        write(&image(), plays, &mut encoded).unwrap();
        assert_eq!(decode_bytes(&encoded).unwrap().2, plays);
    }
}

#[tokio::test]
async fn version_one_is_rejected_and_removed() {
    let mut encoded = Vec::new();
    write(&image(), None, &mut encoded).unwrap();
    encoded[4] = 1;
    encoded.drain(17..21);
    assert!(decode_bytes(&encoded).is_err());

    let directory = temp_directory();
    let path = directory.join("version-one.img");
    fs::write(&path, encoded).unwrap();
    assert!(load(&path).await.is_none());
    assert!(!path.exists());
    fs::remove_dir(directory).unwrap();
}

#[test]
fn truncated_file_is_an_error() {
    let mut encoded = Vec::new();
    write(&image(), None, &mut encoded).unwrap();
    for length in [0, 4, HEADER_LEN, encoded.len() - 1] {
        assert!(decode_bytes(&encoded[..length]).is_err(), "length {length}");
    }
}

#[test]
fn wrong_magic_is_an_error() {
    let mut encoded = Vec::new();
    write(&image(), None, &mut encoded).unwrap();
    encoded[0] = b'X';
    assert!(decode_bytes(&encoded).is_err());
}

#[test]
fn saved_file_matches_streamed_format() {
    let directory = temp_directory();
    let path = directory.join("test.img");
    let image = image();
    let mut encoded = Vec::new();
    write(&image, None, &mut encoded).unwrap();

    assert_eq!(save(&path, &image, None).unwrap(), encoded.len() as u64);
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

    assert_eq!(write(&image, None, &mut encoded).unwrap(), 0);
    assert!(encoded.is_empty());
    assert_eq!(save(&path, &image, None).unwrap(), 0);
    assert!(!path.exists());
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
    fs::remove_dir(directory).unwrap();
}

#[test]
fn write_propagates_errors() {
    let mut buffer = [0; HEADER_LEN];
    let mut out = io::Cursor::new(buffer.as_mut_slice());
    assert_eq!(
        write(&image(), None, &mut out).unwrap_err().kind(),
        io::ErrorKind::WriteZero
    );
}

#[test]
fn failed_save_removes_temp_file() {
    let directory = temp_directory();
    let path = directory.join("test.img");
    fs::create_dir(&path).unwrap();

    assert!(save(&path, &image(), None).is_err());
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
    save(&path, &image, None).unwrap();

    let (cached, bytes, plays) = load(&path).await.unwrap();
    assert_eq!(cached.frame_count(), 2);
    assert_eq!(cached.delay(0).numer_denom_ms(), (50, 1));
    assert_eq!(cached.delay(1).numer_denom_ms(), (100, 1));
    assert_eq!(bytes, 2 * 4);
    assert_eq!(plays, None);
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn cache_hits_refresh_only_old_mtimes() {
    let directory = temp_directory();
    let path = directory.join("test.img");
    save(&path, &image(), None).unwrap();
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
