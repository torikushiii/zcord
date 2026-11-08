use super::*;
use super::{decoding::*, element::*, wakeups::*};
use gpui::{
    Context, Entity, Render, StyleRefinement, TestAppContext, VisualTestContext, Window, div,
    prelude::*,
};
use image::{Rgba, RgbaImage};
use png::{BitDepth, BlendOp, ColorType, DisposeOp, Encoder};
use std::cell::Cell;

#[test]
fn fallback_selection_freezes_media_but_only_retries_failed_attachments() {
    for animated in [false, true] {
        let spec = ImageSpec::Media {
            url: "https://media.discordapp.net/test.gif".into(),
            width: 8192,
            height: 4096,
            animated,
        };
        let expected = ImageSpec::Media {
            url: "https://media.discordapp.net/test.gif".into(),
            width: 8192,
            height: 4096,
            animated: false,
        };
        for failed in [false, true] {
            assert_eq!(fallback_spec(&spec, failed), Some(expected.clone()));
        }
    }
    let attachment = ImageSpec::Attachment {
        id: 1,
        url: "https://cdn.discordapp.com/test.gif".into(),
        width: 8192,
        height: 4096,
    };
    assert_eq!(fallback_spec(&attachment, false), None);
    assert_eq!(fallback_spec(&attachment, true), Some(attachment));
    assert_eq!(
        fallback_spec(&ImageSpec::DefaultAvatar { index: 0 }, true),
        None
    );
}

// 4x2 canvas. Frame 0 is a 2x2 opaque red square at the origin and is disposed to
// background. Frame 1 is a 2x2 alpha-blended subframe at x=2, with one blue pixel
// and transparent padding. If disposal is skipped, frame 0's red square remains on
// frame 1.
const DISPOSAL_OFFSET_WEBP: &[u8] = &[
    0x52, 0x49, 0x46, 0x46, 0x86, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x58,
    0x0a, 0x00, 0x00, 0x00, 0x12, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x01, 0x00, 0x00, 0x41, 0x4e,
    0x49, 0x4d, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x41, 0x4e, 0x4d, 0x46,
    0x28, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x32, 0x00, 0x00, 0x03, 0x56, 0x50, 0x38, 0x4c, 0x0f, 0x00, 0x00, 0x00, 0x2f, 0x01, 0x40, 0x00,
    0x00, 0x07, 0x10, 0xfd, 0x8f, 0xfe, 0x07, 0x22, 0xa2, 0xff, 0x01, 0x00, 0x41, 0x4e, 0x4d, 0x46,
    0x2a, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x32, 0x00, 0x00, 0x00, 0x56, 0x50, 0x38, 0x4c, 0x11, 0x00, 0x00, 0x00, 0x2f, 0x01, 0x40, 0x00,
    0x10, 0x0f, 0x10, 0x31, 0xff, 0xf3, 0x1f, 0x8c, 0x09, 0x88, 0xe8, 0x7f, 0x04, 0x00,
];

fn apng(width: u32, height: u32, delays: &[u16], plays: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut bytes, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        encoder.set_animated(delays.len() as u32, plays).unwrap();
        let mut writer = encoder.write_header().unwrap();
        let pixels = RgbaImage::from_pixel(width, height, Rgba([255, 0, 0, 255]));
        for &delay in delays {
            writer.set_frame_delay(delay, 1000).unwrap();
            writer.write_image_data(pixels.as_raw()).unwrap();
        }
        writer.finish().unwrap();
    }
    bytes
}

fn next(cache: &mut Cache) -> Option<Key> {
    let (key, dropped, evicted) = cache.next();
    assert_eq!(dropped, 0, "unexpected stale drop");
    assert!(evicted.is_empty());
    key
}

fn blank_image() -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![Frame::new(RgbaImage::new(1, 1))]))
}

fn delayed_image(delays: &[u64]) -> RenderImage {
    RenderImage::new(
        delays
            .iter()
            .map(|delay| {
                Frame::from_parts(
                    RgbaImage::new(1, 1),
                    0,
                    0,
                    Delay::from_numer_denom_ms(*delay as u32, 1),
                )
            })
            .collect::<Vec<_>>(),
    )
}

#[test]
fn frame_selection_follows_delays_and_wraps() {
    let image = delayed_image(&[50, 100, 120]);
    let ms = Duration::from_millis;
    for (elapsed, frame, remaining) in [
        (0, 0, 50),
        (49, 0, 1),
        (50, 1, 100),
        (149, 1, 1),
        (150, 2, 120),
        (269, 2, 1),
        (270, 0, 50),
        (305, 0, 15),
        (1000, 2, 80),
    ] {
        assert_eq!(
            frame_at(&image, ms(elapsed), None),
            (frame, ms(remaining)),
            "elapsed {elapsed}ms"
        );
    }
    assert_eq!(
        frame_at(&delayed_image(&[100]), ms(999), None),
        (0, Duration::MAX)
    );
    assert_eq!(
        frame_at(&RenderImage::new(Vec::new()), ms(1), None),
        (0, Duration::ZERO)
    );
}

#[test]
fn one_shot_frames_hold_the_last_frame_instead_of_restarting() {
    let frames = Arc::new(delayed_image(&[50, 100, 120]));
    let ms = Duration::from_millis;
    let started_at = Instant::now();
    assert!(image(frames.clone()).started_at.is_none());
    assert_eq!(
        image(frames.clone())
            .play_from(started_at, Some(NonZeroU32::MIN))
            .started_at,
        Some(started_at)
    );
    for (elapsed, frame, remaining) in [
        (0, 0, ms(50)),
        (50, 1, ms(100)),
        (150, 2, ms(120)),
        (269, 2, ms(1)),
        (270, 2, Duration::MAX),
        (305, 2, Duration::MAX),
        (1000, 2, Duration::MAX),
    ] {
        assert_eq!(
            frame_at(&frames, ms(elapsed), Some(NonZeroU32::MIN)),
            (frame, remaining),
            "elapsed {elapsed}ms"
        );
    }
    assert_eq!(
        frame_at(&delayed_image(&[100]), ms(999), Some(NonZeroU32::MIN)),
        (0, Duration::MAX)
    );
    assert_eq!(
        frame_at(&RenderImage::new(Vec::new()), ms(1), Some(NonZeroU32::MIN)),
        (0, Duration::ZERO)
    );
}

#[test]
fn scheduling_waits_for_the_deadline_unless_it_is_imminent() {
    let ms = Duration::from_millis;
    assert_eq!(wake(ms(0), true, false, true), Wake::AnimationFrame);
    assert_eq!(wake(ms(8), true, false, true), Wake::AnimationFrame);
    assert_eq!(wake(ms(9), true, false, true), Wake::Timer(ms(9)));
    assert_eq!(wake(ms(100), true, false, true), Wake::Timer(ms(100)));
    assert_eq!(wake(ms(100), false, false, true), Wake::None);
    assert_eq!(wake(ms(100), true, true, true), Wake::None);
    assert_eq!(wake(ms(100), true, false, false), Wake::None);
    assert_eq!(wake(Duration::MAX, true, false, true), Wake::None);
}

#[test]
fn finite_play_counts_stop_only_after_the_last_play() {
    let image = delayed_image(&[50, 100]);
    let plays = NonZeroU32::new(2);
    let ms = Duration::from_millis;
    for (elapsed, frame, remaining) in [
        (0, 0, ms(50)),
        (50, 1, ms(100)),
        (150, 0, ms(50)),
        (299, 1, ms(1)),
        (300, 1, Duration::MAX),
        (1000, 1, Duration::MAX),
    ] {
        assert_eq!(frame_at(&image, ms(elapsed), plays), (frame, remaining));
    }
    assert_eq!(frame_at(&image, ms(300), None), (0, ms(50)));
}

#[test]
fn one_wakeup_per_view_keeps_the_earliest_deadline() {
    let mut wakeups = Wakeups::default();
    let view = EntityId::from(1);
    let other = EntityId::from(2);
    let base = Instant::now();
    let at = |ms| base + Duration::from_millis(ms);
    assert!(wakeups.claim(view, at(100)));
    assert!(!wakeups.claim(view, at(100)));
    assert!(!wakeups.claim(view, at(200)));
    assert_eq!(wakeups.pending[&view].deadline, at(100));
    assert!(wakeups.claim(view, at(50)));
    assert_eq!(wakeups.pending[&view].deadline, at(50));
    assert!(wakeups.claim(other, at(75)));
    assert_eq!(wakeups.pending[&other].deadline, at(75));
    assert!(!wakeups.release(view, at(100)));
    assert!(wakeups.release(view, at(50)));
    assert!(!wakeups.pending.contains_key(&view));
    assert!(wakeups.pending.contains_key(&other));
    assert!(wakeups.claim(view, at(25)));
    assert_eq!(wakeups.pending[&view].deadline, at(25));
}

fn put_ready(
    cache: &mut Cache,
    key: &Key,
    image: Arc<RenderImage>,
    bytes: usize,
    used: u64,
    ready_at: Instant,
) {
    cache.entries.entry(key.0.clone()).or_default().insert(
        key.1,
        Entry::Ready {
            image,
            bytes,
            plays: None,
            used,
            ready_at,
            seen: ready_at,
        },
    );
}

fn has(cache: &Cache, key: &Key) -> bool {
    cache
        .entries
        .get(&key.0)
        .is_some_and(|sizes| sizes.contains_key(&key.1))
}

fn assert_same_frames(left: &RenderImage, right: &RenderImage) {
    assert_eq!(left.frame_count(), right.frame_count());
    for index in 0..left.frame_count() {
        assert_eq!(left.size(index).width.0, right.size(index).width.0);
        assert_eq!(left.size(index).height.0, right.size(index).height.0);
        assert_eq!(
            left.delay(index).numer_denom_ms(),
            right.delay(index).numer_denom_ms()
        );
        assert_eq!(left.as_bytes(index), right.as_bytes(index));
    }
}

#[test]
fn apng_delays_are_preserved_and_short_delays_clamped() {
    let (image, bytes) = decode(&apng(2, 1, &[50, 0, 120], 0), 2).unwrap();
    assert_eq!(image.frame_count(), 3);
    assert_eq!(bytes, 24);
    assert_eq!(
        (0..3)
            .map(|index| image.delay(index).numer_denom_ms())
            .collect::<Vec<_>>(),
        [(50, 1), (100, 1), (120, 1)]
    );
    let (image, _) = decode(&apng(1, 1, &[19, 20], 0), 1).unwrap();
    assert_eq!(image.delay(0).numer_denom_ms(), (100, 1));
    assert_eq!(image.delay(1).numer_denom_ms(), (20, 1));
}

#[test]
fn collectible_apng_preserves_the_source_play_count() {
    for plays in [0, 1, 2, 17, 0x7FFF_FFFF] {
        let source = apng(1, 1, &[80, 80], plays);
        assert_eq!(source_play_count(&source).unwrap(), NonZeroU32::new(plays));
    }
    // The PNG decoder ignores animation-control values above its 31-bit limit.
    assert_eq!(
        source_play_count(&apng(1, 1, &[80, 80], u32::MAX)).unwrap(),
        None
    );
    assert_eq!(source_play_count(DISPOSAL_OFFSET_WEBP).unwrap(), None);
    assert!(source_play_count(b"not an image").is_err());
}

#[tokio::test]
async fn collectible_play_count_is_restored_alongside_cached_frames() {
    let source = apng(1, 1, &[83; 36], 1);
    let plays = source_play_count(&source).unwrap();
    let (decoded, _) = decode(&source, 1).unwrap();
    let path = std::env::temp_dir().join(format!(
        "zcord-effect-frames-{:016x}.frames",
        rand::random::<u64>()
    ));
    frames::save(&path, &decoded, plays).unwrap();
    drop(source);
    let cached = frames::load(&path).await;
    std::fs::remove_file(&path).unwrap();
    let (cached, _, cached_plays) = cached.unwrap();
    assert_eq!(cached_plays, plays);
    assert_same_frames(&decoded, &cached);
    for (image, plays) in [(decoded, plays), (cached, cached_plays)] {
        assert_eq!(image.frame_count(), 36);
        assert_eq!(frame_at(&image, Duration::from_millis(2880), plays).0, 34);
        for elapsed in [3000, 5760, 8640, 50_000] {
            assert_eq!(
                frame_at(&image, Duration::from_millis(elapsed), plays),
                (35, Duration::MAX)
            );
        }
    }
    let repeated = apng(1, 1, &[80; 36], 0);
    let (image, _) = decode(&repeated, 1).unwrap();
    assert_eq!(
        frame_at(
            &image,
            Duration::from_millis(2880),
            source_play_count(&repeated).unwrap()
        ),
        (0, Duration::from_millis(80))
    );
}

#[gpui::test]
fn collectible_cache_hits_keep_the_play_count(cx: &mut TestAppContext) {
    cx.update(init);
    cx.update(|cx| {
        let spec = ImageSpec::Collectible {
            url: "https://cdn.discordapp.com/assets/content/test-effect".into(),
        };
        let source = apng(1, 1, &[80, 80], 1);
        let (image, bytes) = decode(&source, 64).unwrap();
        let cache = cx.global::<AnimatedCache>().0.clone();
        cache
            .borrow_mut()
            .entries
            .entry(spec.clone())
            .or_default()
            .insert(
                64,
                Entry::Ready {
                    image: image.clone(),
                    bytes,
                    plays: source_play_count(&source).unwrap(),
                    used: 0,
                    ready_at: Instant::now(),
                    seen: Instant::now(),
                },
            );
        assert_eq!(play_count(&spec, 64, cx), Some(NonZeroU32::MIN));
        let Load::Ready(cached) = state(&spec, 64, cx) else {
            panic!("the existing cache entry should be ready");
        };
        assert!(Arc::ptr_eq(&image, &cached));
        assert_eq!(play_count(&spec, 64, cx), Some(NonZeroU32::MIN));
    });
}

#[test]
fn red_rgba_becomes_bgra() {
    let (image, _) = decode(&apng(1, 1, &[50], 0), 1).unwrap();
    assert_eq!(image.as_bytes(0).unwrap(), &[0, 0, 255, 255]);
}

#[test]
fn frames_downscale_without_upscaling() {
    let input = apng(64, 32, &[50, 100], 0);
    for (size, width, height) in [(16, 16, 8), (128, 64, 32)] {
        let (image, bytes) = decode(&input, size).unwrap();
        for index in 0..image.frame_count() {
            assert_eq!(image.size(index).width.0, width);
            assert_eq!(image.size(index).height.0, height);
        }
        assert_eq!(bytes, width as usize * height as usize * 4 * 2);
    }
}

#[test]
fn box_shrink_averages_blocks_and_drops_leftovers() {
    let mut pixels = RgbaImage::new(4, 4);
    for y in 0..4 {
        for x in 0..4 {
            let value = match (x / 2, y / 2) {
                (0, 0) => [0, 0, 0, 0],
                (1, 0) => [10, 11, 12, 13],
                (0, 1) => [20, 21, 22, 23],
                _ => [255, 255, 255, 255],
            };
            pixels.put_pixel(x, y, Rgba(value));
        }
    }
    for (x, y) in [(2, 2), (2, 3), (3, 2)] {
        pixels.put_pixel(x, y, Rgba([0, 0, 0, 0]));
    }
    let shrunk = box_shrink(&pixels, 2);
    assert_eq!(shrunk.dimensions(), (2, 2));
    assert_eq!(
        shrunk.as_raw(),
        &[0, 0, 0, 0, 10, 11, 12, 13, 20, 21, 22, 23, 63, 63, 63, 63]
    );

    let leftovers = RgbaImage::from_fn(5, 5, |x, y| {
        if x < 4 && y < 4 {
            Rgba([1, 2, 3, 4])
        } else {
            Rgba([200, 200, 200, 200])
        }
    });
    let shrunk = box_shrink(&leftovers, 2);
    assert_eq!(shrunk.dimensions(), (2, 2));
    assert_eq!(shrunk.as_raw(), &[1u8, 2, 3, 4].repeat(4));
}

#[test]
fn process_frame_box_shrinks_large_downscales_to_the_exact_target() {
    let frame = Frame::from_parts(
        RgbaImage::from_pixel(64, 32, Rgba([10, 20, 30, 255])),
        0,
        0,
        Delay::from_numer_denom_ms(50, 1),
    );
    let processed = process_frame(frame, (8, 4));
    assert_eq!(processed.buffer().dimensions(), (8, 4));
    assert_eq!(processed.buffer().get_pixel(0, 0).0, [30, 20, 10, 255]);
}

#[test]
fn apng_background_disposal_and_over_blending_are_composited() {
    for (dispose, expected) in [
        (DisposeOp::Background, [0, 0, 0, 0]),
        (DisposeOp::None, [0, 0, 255, 255]),
    ] {
        let mut bytes = Vec::new();
        {
            let mut encoder = Encoder::new(&mut bytes, 3, 1);
            encoder.set_color(ColorType::Rgba);
            encoder.set_depth(BitDepth::Eight);
            encoder.set_animated(2, 0).unwrap();
            encoder.set_frame_delay(50, 1000).unwrap();
            encoder.set_dispose_op(dispose).unwrap();
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&[255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255])
                .unwrap();
            writer.set_dispose_op(DisposeOp::None).unwrap();
            writer.set_blend_op(BlendOp::Over).unwrap();
            writer.set_frame_dimension(2, 1).unwrap();
            writer.set_frame_position(1, 0).unwrap();
            writer
                .write_image_data(&[0, 0, 0, 0, 0, 0, 255, 255])
                .unwrap();
            writer.finish().unwrap();
        }
        let (image, _) = decode(&bytes, 3).unwrap();
        assert_eq!(image.frame_count(), 2);
        let second = image.as_bytes(1).unwrap();
        assert_eq!(&second[..4], &expected);
        assert_eq!(&second[4..8], &expected);
        assert_eq!(&second[8..], &[255, 0, 0, 255]);
    }
}

#[test]
fn apng_previous_disposal_restores_what_background_disposal_cleared() {
    let mut bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        encoder.set_animated(3, 0).unwrap();
        encoder.set_frame_delay(50, 1000).unwrap();
        encoder.set_dispose_op(DisposeOp::Background).unwrap();
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 255]).unwrap();
        writer.set_dispose_op(DisposeOp::Previous).unwrap();
        writer.write_image_data(&[0, 255, 0, 255]).unwrap();
        writer.set_dispose_op(DisposeOp::None).unwrap();
        writer.set_blend_op(BlendOp::Over).unwrap();
        writer.write_image_data(&[0, 0, 0, 0]).unwrap();
        writer.finish().unwrap();
    }
    let (image, _) = decode(&bytes, 3).unwrap();
    assert_eq!(image.frame_count(), 3);
    assert_eq!(image.as_bytes(1).unwrap(), &[0, 255, 0, 255]);
    assert_eq!(image.as_bytes(2).unwrap(), &[0, 0, 0, 0]);
}

#[test]
fn webp_disposes_first_frame_before_offset_alpha_blend() {
    let (image, bytes) = decode(DISPOSAL_OFFSET_WEBP, 16).unwrap();
    assert_eq!(image.frame_count(), 2);
    assert_eq!(bytes, 4 * 2 * 4 * 2);
    assert_eq!(image.delay(0).numer_denom_ms(), (50, 1));
    assert_eq!(image.delay(1).numer_denom_ms(), (50, 1));

    // Opaque red becomes BGRA after process_frame.
    assert_eq!(&image.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);

    let second = image.as_bytes(1).unwrap();
    assert_eq!(&second[..8], &[0u8; 8]);
    // image-webp's fixed-point blend yields 254 for an opaque pixel over a
    // transparent one.
    let blue = &second[8..12];
    assert!(blue[0] >= 254, "blue channel {}", blue[0]);
    assert_eq!(&blue[1..], &[0, 0, 255]);
    assert_eq!(&second[12..16], &[0u8; 4]);
    assert_eq!(&second[16..], &[0u8; 16]);

    let (streamed, streamed_bytes) =
        decode_with_budget(DISPOSAL_OFFSET_WEBP, 16, 0, MAX_IMAGE).unwrap();
    assert_eq!(bytes, streamed_bytes);
    assert_same_frames(&image, &streamed);
}

#[test]
fn plain_png_has_one_frame() {
    let mut bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255, 0, 0, 255]).unwrap();
        writer.finish().unwrap();
    }
    assert_eq!(source_play_count(&bytes).unwrap(), None);
    let (image, bytes) = decode(&bytes, 16).unwrap();
    assert_eq!(image.frame_count(), 1);
    assert_eq!(image.as_bytes(0).unwrap(), &[0, 0, 255, 255]);
    assert_eq!(bytes, 4);
}

#[test]
fn gif_keeps_all_frames_and_delays() {
    let mut bytes = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        for (pixel, delay) in [([255, 0, 0, 255], 50), ([0, 0, 255, 255], 10)] {
            encoder
                .encode_frame(Frame::from_parts(
                    RgbaImage::from_pixel(2, 1, Rgba(pixel)),
                    0,
                    0,
                    Delay::from_numer_denom_ms(delay, 1),
                ))
                .unwrap();
        }
    }
    let (image, bytes) = decode(&bytes, 1).unwrap();
    assert_eq!(image.frame_count(), 2);
    assert_eq!(image.as_bytes(0).unwrap(), &[0, 0, 255, 255]);
    assert_eq!(image.as_bytes(1).unwrap(), &[255, 0, 0, 255]);
    assert_eq!(image.delay(0).numer_denom_ms(), (50, 1));
    assert_eq!(image.delay(1).numer_denom_ms(), (100, 1));
    assert_eq!(bytes, 8);
}

#[test]
fn static_webp_and_jpeg_decode() {
    for format in [ImageFormat::WebP, ImageFormat::Jpeg] {
        let mut bytes = Cursor::new(Vec::new());
        let pixels = image::RgbImage::from_pixel(2, 1, image::Rgb([255, 0, 0]));
        DynamicImage::ImageRgb8(pixels)
            .write_to(&mut bytes, format)
            .unwrap();
        let (image, bytes) = decode(bytes.get_ref(), 1).unwrap();
        assert_eq!(image.frame_count(), 1);
        assert_eq!(image.size(0).width.0, 1);
        assert_eq!(image.size(0).height.0, 1);
        assert_eq!(image.delay(0).numer_denom_ms(), (100, 1));
        let pixel = image.as_bytes(0).unwrap();
        assert!(pixel[0] < 5 && pixel[1] < 5 && pixel[2] > 250);
        assert_eq!(pixel[3], 255);
        assert_eq!(bytes, 4);
    }
}

#[test]
fn image_budget_shrinks_size_without_dropping_frames() {
    let (width, height) = output_size(2048, 1024, 2048, 600).unwrap();
    assert!(width < 2048);
    assert_eq!(height, width / 2);
    assert!(width as usize * height as usize * 4 * 600 <= MAX_IMAGE);
    assert!((width + 1) as usize * width.div_ceil(2) as usize * 4 * 600 > MAX_IMAGE);
    assert_eq!(output_size(64, 32, 128, 600).unwrap(), (64, 32));
}

#[test]
fn invalid_dimensions_counts_and_garbage_are_rejected() {
    for bytes in [b"".as_slice(), b"garbage", b"\x89PNG\r\n\x1a\n"] {
        assert!(decode(bytes, 16).is_err());
    }
    assert!(output_size(4097, 1, 16, 1).is_err());
    assert!(output_size(1, 4097, 16, 1).is_err());
    assert!(output_size(0, 1, 16, 1).is_err());
    assert!(output_size(1, 1, 0, 1).is_err());
    assert!(output_size(1, 1, 16, 0).is_err());
    assert!(output_size(1, 1, 16, 601).is_err());
    assert!(decode(&apng(1, 1, &vec![50; 601], 0), 1).is_err());
    assert_eq!(
        decode(&apng(1, 1, &vec![50; 600], 0), 1)
            .unwrap()
            .0
            .frame_count(),
        600
    );
    assert!(decode(&apng(4097, 1, &[50], 0), 16).is_err());
}

#[test]
fn decode_falls_back_to_streaming_with_identical_output() {
    let input = apng(64, 32, &[50, 100, 120], 0);
    let (single, single_bytes) = decode_with_budget(&input, 16, u64::MAX, MAX_IMAGE).unwrap();
    // Two 64x32 frames need 16KB, so one frame fits and the second triggers fallback.
    let budget = u64::from(64_u32 * 32 * 4);
    let (streamed, streamed_bytes) = decode_with_budget(&input, 16, budget, MAX_IMAGE).unwrap();
    assert_eq!(single_bytes, streamed_bytes);
    assert_same_frames(&single, &streamed);
}

#[test]
fn streaming_fallback_keeps_frame_and_dimension_limits() {
    assert!(decode_with_budget(&apng(1, 1, &vec![50; 601], 0), 1, 0, MAX_IMAGE).is_err());
    assert_eq!(
        decode_with_budget(&apng(1, 1, &vec![50; 600], 0), 1, 0, MAX_IMAGE)
            .unwrap()
            .0
            .frame_count(),
        600
    );
    assert!(decode_with_budget(&apng(4097, 1, &[50], 0), 16, 0, MAX_IMAGE).is_err());
}

#[test]
fn cache_uses_all_workers_and_evicts_only_old_unheld_lru_entries() {
    let mut cache = Cache::default();
    let now = Instant::now();
    for index in 0..(DOWNLOADS as u8 + 1) {
        cache.request(&ImageSpec::DefaultAvatar { index }, 16, now);
    }
    for _ in 0..DOWNLOADS {
        assert!(next(&mut cache).is_some());
    }
    assert!(next(&mut cache).is_none());
    assert_eq!(cache.running[Lane::Gif as usize], DOWNLOADS);

    // At max-1 workers a non-empty cache must still admit the last worker.
    cache.running[Lane::Gif as usize] = DOWNLOADS - 1;
    let mb = 1024 * 1024;
    let stale = now - VISIBLE_GRACE - Duration::from_secs(1);
    let held = blank_image();
    let held_key = (ImageSpec::DefaultAvatar { index: 10 }, 16);
    put_ready(&mut cache, &held_key, held.clone(), 64 * mb, 0, stale);
    let older_key = (ImageSpec::DefaultAvatar { index: 11 }, 16);
    put_ready(&mut cache, &older_key, blank_image(), 96 * mb, 1, stale);
    let fresh_key = (ImageSpec::DefaultAvatar { index: 12 }, 16);
    put_ready(
        &mut cache,
        &fresh_key,
        blank_image(),
        80 * mb,
        2,
        Instant::now(),
    );
    cache.bytes = 240 * mb;
    cache.request(&ImageSpec::DefaultAvatar { index: 13 }, 16, now);
    let (key, dropped, evicted) = cache.next();
    assert!(key.is_some());
    assert_eq!(dropped, 0);
    assert_eq!(evicted.len(), 1);
    assert_eq!(cache.running[Lane::Gif as usize], DOWNLOADS);
    assert!(!has(&cache, &older_key));
    assert!(has(&cache, &fresh_key));
    assert!(has(&cache, &held_key));
    assert!(cache.bytes + MAX_IMAGE <= MAX_TOTAL);
}

#[test]
fn cache_starts_without_evicting_fresh_or_held_entries() {
    let mut cache = Cache::default();
    let now = Instant::now();
    let mb = 1024 * 1024;
    let held = blank_image();
    let held_key = (ImageSpec::DefaultAvatar { index: 20 }, 16);
    put_ready(
        &mut cache,
        &held_key,
        held.clone(),
        160 * mb,
        0,
        now - VISIBLE_GRACE - Duration::from_secs(1),
    );
    let fresh_key = (ImageSpec::DefaultAvatar { index: 21 }, 16);
    put_ready(
        &mut cache,
        &fresh_key,
        blank_image(),
        96 * mb,
        1,
        Instant::now(),
    );
    cache.bytes = 256 * mb;
    cache.request(&ImageSpec::DefaultAvatar { index: 22 }, 16, now);
    let (key, dropped, evicted) = cache.next();
    assert!(key.is_some());
    assert_eq!(dropped, 0);
    assert!(evicted.is_empty());
    assert_eq!(cache.running[Lane::Gif as usize], 1);
    assert!(has(&cache, &held_key));
    assert!(has(&cache, &fresh_key));
    assert_eq!(cache.bytes, 256 * mb);

    // Once released, the old held entry is evictable again.
    drop(held);
    cache.request(&ImageSpec::DefaultAvatar { index: 23 }, 16, now);
    let (key, _, evicted) = cache.next();
    assert!(key.is_some());
    assert_eq!(evicted.len(), 1);
    assert!(!has(&cache, &held_key));
    assert!(has(&cache, &fresh_key));
}

#[test]
fn eviction_reports_the_image_for_dropping() {
    let mut cache = Cache::default();
    let now = Instant::now();
    let mb = 1024 * 1024;
    let fresh_key = (ImageSpec::DefaultAvatar { index: 30 }, 16);
    put_ready(
        &mut cache,
        &fresh_key,
        blank_image(),
        96 * mb,
        0,
        Instant::now(),
    );
    let evicted_key = (ImageSpec::DefaultAvatar { index: 31 }, 16);
    let image = blank_image();
    put_ready(
        &mut cache,
        &evicted_key,
        image.clone(),
        160 * mb,
        1,
        now - VISIBLE_GRACE - Duration::from_secs(1),
    );
    let image_ptr = Arc::as_ptr(&image);
    drop(image);
    cache.bytes = 256 * mb;
    cache.request(&ImageSpec::DefaultAvatar { index: 32 }, 16, now);
    let (key, dropped, evicted) = cache.next();
    assert!(key.is_some());
    assert_eq!(dropped, 0);
    assert!(!has(&cache, &evicted_key));
    assert!(has(&cache, &fresh_key));
    assert_eq!(evicted.len(), 1);
    assert_eq!(Arc::as_ptr(&evicted[0]), image_ptr);
    assert_eq!(Arc::strong_count(&evicted[0]), 1);
}

#[test]
fn effects_share_a_larger_ready_decode_and_unseen_animations_go() {
    let mut cache = Cache::default();
    let effect = ImageSpec::Collectible {
        url: "https://cdn.discordapp.com/assets/content/effect".into(),
    };
    let old = Instant::now() - UNSEEN_DROP - Duration::from_secs(1);
    assert_eq!(cache.serving_size(&effect, 600), 600);
    // The stream player holds a registered effect's still image too.
    let registered = blank_image();
    put_ready(
        &mut cache,
        &(effect.clone(), 880),
        registered.clone(),
        100,
        0,
        old,
    );
    cache.bytes = 100;
    assert_eq!(cache.serving_size(&effect, 600), 880);
    assert_eq!(cache.serving_size(&effect, 1200), 1200);
    let avatar = ImageSpec::DefaultAvatar { index: 1 };
    put_ready(
        &mut cache,
        &(avatar.clone(), 880),
        blank_image(),
        10,
        1,
        old,
    );
    assert_eq!(cache.serving_size(&avatar, 600), 600);
    let shown = ImageSpec::DefaultAvatar { index: 2 };
    put_ready(&mut cache, &(shown.clone(), 880), blank_image(), 1, 2, old);
    if let Some(Entry::Ready { seen, .. }) = cache
        .entries
        .get_mut(&shown)
        .and_then(|sizes| sizes.get_mut(&880))
    {
        *seen = Instant::now();
    }
    cache.bytes = 111;

    let (purged, remaining) = cache.purge_unseen();
    assert_eq!(purged.len(), 2);
    assert!(remaining);
    assert!(!has(&cache, &(effect, 880)));
    assert!(!has(&cache, &(avatar, 880)));
    assert!(has(&cache, &(shown, 880)));
    assert_eq!(cache.bytes, 1);
}

#[test]
fn visible_entries_survive_eviction_until_grace_expires() {
    let mut cache = Cache::default();
    let now = Instant::now();
    let mb = 1024 * 1024;
    let old = now - VISIBLE_GRACE - Duration::from_secs(1);
    let visible_key = (ImageSpec::DefaultAvatar { index: 40 }, 16);
    put_ready(&mut cache, &visible_key, blank_image(), 120 * mb, 0, old);
    if let Some(Entry::Ready { seen, .. }) = cache
        .entries
        .get_mut(&visible_key.0)
        .and_then(|sizes| sizes.get_mut(&visible_key.1))
    {
        *seen = Instant::now();
    }
    let unseen_key = (ImageSpec::DefaultAvatar { index: 41 }, 16);
    put_ready(&mut cache, &unseen_key, blank_image(), 120 * mb, 1, old);
    cache.bytes = 240 * mb;
    cache.request(&ImageSpec::DefaultAvatar { index: 42 }, 16, now);
    let (key, _, evicted) = cache.next();
    assert!(key.is_some());
    assert_eq!(evicted.len(), 1);
    assert!(has(&cache, &visible_key));
    assert!(!has(&cache, &unseen_key));

    if let Some(Entry::Ready { seen, .. }) = cache
        .entries
        .get_mut(&visible_key.0)
        .and_then(|sizes| sizes.get_mut(&visible_key.1))
    {
        *seen = old;
    }
    let filler_key = (ImageSpec::DefaultAvatar { index: 43 }, 16);
    put_ready(&mut cache, &filler_key, blank_image(), 120 * mb, 2, old);
    cache.bytes = 240 * mb;
    cache.request(&ImageSpec::DefaultAvatar { index: 44 }, 16, now);
    let (key, _, evicted) = cache.next();
    assert!(key.is_some());
    assert_eq!(evicted.len(), 1);
    assert!(!has(&cache, &visible_key));
    assert!(has(&cache, &filler_key));
}

#[test]
fn newest_request_is_served_first_and_re_request_moves_to_front() {
    let mut cache = Cache::default();
    let now = Instant::now();
    let keys: Vec<_> = (0..3)
        .map(|index| (ImageSpec::DefaultAvatar { index }, 16))
        .collect();
    for key in &keys {
        cache.request(&key.0, key.1, now);
    }
    assert_eq!(next(&mut cache), Some(keys[2].clone()));
    assert_eq!(next(&mut cache), Some(keys[1].clone()));
    cache.running[Lane::Gif as usize] = 0;
    assert_eq!(next(&mut cache), Some(keys[0].clone()));

    let mut cache = Cache::default();
    for key in &keys {
        cache.request(&key.0, key.1, now);
    }
    cache.request(&keys[0].0, keys[0].1, now);
    assert_eq!(next(&mut cache), Some(keys[0].clone()));
}

#[test]
fn stale_requests_are_dropped_without_becoming_failures() {
    let mut cache = Cache::default();
    let now = Instant::now();
    let stale = (ImageSpec::DefaultAvatar { index: 1 }, 16);
    let fresh = (ImageSpec::DefaultAvatar { index: 2 }, 16);
    cache.request(
        &stale.0,
        stale.1,
        now - STALE_AFTER - Duration::from_millis(1),
    );
    cache.request(&fresh.0, fresh.1, now);
    let (key, dropped, evicted) = cache.next();
    assert_eq!(key, Some(fresh));
    assert_eq!(dropped, 0);
    assert!(evicted.is_empty());
    cache.running[Lane::Gif as usize] = 0;
    let (key, dropped, evicted) = cache.next();
    assert_eq!(key, None);
    assert_eq!(dropped, 1);
    assert!(evicted.is_empty());
    assert!(!has(&cache, &stale));
    assert!(!matches!(
        cache
            .entries
            .get(&stale.0)
            .and_then(|sizes| sizes.get(&stale.1)),
        Some(Entry::Failed)
    ));

    let mut cache = Cache::default();
    cache.request(
        &stale.0,
        stale.1,
        now - STALE_AFTER - Duration::from_millis(1),
    );
    let (key, dropped, evicted) = cache.next();
    assert_eq!(key, None);
    assert_eq!(dropped, 1);
    assert!(evicted.is_empty());
    assert!(!has(&cache, &stale));
    assert!(!matches!(
        cache
            .entries
            .get(&stale.0)
            .and_then(|sizes| sizes.get(&stale.1)),
        Some(Entry::Failed)
    ));
}

#[test]
fn requests_refresh_a_running_jobs_wanted_stamp() {
    let mut cache = Cache::default();
    let now = Instant::now();
    let key = (ImageSpec::DefaultAvatar { index: 1 }, 16);
    cache.request(&key.0, key.1, now);
    assert_eq!(next(&mut cache), Some(key.clone()));
    let stamp = cache.wanted_stamp(&key).unwrap();
    *stamp.lock().unwrap() = now - STALE_AFTER - Duration::from_millis(1);
    cache.request(&key.0, key.1, now);
    assert_eq!(*stamp.lock().unwrap(), now);
}

#[test]
fn wanted_jobs_expire_after_the_stale_window() {
    let now = Instant::now();
    let old = now - STALE_AFTER - Duration::from_millis(1);
    let fresh = now - STALE_AFTER + Duration::from_millis(1);
    assert!(wanted(now, now));
    assert!(wanted(fresh, now));
    assert!(wanted(now - STALE_AFTER, now));
    assert!(!wanted(old, now));
}

#[test]
fn queued_small_images_are_served_while_the_gif_lane_is_full() {
    let mut cache = Cache::default();
    let now = Instant::now();
    for index in 0..DOWNLOADS as u8 {
        cache.request(&ImageSpec::DefaultAvatar { index }, 16, now);
    }
    for _ in 0..DOWNLOADS {
        assert!(next(&mut cache).is_some());
    }
    assert_eq!(cache.running[Lane::Gif as usize], DOWNLOADS);
    assert!(next(&mut cache).is_none());

    let emoji = (
        ImageSpec::Emoji {
            id: 1,
            animated: true,
        },
        32,
    );
    cache.request(&emoji.0, emoji.1, now);
    // A fresh GIF request is ahead in the queue; its full lane must not block the emoji.
    cache.request(&ImageSpec::DefaultAvatar { index: 100 }, 16, now);
    assert_eq!(next(&mut cache), Some(emoji));
    assert_eq!(cache.running[Lane::Small as usize], 1);
    assert_eq!(cache.running[Lane::Gif as usize], DOWNLOADS);
}

#[test]
fn heartbeat_runs_until_both_lanes_and_the_queue_are_idle() {
    let mut cache = Cache::default();
    assert!(!cache.needs_heartbeat());

    let key = (
        ImageSpec::Emoji {
            id: 2,
            animated: true,
        },
        32,
    );
    cache.request(&key.0, key.1, Instant::now());
    assert!(cache.needs_heartbeat());
    assert_eq!(next(&mut cache), Some(key));
    assert!(cache.needs_heartbeat());

    cache.running[Lane::Small as usize] = 0;
    assert!(!cache.needs_heartbeat());
    cache.running[Lane::Gif as usize] = 1;
    assert!(cache.needs_heartbeat());
    cache.running[Lane::Gif as usize] = 0;
    assert!(!cache.needs_heartbeat());
}

struct AnimationCounter {
    image: Arc<RenderImage>,
    renders: Rc<Cell<usize>>,
}

impl Render for AnimationCounter {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div()
            .size(px(32.))
            .child(image(self.image.clone()).size(px(32.)).id("test-animation"))
    }
}

struct CanvasPlayback {
    image: Arc<RenderImage>,
    frame: Rc<Cell<usize>>,
}

impl Render for CanvasPlayback {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let image = self.image.clone();
        let frame = self.frame.clone();
        gpui::canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let index = window.with_global_id("canvas-playback".into(), |id, window| {
                    frame_index(&image, id, window, cx)
                });
                frame.set(index);
                window
                    .paint_image(bounds, gpui::Corners::default(), image, index, false)
                    .ok();
            },
        )
        .size(px(32.))
    }
}

#[gpui::test]
fn canvas_playback_advances_wraps_and_resets_when_its_image_changes(cx: &mut TestAppContext) {
    cx.update(init);
    FORCE_ACTIVE.set(true);
    let clock = cx.clone();
    let frame = Rc::new(Cell::new(usize::MAX));
    let (view, cx) = cx.add_window_view(|_, _| CanvasPlayback {
        image: Arc::new(delayed_image(&[50, 100])),
        frame: frame.clone(),
    });
    for (elapsed, expected) in [(0, 0), (50, 1), (99, 1), (1, 0), (50, 1)] {
        clock
            .background_executor
            .advance_clock(Duration::from_millis(elapsed));
        view.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(frame.get(), expected);
    }
    view.update(cx, |view, cx| {
        view.image = blank_image();
        cx.notify();
    });
    for elapsed in [0, 500] {
        clock
            .background_executor
            .advance_clock(Duration::from_millis(elapsed));
        view.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(frame.get(), 0);
    }
    FORCE_ACTIVE.set(false);
}

struct StaticPanel {
    renders: Rc<Cell<usize>>,
}

struct ClickableAnimation {
    image: Arc<RenderImage>,
    clicks: Rc<Cell<usize>>,
}

impl Render for ClickableAnimation {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        div()
            .id("animated-media-target")
            .size(px(32.))
            .on_click(move |_, _, _| clicks.set(clicks.get() + 1))
            .child(
                image(self.image.clone())
                    .size(px(32.))
                    .id("clickable-animation"),
            )
    }
}

#[gpui::test]
fn animated_media_keeps_its_parent_clickable_after_repainting(cx: &mut TestAppContext) {
    cx.update(init);
    let clicks = Rc::new(Cell::new(0));
    let clock = cx.clone();
    let (view, cx) = cx.add_window_view(|_, _| ClickableAnimation {
        image: Arc::new(delayed_image(&[100, 100])),
        clicks: clicks.clone(),
    });
    for expected in 1..=3 {
        clock
            .background_executor
            .advance_clock(Duration::from_millis(100));
        view.update(cx, |_, cx| cx.notify());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_click(gpui::point(px(16.), px(16.)), gpui::Modifiers::none());
        assert_eq!(clicks.get(), expected);
    }
    let position = gpui::point(px(16.), px(16.));
    cx.simulate_mouse_down(position, gpui::MouseButton::Left, gpui::Modifiers::none());
    clock
        .background_executor
        .advance_clock(Duration::from_millis(100));
    view.update(cx, |_, cx| cx.notify());
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(position, gpui::MouseButton::Left, gpui::Modifiers::none());
    assert_eq!(clicks.get(), 4);
}

impl Render for StaticPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().size(px(32.))
    }
}

struct AnimationHost {
    panel: Entity<AnimationCounter>,
    sibling: Entity<StaticPanel>,
}

impl Render for AnimationHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(
                self.panel
                    .clone()
                    .cached(StyleRefinement::default().size(px(32.))),
            )
            .child(
                self.sibling
                    .clone()
                    .cached(StyleRefinement::default().size(px(32.))),
            )
    }
}

#[gpui::test]
fn hundred_millisecond_frames_render_about_ten_times_per_second(cx: &mut TestAppContext) {
    cx.update(init);
    let image = Arc::new(delayed_image(&[100, 100]));
    let renders = Rc::new(Cell::new(0));
    let counted = renders.clone();
    let sibling_renders = Rc::new(Cell::new(0));
    let counted_sibling = sibling_renders.clone();
    FORCE_ACTIVE.set(true);
    let clock = cx.clone();
    let (_host, cx) = cx.add_window_view(move |_, cx| {
        let panel = cx.new(|_| AnimationCounter {
            image,
            renders: counted,
        });
        let sibling = cx.new(|_| StaticPanel {
            renders: counted_sibling,
        });
        AnimationHost { panel, sibling }
    });
    let draw = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    };
    draw(cx);
    let initial = renders.get();
    let sibling_initial = sibling_renders.get();
    assert_eq!(initial, 1);

    // 165 draws per simulated second; only the animation's own wakeups may
    // dirty the cached panel.
    let step = Duration::from_nanos(1_000_000_000 / 165);
    for _ in 0..165 {
        clock.background_executor.advance_clock(step);
        draw(cx);
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
        });
    }
    let counted = renders.get() - initial;
    println!(
        "100ms animation renders per simulated second: {counted}; \
         sibling panel renders: {}",
        sibling_renders.get() - sibling_initial
    );
    assert_eq!(sibling_renders.get(), sibling_initial);
    assert!(
        (8..=12).contains(&counted),
        "a 100ms animation should repaint its view ~10 times per second, got {counted}"
    );
    FORCE_ACTIVE.set(false);
}
