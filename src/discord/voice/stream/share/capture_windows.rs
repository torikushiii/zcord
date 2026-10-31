use super::RawFrame;
use crate::discord::voice::stream::{ScreenSource, ScreenSourceId};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::{Duration, Instant},
};
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::{GraphicsCaptureApi, InternalCaptureControl},
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
};

const MAX_SOURCES: usize = 64;
const MAX_SOURCE_WIDTH: u32 = 7680;
const MAX_SOURCE_HEIGHT: u32 = 7680;
const MAX_FRAME_WIDTH: u32 = 7680;
const MAX_FRAME_HEIGHT: u32 = 4320;
const MAX_RAW_BYTES: usize = 256 * 1024 * 1024;

fn monitor_id(monitor: &Monitor) -> u64 {
    u64::try_from(monitor.as_raw_hmonitor() as usize).unwrap_or(0)
}

fn window_id(window: &Window) -> u64 {
    u64::try_from(window.as_raw_hwnd() as usize).unwrap_or(0)
}

fn bounded_name(mut name: String) -> String {
    const MAX: usize = 128;
    if name.len() > MAX {
        let mut end = MAX;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    name
}

pub(crate) fn sources() -> Vec<ScreenSource> {
    let mut sources = Vec::new();
    for monitor in Monitor::enumerate().unwrap_or_default() {
        let (Ok(width), Ok(height)) = (monitor.width(), monitor.height()) else {
            continue;
        };
        if width == 0 || height == 0 || width > MAX_SOURCE_WIDTH || height > MAX_SOURCE_HEIGHT {
            continue;
        }
        let name = monitor
            .name()
            .or_else(|_| monitor.device_string())
            .unwrap_or_else(|_| "Display".to_owned());
        sources.push(ScreenSource {
            id: ScreenSourceId::Monitor(monitor_id(&monitor)),
            name: bounded_name(name),
            is_window: false,
        });
        if sources.len() == MAX_SOURCES {
            return sources;
        }
    }
    for window in Window::enumerate().unwrap_or_default() {
        let Ok(title) = window.title() else {
            continue;
        };
        let (Ok(width), Ok(height)) = (window.width(), window.height()) else {
            continue;
        };
        let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
            continue;
        };
        if title.trim().is_empty()
            || width == 0
            || height == 0
            || width > MAX_SOURCE_WIDTH
            || height > MAX_SOURCE_HEIGHT
        {
            continue;
        }
        sources.push(ScreenSource {
            id: ScreenSourceId::Window(window_id(&window)),
            name: bounded_name(title),
            is_window: true,
        });
        if sources.len() == MAX_SOURCES {
            break;
        }
    }
    sources
}

struct Flags {
    frames: SyncSender<RawFrame>,
    stop: Arc<AtomicBool>,
    keyframe: Arc<AtomicBool>,
    next_frame: Instant,
    interval: Duration,
}

struct Handler(Flags);

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Flags;
    type Error = &'static str;

    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self(context.flags))
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.0.stop.load(Ordering::Acquire) {
            control.stop();
            return Ok(());
        }
        let now = Instant::now();
        if now < self.0.next_frame {
            return Ok(());
        }
        self.0.next_frame = (self.0.next_frame + self.0.interval).max(now + self.0.interval / 2);
        let (width, height) = (frame.width(), frame.height());
        let Some(row_bytes) = (width as usize).checked_mul(4) else {
            control.stop();
            return Err("Captured frame dimensions are invalid");
        };
        let Some(data_len) = row_bytes.checked_mul(height as usize) else {
            control.stop();
            return Err("Captured frame dimensions are invalid");
        };
        if width == 0
            || height == 0
            || width > MAX_FRAME_WIDTH
            || height > MAX_FRAME_HEIGHT
            || data_len > MAX_RAW_BYTES
        {
            control.stop();
            return Err("Captured frame exceeds the 4K limit");
        }
        let mut buffer = frame
            .buffer()
            .map_err(|_| "Captured frame could not be read")?;
        let stride = buffer.row_pitch() as usize;
        let Some(source_len) = stride.checked_mul(height as usize) else {
            control.stop();
            return Err("Captured frame stride is invalid");
        };
        if stride < row_bytes || source_len > MAX_RAW_BYTES {
            control.stop();
            return Err("Captured frame stride exceeds the 4K limit");
        }
        let source = buffer.as_raw_buffer();
        if source.len() < source_len {
            control.stop();
            return Err("Captured frame buffer is incomplete");
        }
        let mut data = Vec::with_capacity(data_len);
        for row in source[..source_len].chunks_exact(stride) {
            data.extend_from_slice(&row[..row_bytes]);
        }
        if self
            .0
            .frames
            .try_send(RawFrame {
                width,
                height,
                stride: row_bytes,
                data,
            })
            .is_err()
        {
            self.0.keyframe.store(true, Ordering::Release);
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.0.stop.store(true, Ordering::Release);
        Ok(())
    }
}

pub(crate) struct Capture {
    control: Option<CaptureControl<Handler, &'static str>>,
}

impl Capture {
    pub(crate) fn start(
        source: &ScreenSource,
        frames: SyncSender<RawFrame>,
        stop: Arc<AtomicBool>,
        keyframe: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let flags = |next_frame| Flags {
            frames,
            stop,
            keyframe,
            next_frame,
            interval: Duration::from_secs_f64(1. / 30.),
        };
        let control = match source.id {
            ScreenSourceId::Monitor(id) => {
                let monitor = Monitor::enumerate()
                    .map_err(|_| "Displays could not be enumerated".to_owned())?
                    .into_iter()
                    .find(|monitor| monitor_id(monitor) == id)
                    .ok_or_else(|| "Selected display is no longer available".to_owned())?;
                start_item(monitor, flags)
            }
            ScreenSourceId::Window(id) => {
                let window = Window::enumerate()
                    .map_err(|_| "Windows could not be enumerated".to_owned())?
                    .into_iter()
                    .find(|window| window_id(window) == id)
                    .ok_or_else(|| "Selected window is no longer available".to_owned())?;
                start_item(window, flags)
            }
        }?;
        Ok(Self {
            control: Some(control),
        })
    }
}

pub(crate) fn thumbnail(source: &ScreenSource) -> Option<RawFrame> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let stop = Arc::new(AtomicBool::new(false));
    let keyframe = Arc::new(AtomicBool::new(true));
    let capture = Capture::start(source, sender, stop.clone(), keyframe).ok()?;
    let frame = receiver.recv_timeout(Duration::from_secs(2)).ok();
    stop.store(true, Ordering::Release);
    drop(capture);
    frame
}

impl Drop for Capture {
    fn drop(&mut self) {
        if let Some(control) = self.control.take() {
            let _ = control.stop();
        }
    }
}

trait NativeSource:
    TryInto<windows_capture::settings::GraphicsCaptureItemType> + Send + 'static
{
    fn dimensions(&self) -> Option<(u32, u32)>;
}

impl NativeSource for Monitor {
    fn dimensions(&self) -> Option<(u32, u32)> {
        self.width().ok().zip(self.height().ok())
    }
}

impl NativeSource for Window {
    fn dimensions(&self) -> Option<(u32, u32)> {
        self.width()
            .ok()
            .and_then(|width| u32::try_from(width).ok())
            .zip(
                self.height()
                    .ok()
                    .and_then(|height| u32::try_from(height).ok()),
            )
    }
}

fn start_item<T>(
    item: T,
    flags: impl FnOnce(Instant) -> Flags,
) -> Result<CaptureControl<Handler, &'static str>, String>
where
    T: NativeSource,
{
    let Some((width, height)) = item.dimensions() else {
        return Err("Selected source dimensions are unavailable".into());
    };
    if width == 0 || height == 0 || width > MAX_FRAME_WIDTH || height > MAX_FRAME_HEIGHT {
        return Err("Selected source exceeds the 4K capture limit".into());
    }
    let now = Instant::now();
    let settings = Settings::new(
        item,
        CursorCaptureSettings::Default,
        if GraphicsCaptureApi::is_border_settings_supported().unwrap_or(false) {
            DrawBorderSettings::WithoutBorder
        } else {
            DrawBorderSettings::Default
        },
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags(now),
    );
    Handler::start_free_threaded(settings)
        .map_err(|_| "Screen capture could not be started".to_owned())
}
