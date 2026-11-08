use super::decoder::source;
use super::*;

const SPOOL_HEADER: u64 = 12;
const SPOOL_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
pub(super) enum SourceKind {
    Image,
    #[cfg(windows)]
    Video {
        max_fps: u32,
    },
    #[cfg(test)]
    Fake(Arc<dyn Fn() -> Result<Box<dyn FrameSource>> + Send + Sync>),
}

impl SourceKind {
    pub(super) fn of(spec: &ImageSpec) -> Self {
        #[cfg(windows)]
        if matches!(spec, ImageSpec::Video { .. } | ImageSpec::LocalVideo { .. }) {
            return Self::Video {
                max_fps: crate::media::video::animation::max_fps(spec),
            };
        }
        let _ = spec;
        Self::Image
    }
}

pub(crate) enum SourceRead {
    Frame(Frame),
    End,
}

pub(crate) trait FrameSource {
    fn read(&mut self) -> Result<SourceRead>;
    fn restart(&mut self) -> Result<()>;
    fn max_frames(&self) -> usize;
    fn target(&self) -> Option<(u32, u32)> {
        None
    }
    fn delay(&self, frame: &Frame) -> Duration {
        Duration::from(frame.delay())
    }
    fn render(&self, frame: Frame) -> Frame {
        frame
    }
}

pub(super) struct ImageFrames {
    pub(super) path: PathBuf,
    pub(super) frames: Frames<'static>,
    pub(super) dimensions: (u32, u32),
    pub(super) target: (u32, u32),
}

impl FrameSource for ImageFrames {
    fn read(&mut self) -> Result<SourceRead> {
        match self.frames.next() {
            Some(frame) => {
                let frame = frame?;
                ensure!(
                    frame.buffer().dimensions() == self.dimensions,
                    "frame size changed"
                );
                Ok(SourceRead::Frame(frame))
            }
            None => Ok(SourceRead::End),
        }
    }

    fn restart(&mut self) -> Result<()> {
        let (dimensions, frames) = source(&self.path)?;
        ensure!(
            dimensions == self.dimensions,
            "animation dimensions changed"
        );
        self.frames = frames;
        Ok(())
    }

    fn max_frames(&self) -> usize {
        MAX_FRAMES
    }

    fn target(&self) -> Option<(u32, u32)> {
        Some(self.target)
    }

    fn delay(&self, frame: &Frame) -> Duration {
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        if u64::from(numerator) < 20 * u64::from(denominator) {
            Duration::from_millis(100)
        } else {
            Duration::from(frame.delay())
        }
    }

    fn render(&self, frame: Frame) -> Frame {
        process_frame(frame, self.target)
    }
}

pub(super) struct SpoolFrames {
    path: PathBuf,
    reader: BufReader<File>,
    width: u32,
    height: u32,
}

impl SpoolFrames {
    pub(super) fn open(path: &Path, expected: (u32, u32)) -> Result<Self> {
        let file = File::options().read(true).write(true).open(path)?;
        file.set_modified(SystemTime::now())?;
        let mut reader = BufReader::new(file);
        let mut header = [0; SPOOL_HEADER as usize];
        reader.read_exact(&mut header)?;
        ensure!(&header[..4] == b"ZCSP", "invalid spool file");
        let width = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let height = u32::from_le_bytes(header[8..12].try_into().unwrap());
        ensure!((width, height) == expected, "spool dimensions changed");
        Ok(Self {
            path: path.to_path_buf(),
            reader,
            width,
            height,
        })
    }
}

impl FrameSource for SpoolFrames {
    fn read(&mut self) -> Result<SourceRead> {
        let mut delay = [0; 4];
        let mut filled = 0;
        while filled < delay.len() {
            match self.reader.read(&mut delay[filled..]) {
                Ok(0) if filled == 0 => return Ok(SourceRead::End),
                Ok(0) => anyhow::bail!("truncated spool"),
                Ok(read) => filled += read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error.into()),
            }
        }
        let delay = u32::from_le_bytes(delay);
        let mut pixels = vec![0; self.width as usize * self.height as usize * 4];
        self.reader
            .read_exact(&mut pixels)
            .with_context(|| format!("truncated spool {}", self.path.display()))?;
        let pixels =
            RgbaImage::from_raw(self.width, self.height, pixels).context("invalid spool frame")?;
        Ok(SourceRead::Frame(Frame::from_parts(
            pixels,
            0,
            0,
            Delay::from_numer_denom_ms(delay, 1),
        )))
    }

    fn restart(&mut self) -> Result<()> {
        self.reader.seek(SeekFrom::Start(SPOOL_HEADER))?;
        Ok(())
    }

    fn max_frames(&self) -> usize {
        MAX_FRAMES
    }
}

pub(super) fn write_spool(
    source: &Path,
    size: u32,
    file: &Path,
    keep_going: impl Fn() -> bool,
) -> Result<u64> {
    let temp = file.with_extension("spool.tmp");
    let result = write_spool_frames(source, size, &temp, keep_going).and_then(|written| {
        std::fs::rename(&temp, file).context("could not publish spool")?;
        Ok(written)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn write_spool_frames(
    source: &Path,
    size: u32,
    file: &Path,
    keep_going: impl Fn() -> bool,
) -> Result<u64> {
    let (dimensions, frames) = self::source(source)?;
    let target = output_size(dimensions.0, dimensions.1, size, 1 + PLAYBACK_FRAMES)?;
    let frame_bytes = u64::from(target.0) * u64::from(target.1) * 4;
    let mut writer = BufWriter::new(File::create(file)?);
    writer.write_all(b"ZCSP")?;
    writer.write_all(&target.0.to_le_bytes())?;
    writer.write_all(&target.1.to_le_bytes())?;
    let mut total = SPOOL_HEADER;
    for (count, frame) in frames.enumerate() {
        ensure!(keep_going(), "spool cancelled");
        ensure!(count < MAX_FRAMES, "too many animation frames");
        ensure!(total + 4 + frame_bytes <= SPOOL_LIMIT, "spool too large");
        let frame = process_frame(frame?, target);
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = (numerator / denominator).max(1);
        writer.write_all(&delay.to_le_bytes())?;
        writer.write_all(frame.buffer().as_raw())?;
        total += 4 + frame_bytes;
    }
    writer.flush()?;
    Ok(total)
}

pub(super) struct SpoolJob {
    pub(super) source: PathBuf,
    pub(super) size: u32,
    pub(super) file: PathBuf,
    pub(super) service: Option<Arc<images::Service>>,
    pub(super) shared: Weak<Mutex<Demand>>,
}

pub(super) fn spool_pool() -> &'static mpsc::Sender<SpoolJob> {
    static POOL: OnceLock<mpsc::Sender<SpoolJob>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<SpoolJob>();
        let spawned = std::thread::Builder::new()
            .name("gif-spool".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let shared = job.shared;
                    let keep_going = move || {
                        shared.upgrade().is_some_and(|shared| {
                            !shared
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .idle(Instant::now())
                        })
                    };
                    match write_spool(&job.source, job.size, &job.file, keep_going) {
                        Ok(written) => {
                            if let Some(service) = job.service {
                                service.record_frames_write(written);
                            }
                        }
                        Err(error) => log::debug!("Could not spool GIF frames: {error}"),
                    }
                }
            });
        if spawned.is_err() {
            log::warn!("Could not start GIF spool worker");
        }
        sender
    })
}
