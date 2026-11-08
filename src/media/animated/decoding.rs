use super::cache::{drop_images, schedule_purge, schedule_refresh, start_heartbeat};
use super::*;

pub(super) fn start_next(cache: Rc<RefCell<Cache>>, cx: &App) {
    loop {
        let (key, dropped, evicted) = cache.borrow_mut().next();
        drop_images(evicted, cx);
        if dropped > 0 {
            // Dropped entries may still be visible; repaint so `resolve` re-requests them.
            schedule_refresh(cache.clone(), cx);
        }
        let Some(key) = key else {
            let evicted = {
                let mut state = cache.borrow_mut();
                if state.queue.is_empty() {
                    state.evict()
                } else {
                    Vec::new()
                }
            };
            drop_images(evicted, cx);
            return;
        };
        let lane = Lane::of(&key.0);
        let stamp = {
            let state = cache.borrow();
            state.wanted_stamp(&key)
        };
        let Some(stamp) = stamp else {
            cache.borrow_mut().running[lane as usize] -= 1;
            continue;
        };
        start_heartbeat(cache.clone(), cx);
        let service = images::download_service(cx);
        let request = key.clone();
        let job_stamp = stamp.clone();
        let task = Tokio::spawn_result(cx, async move {
            let service = service.context("download service unavailable")?;
            if !still_wanted(&job_stamp) {
                return Ok(DecodeOutcome::Cancelled);
            }
            let collectible = matches!(&request.0, ImageSpec::Collectible { .. });
            let streaming = stream::selected(&request.0);
            let cached = if matches!(lane, Lane::Gif) && !streaming {
                // A cached animation is as large in memory as a fresh decode, so it waits its
                // turn the same way.
                let _permit = decode_permits(lane)
                    .acquire()
                    .await
                    .context("animation decode permits closed")?;
                if !still_wanted(&job_stamp) {
                    return Ok(DecodeOutcome::Cancelled);
                }
                frames::load(&service.frames_path(&request.0, request.1)).await
            } else {
                None
            };
            if !still_wanted(&job_stamp) {
                return Ok(DecodeOutcome::Cancelled);
            }
            if let Some((image, bytes, plays)) = cached {
                return Ok(DecodeOutcome::Decoded(image, bytes, plays));
            }
            let source_spec = if streaming {
                // Some embeds omit `animated`; never fetch the still-converted cache file.
                stream::source_spec(&request.0)
            } else {
                request.0.clone()
            };
            let path = service
                .load(&source_spec, request.1)
                .await
                .context("animation download failed")?;
            if !still_wanted(&job_stamp) {
                return Ok(DecodeOutcome::Cancelled);
            }
            if streaming {
                let spool = service
                    .frames_path(&request.0, request.1)
                    .with_extension("spool");
                let image = stream::first(path.clone(), request.1, job_stamp.clone()).await?;
                let plays = if collectible {
                    source_play_count(&tokio::fs::read(&path).await?)?
                } else {
                    None
                };
                return Ok(match image {
                    Some(image) if still_wanted(&job_stamp) => {
                        DecodeOutcome::Stream(image, path, spool, plays)
                    }
                    _ => DecodeOutcome::Cancelled,
                });
            }
            let _permit = decode_permits(lane)
                .acquire()
                .await
                .context("animation decode permits closed")?;
            // The tile may have scrolled away while this job waited for a permit.
            if !still_wanted(&job_stamp) {
                return Ok(DecodeOutcome::Cancelled);
            }
            let frames_path = service.frames_path(&request.0, request.1);
            let (image, bytes, plays) = tokio::task::spawn_blocking(move || {
                let source = std::fs::read(path)?;
                let (image, bytes) =
                    decode_with_budget(&source, request.1, MAX_SOURCE_BYTES, MAX_IMAGE)?;
                if matches!(lane, Lane::Gif) {
                    match frames::save(&frames_path, &image, None) {
                        Ok(written) => service.record_frames_write(written),
                        Err(error) => log::debug!(
                            "Could not cache decoded frames at {}: {error}",
                            frames_path.display()
                        ),
                    }
                }
                Ok::<_, anyhow::Error>((image, bytes, None))
            })
            .await
            .context("decode worker failed")??;
            drop(_permit);
            Ok(DecodeOutcome::Decoded(image, bytes, plays))
        });
        let cache = cache.clone();
        cx.spawn(async move |cx| {
            let result = task.await;
            let result = cx.update(|cx| match result {
                Ok(DecodeOutcome::Stream(image, path, spool, plays)) => {
                    if !still_wanted(&stamp) {
                        return Ok(DecodeOutcome::Cancelled);
                    }
                    let bytes =
                        stream::register(&key.0, key.1, path, Some(spool), image.clone(), cx);
                    Ok(DecodeOutcome::Decoded(image, bytes, plays))
                }
                other => other,
            });
            let (success, cancelled) = {
                let mut state = cache.borrow_mut();
                state.running[lane as usize] -= 1;
                match result {
                    Ok(DecodeOutcome::Decoded(image, bytes, plays)) => {
                        state.bytes += bytes;
                        state.clock = state.clock.saturating_add(1);
                        let used = state.clock;
                        let (spec, size) = key;
                        state.entries.entry(spec).or_default().insert(
                            size,
                            Entry::Ready {
                                image,
                                bytes,
                                plays,
                                used,
                                ready_at: Instant::now(),
                                seen: Instant::now(),
                            },
                        );
                        (true, false)
                    }
                    Ok(DecodeOutcome::Cancelled) => {
                        let owned = matches!(
                            state
                                .entries
                                .get(&key.0)
                                .and_then(|sizes| sizes.get(&key.1)),
                            Some(Entry::Pending { wanted, .. }) if Arc::ptr_eq(wanted, &stamp)
                        );
                        if owned {
                            let empty = if let Some(sizes) = state.entries.get_mut(&key.0) {
                                sizes.remove(&key.1);
                                sizes.is_empty()
                            } else {
                                false
                            };
                            if empty {
                                state.entries.remove(&key.0);
                            }
                        }
                        (false, owned)
                    }
                    other => {
                        let reason = match &other {
                            Err(error) => format!("{error:#}"),
                            Ok(_) => "unexpected outcome".to_owned(),
                        };
                        log::warn!(
                            "Animation download or decode failed for {:?} (size {}): {reason}",
                            key.0,
                            key.1
                        );
                        let (spec, size) = key;
                        state
                            .entries
                            .entry(spec)
                            .or_default()
                            .insert(size, Entry::Failed);
                        (false, false)
                    }
                }
            };
            if success || cancelled {
                let cache = cache.clone();
                cx.update(|cx| schedule_refresh(cache, cx));
            }
            if success {
                // It may finish after whoever asked has gone, and never be asked for again.
                let cache = cache.clone();
                cx.update(|cx| schedule_purge(cache, cx));
            }
            cx.update(|cx| start_next(cache, cx));
        })
        .detach();
    }
}

pub(super) fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits
}

pub(super) fn still_frames(decoder: impl ImageDecoder) -> Result<Frames<'static>> {
    let frame = Frame::new(DynamicImage::from_decoder(decoder)?.into_rgba8());
    Ok(Frames::new(Box::new(std::iter::once(Ok(frame)))))
}

pub(super) fn source_play_count(bytes: &[u8]) -> Result<Option<NonZeroU32>> {
    if image::guess_format(bytes)? != ImageFormat::Png {
        return Ok(None);
    }
    let decoder = PngDecoder::with_limits(Cursor::new(bytes), limits())?;
    if !decoder.is_apng()? {
        return Ok(None);
    }
    Ok(match decoder.apng()?.loop_count() {
        LoopCount::Infinite => None,
        LoopCount::Finite(plays) => Some(plays),
    })
}

fn source_frames(bytes: &[u8]) -> Result<((u32, u32), Frames<'_>)> {
    let format = image::guess_format(bytes)?;
    let reader = Cursor::new(bytes);
    match format {
        ImageFormat::Png => apng::frames(reader),
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(reader)?;
            decoder.set_limits(limits())?;
            Ok((decoder.dimensions(), decoder.into_frames()))
        }
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(reader)?;
            decoder.set_limits(limits())?;
            let dimensions = decoder.dimensions();
            if decoder.has_animation() {
                Ok((dimensions, decoder.into_frames()))
            } else {
                Ok((dimensions, still_frames(decoder)?))
            }
        }
        _ => {
            let mut reader = ImageReader::with_format(reader, format);
            reader.limits(limits());
            let decoder = reader.into_decoder()?;
            let dimensions = decoder.dimensions();
            Ok((dimensions, still_frames(decoder)?))
        }
    }
}

pub(super) fn output_size(width: u32, height: u32, size: u32, frames: usize) -> Result<(u32, u32)> {
    fitted_size(width, height, size, frames, MAX_IMAGE)
}

/// The largest size up to `size` whose `frames` fit in `max_image` bytes.
fn fitted_size(
    width: u32,
    height: u32,
    size: u32,
    frames: usize,
    max_image: usize,
) -> Result<(u32, u32)> {
    ensure!(
        width > 0 && height > 0 && width <= MAX_DIMENSION && height <= MAX_DIMENSION,
        "invalid input dimensions"
    );
    ensure!(size > 0, "empty target size");
    ensure!((1..=MAX_FRAMES).contains(&frames), "invalid frame count");
    let longest = width.max(height);
    let dimensions = |side: u32| {
        (
            (u64::from(width) * u64::from(side) / u64::from(longest)).max(1) as u32,
            (u64::from(height) * u64::from(side) / u64::from(longest)).max(1) as u32,
        )
    };
    let mut low = 1;
    let mut high = size.min(longest);
    while low < high {
        let side = low + (high - low).div_ceil(2);
        let (width, height) = dimensions(side);
        let bytes = u64::from(width) * u64::from(height) * 4 * frames as u64;
        if bytes <= max_image as u64 {
            low = side;
        } else {
            high = side - 1;
        }
    }
    Ok(dimensions(low))
}

pub(super) fn valid_dimensions(dimensions: (u32, u32)) -> Result<()> {
    ensure!(
        dimensions.0 > 0
            && dimensions.1 > 0
            && dimensions.0 <= MAX_DIMENSION
            && dimensions.1 <= MAX_DIMENSION,
        "invalid input dimensions"
    );
    Ok(())
}

pub(super) fn process_frame(frame: Frame, target: (u32, u32)) -> Frame {
    let delay = frame.delay();
    let (numerator, denominator) = delay.numer_denom_ms();
    let delay = if u64::from(numerator) < 20 * u64::from(denominator) {
        Delay::from_numer_denom_ms(100, 1)
    } else {
        delay
    };
    let mut pixels = frame.into_buffer();
    let factor = (pixels.width() / target.0).min(pixels.height() / target.1);
    if factor >= 2 {
        pixels = box_shrink(&pixels, factor);
    }
    if pixels.dimensions() != target {
        pixels = resize(&pixels, target.0, target.1, FilterType::Triangle);
    }
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Frame::from_parts(pixels, 0, 0, delay)
}

pub(super) fn box_shrink(pixels: &RgbaImage, factor: u32) -> RgbaImage {
    let factor = factor as usize;
    let source_width = pixels.width() as usize;
    let width = source_width / factor;
    let height = pixels.height() as usize / factor;
    let raw = pixels.as_raw();
    let area = (factor * factor) as u32;
    let mut output = vec![0; width * height * 4];
    let mut sums = vec![[0u32; 4]; width];
    for y in 0..height {
        sums.fill([0; 4]);
        for offset in 0..factor {
            let row = (y * factor + offset) * source_width * 4;
            for (x, sum) in sums.iter_mut().enumerate() {
                let start = row + x * factor * 4;
                for pixel in raw[start..start + factor * 4].as_chunks::<4>().0 {
                    sum[0] += u32::from(pixel[0]);
                    sum[1] += u32::from(pixel[1]);
                    sum[2] += u32::from(pixel[2]);
                    sum[3] += u32::from(pixel[3]);
                }
            }
        }
        for (x, sum) in sums.iter().enumerate() {
            let out = (y * width + x) * 4;
            for channel in 0..4 {
                output[out + channel] = (sum[channel] / area) as u8;
            }
        }
    }
    RgbaImage::from_raw(width as u32, height as u32, output).expect("shrunk dimensions fit")
}

#[cfg(test)]
pub(super) fn decode(bytes: &[u8], size: u32) -> Result<(Arc<RenderImage>, usize)> {
    decode_with_budget(bytes, size, MAX_SOURCE_BYTES, MAX_IMAGE)
}

pub(super) fn decode_with_budget(
    bytes: &[u8],
    size: u32,
    budget: u64,
    max_image: usize,
) -> Result<(Arc<RenderImage>, usize)> {
    let (dimensions, source) = source_frames(bytes)?;
    valid_dimensions(dimensions)?;
    // The target size depends on the real frame count, so hold decoded source frames only
    // until the single pass is done. Above the budget, fall back to counting and resizing
    // in two streaming passes instead of failing.
    let frame_bytes = u64::from(dimensions.0) * u64::from(dimensions.1) * 4;
    let mut source_frames = Vec::new();
    let mut over_budget = false;
    for frame in source {
        ensure!(
            source_frames.len() < MAX_FRAMES,
            "too many animation frames"
        );
        let held = (source_frames.len() as u64 + 1)
            .checked_mul(frame_bytes)
            .context("animation size overflow")?;
        if held > budget {
            over_budget = true;
            break;
        }
        let frame = frame?;
        ensure!(
            frame.buffer().dimensions() == dimensions,
            "frame size changed"
        );
        source_frames.push(frame);
    }
    if over_budget {
        // The decoder went with the loop; free the frames it gave before decoding twice more.
        drop(source_frames);
        return decode_streaming(bytes, size, max_image);
    }
    let count = source_frames.len();
    let target = fitted_size(dimensions.0, dimensions.1, size, count, max_image)?;
    let frames: Vec<_> = source_frames
        .into_iter()
        .map(|frame| process_frame(frame, target))
        .collect();
    let bytes = target.0 as usize * target.1 as usize * 4 * count;
    Ok((Arc::new(RenderImage::new(frames)), bytes))
}

fn decode_streaming(
    bytes: &[u8],
    size: u32,
    max_image: usize,
) -> Result<(Arc<RenderImage>, usize)> {
    let (dimensions, source) = source_frames(bytes)?;
    valid_dimensions(dimensions)?;
    let mut count = 0;
    for frame in source {
        ensure!(count < MAX_FRAMES, "too many animation frames");
        let frame = frame?;
        ensure!(
            frame.buffer().dimensions() == dimensions,
            "frame size changed"
        );
        count += 1;
    }
    let target = fitted_size(dimensions.0, dimensions.1, size, count, max_image)?;
    let (source_dimensions, source) = source_frames(bytes)?;
    ensure!(source_dimensions == dimensions, "frame size changed");
    let mut frames = Vec::with_capacity(count);
    for frame in source {
        ensure!(frames.len() < count, "frame count changed");
        let frame = frame?;
        ensure!(
            frame.buffer().dimensions() == dimensions,
            "frame size changed"
        );
        frames.push(process_frame(frame, target));
    }
    ensure!(frames.len() == count, "frame count changed");
    let bytes = target.0 as usize * target.1 as usize * 4 * count;
    Ok((Arc::new(RenderImage::new(frames)), bytes))
}
