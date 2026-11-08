use super::sources::{FrameSource, ImageFrames, SourceKind, SourceRead, SpoolFrames};
use super::*;

const CATCH_UP_BUDGET: Duration = Duration::from_millis(100);

pub(super) struct Core {
    source: Box<dyn FrameSource>,
    pub(super) count: usize,
    pub(super) position: Duration,
    pub(super) base: Duration,
    /// How many times to play before holding the last frame; `None` loops forever.
    plays: Option<NonZeroU32>,
    cycles: u64,
    pub(super) terminal: bool,
    pub(super) spool: Option<PathBuf>,
    spool_requested: bool,
    pub(super) spent: Duration,
    #[cfg(test)]
    pub(super) decoded: usize,
}

pub(super) fn source(path: &Path) -> Result<((u32, u32), Frames<'static>)> {
    let file = File::open(path)?;
    ensure!(
        file.metadata()?.len() <= MAX_SOURCE_BYTES,
        "animation source too large"
    );
    let reader = ImageReader::new(BufReader::new(file)).with_guessed_format()?;
    let format = reader.format().context("unknown animation format")?;
    let reader = reader.into_inner();
    let (dimensions, frames) = match format {
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(reader)?;
            decoder.set_limits(limits())?;
            (decoder.dimensions(), decoder.into_frames())
        }
        ImageFormat::Png => apng::frames(reader)?,
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(reader)?;
            decoder.set_limits(limits())?;
            let dimensions = decoder.dimensions();
            let frames = if decoder.has_animation() {
                decoder.into_frames()
            } else {
                still_frames(decoder)?
            };
            (dimensions, frames)
        }
        _ => {
            let mut reader = ImageReader::with_format(reader, format);
            reader.limits(limits());
            let decoder = reader.into_decoder()?;
            (decoder.dimensions(), still_frames(decoder)?)
        }
    };
    valid_dimensions(dimensions)?;
    Ok((dimensions, frames))
}

impl Core {
    pub(super) fn open(path: PathBuf, size: u32) -> Result<Self> {
        let (dimensions, frames) = source(&path)?;
        let target = output_size(dimensions.0, dimensions.1, size, 1 + PLAYBACK_FRAMES)?;
        Ok(Self::from_source(Box::new(ImageFrames {
            path,
            frames,
            dimensions,
            target,
        })))
    }

    pub(super) fn open_kind(
        path: PathBuf,
        size: u32,
        kind: SourceKind,
        spool: Option<PathBuf>,
    ) -> Result<Self> {
        match kind {
            SourceKind::Image => {
                let mut core = Self::open(path, size)?;
                let Some(file) = spool else {
                    return Ok(core);
                };
                if file.exists()
                    && let Some(target) = core.source.target()
                {
                    match SpoolFrames::open(&file, target) {
                        Ok(source) => return Ok(Self::from_source(Box::new(source))),
                        Err(_) => {
                            let _ = std::fs::remove_file(&file);
                        }
                    }
                }
                core.spool = Some(file);
                Ok(core)
            }
            #[cfg(windows)]
            SourceKind::Video { max_fps } => {
                let reader = crate::media::video::Reader::open(&path, size)?;
                let rate = reader.rate();
                Ok(Self::from_source(Box::new(
                    crate::media::video::animation::VideoFrames::new(reader, rate, max_fps),
                )))
            }
            #[cfg(test)]
            SourceKind::Fake(open) => Ok(Self::from_source(open()?)),
        }
    }

    fn from_source(source: Box<dyn FrameSource>) -> Self {
        Self {
            source,
            count: 0,
            position: Duration::ZERO,
            base: Duration::ZERO,
            plays: None,
            cycles: 0,
            terminal: false,
            spool: None,
            spool_requested: false,
            spent: Duration::ZERO,
            #[cfg(test)]
            decoded: 0,
        }
    }

    pub(super) fn with_plays(mut self, plays: Option<NonZeroU32>) -> Self {
        self.plays = plays;
        self
    }

    pub(super) fn wants_spool(&mut self) -> Option<PathBuf> {
        if self.spool_requested || self.count < 4 || self.spent <= self.position {
            return None;
        }
        let file = self.spool.clone()?;
        self.spool_requested = true;
        Some(file)
    }

    pub(super) fn step(
        &mut self,
        target: Duration,
        wanted: impl Fn() -> bool,
    ) -> Result<Option<(Arc<RenderImage>, Duration, Duration)>> {
        if self.terminal || !wanted() {
            return Ok(None);
        }
        let started = Instant::now();
        let mut latest = None;
        // Whether `latest` has been through its source's `render` already.
        let mut rendered = false;
        for _ in 0..DECODE_SLICE {
            if !wanted() {
                break;
            }
            let reading = Instant::now();
            let read = self.source.read();
            if self.spool.is_some() {
                self.spent += reading.elapsed();
            }
            let frame = match read {
                Ok(SourceRead::Frame(frame)) if self.count < self.source.max_frames() => frame,
                Ok(SourceRead::End) if self.count > 1 => {
                    let cycle = self.position;
                    let loops = target.saturating_sub(self.base).as_nanos() / cycle.as_nanos();
                    self.cycles = self
                        .cycles
                        .saturating_add(u64::try_from(loops.max(1)).unwrap_or(u64::MAX));
                    if self
                        .plays
                        .is_some_and(|plays| self.cycles >= u64::from(plays.get()))
                    {
                        // Played out: the last frame shown stays.
                        self.terminal = true;
                        break;
                    }
                    self.base += Duration::from_nanos(
                        (cycle.as_nanos() * loops.max(1)).min(u128::from(u64::MAX)) as u64,
                    );
                    let mut swapped = false;
                    if let Some(file) = self.spool.as_ref()
                        && file.exists()
                        && let Some(expected) = self.source.target()
                        && let Ok(spool) = SpoolFrames::open(file, expected)
                    {
                        // A frame read from the old source needs that source's resize and
                        // colour order; the spool's frames are stored done.
                        if !rendered {
                            latest = latest
                                .map(|(frame, start, end)| (self.source.render(frame), start, end));
                            rendered = true;
                        }
                        self.source = Box::new(spool);
                        swapped = true;
                    }
                    if swapped {
                        self.spool = None;
                    } else if self.source.restart().is_err() {
                        self.terminal = true;
                        break;
                    }
                    self.count = 0;
                    self.position = Duration::ZERO;
                    self.spent = Duration::ZERO;
                    continue;
                }
                _ => {
                    self.terminal = true;
                    break;
                }
            };
            self.count += 1;
            #[cfg(test)]
            {
                self.decoded += 1;
            }
            let delay = self.source.delay(&frame).max(Duration::from_nanos(1));
            let start = self.base + self.position;
            self.position += delay;
            let end = self.base + self.position;
            latest = Some((frame, start, end));
            rendered = false;
            if end > target || started.elapsed() >= CATCH_UP_BUDGET {
                break;
            }
        }
        let Some((frame, start, end)) = latest else {
            return Ok(None);
        };
        let rendering = Instant::now();
        let frame = if rendered {
            frame
        } else {
            self.source.render(frame)
        };
        if self.spool.is_some() {
            self.spent += rendering.elapsed();
        }
        Ok(Some((Arc::new(RenderImage::new(vec![frame])), start, end)))
    }
}
