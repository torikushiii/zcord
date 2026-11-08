mod devices;
mod flags;
mod mic_test;
mod pipeline;
mod streams;

pub use devices::{AudioDevice, AudioDeviceKind, AudioDevices, audio_devices};
pub(crate) use flags::AudioFlags;
pub use mic_test::MicTest;
pub(super) use pipeline::{VoiceGate, stop_sequence};

use pipeline::{frame_values, run_input, run_output};
use streams::{StereoResampler, build_input, build_output};

use cpal::{
    FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use nnnoiseless::DenoiseState;
use opus::{Application, Bitrate, Channels, Decoder, Encoder};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

const SAMPLE_RATE: u32 = 48_000;
const FRAME_SAMPLES: usize = 960;
const FRAME_VALUES: usize = FRAME_SAMPLES * 2;
const MAX_OPUS_FRAME: usize = 5760;
const VAD_THRESHOLD_DB: f32 = -50.0;
const VOICE_PROBABILITY_THRESHOLD: f32 = 0.5;
const VAD_HANGOVER: Duration = Duration::from_millis(300);
const RNNOISE_FRAME_SAMPLES: usize = 480;
const PLAYBACK_LIMIT: usize = FRAME_VALUES * 10;
const MAX_OPUS_PAYLOAD: usize = 1275;
const STREAM_AUDIO_LIMIT: usize = 8;

#[derive(Debug)]
pub(super) enum Outgoing {
    Speaking(bool),
    Frame(Vec<u8>),
}

pub(super) struct Incoming {
    pub ssrc: u32,
    pub sequence: u16,
    pub opus: Vec<u8>,
}

pub(super) struct AudioHandle {
    outgoing: Option<UnboundedReceiver<Outgoing>>,
    pub incoming: UnboundedSender<Incoming>,
    stop: Arc<AtomicBool>,
    outgoing_tx: UnboundedSender<Outgoing>,
    incoming_rx: Option<UnboundedReceiver<Incoming>>,
    flags: AudioFlags,
    input: Side,
    output: Side,
    mic_frames: Arc<AtomicU64>,
    gate_open: Arc<AtomicU64>,
    encoded: Arc<AtomicU64>,
}

struct Side {
    label: &'static str,
    worker: Option<JoinHandle<SideExit>>,
    stop: Arc<AtomicBool>,
    device_generation: u64,
    started: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    started_at: Instant,
    logged_running: bool,
    failures: u32,
    next_attempt: Instant,
    last_warn: Option<Instant>,
}

impl Side {
    fn new(label: &'static str) -> Self {
        Self {
            label,
            worker: None,
            stop: Arc::new(AtomicBool::new(false)),
            device_generation: 0,
            started: Arc::new(AtomicBool::new(false)),
            active: Arc::new(AtomicBool::new(false)),
            started_at: Instant::now(),
            logged_running: false,
            failures: 0,
            next_attempt: Instant::now(),
            last_warn: None,
        }
    }

    fn begin_spawn(
        &mut self,
        generation: u64,
    ) -> (Arc<AtomicBool>, Arc<AtomicBool>, Arc<AtomicBool>) {
        self.stop = Arc::new(AtomicBool::new(false));
        self.device_generation = generation;
        self.started = Arc::new(AtomicBool::new(false));
        self.active = Arc::new(AtomicBool::new(false));
        self.started_at = Instant::now();
        self.logged_running = false;
        (self.started.clone(), self.active.clone(), self.stop.clone())
    }

    fn failed(&mut self, now: Instant, reason: String) {
        self.failures = self.failures.saturating_add(1);
        self.next_attempt = now + retry_delay(self.failures);
        if failure_warning_due(now, self.last_warn) {
            if self.last_warn.is_some() {
                log::warn!("{} still failing: {reason}", self.label);
            } else {
                log::warn!("{} stream failed: {reason}", self.label);
            }
            self.last_warn = Some(now);
        }
    }

    fn recovered(&mut self, now: Instant) {
        self.failures = 0;
        self.next_attempt = now;
        self.last_warn = None;
    }

    fn ready(&self, now: Instant) -> bool {
        self.worker.is_none() && now >= self.next_attempt
    }

    fn running_due(&self, now: Instant) -> bool {
        self.worker.is_some()
            && !self.logged_running
            && self.started.load(Ordering::Acquire)
            && now.saturating_duration_since(self.started_at) >= Duration::from_secs(1)
    }

    fn mark_running(&mut self) {
        if self.active.load(Ordering::Acquire) {
            log::info!("{} running", self.label);
        } else {
            log::info!("{} unavailable", self.label);
        }
        self.logged_running = true;
        self.failures = 0;
        self.last_warn = None;
    }
}

struct SideExit {
    incoming: Option<UnboundedReceiver<Incoming>>,
    failure: Option<String>,
    stopped: bool,
}

fn is_fatal(kind: cpal::ErrorKind) -> bool {
    !matches!(
        kind,
        cpal::ErrorKind::Xrun | cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied
    )
}

#[derive(Clone, Default)]
struct StreamFailure {
    failed: Arc<AtomicBool>,
    message: Arc<Mutex<Option<String>>>,
    notice: Arc<Mutex<Option<(cpal::ErrorKind, String)>>>,
}

impl StreamFailure {
    fn report(&self, error: cpal::Error) {
        let kind = error.kind();
        if is_fatal(kind) {
            let mut current = self.message.lock().unwrap_or_else(PoisonError::into_inner);
            *current = Some(error.to_string());
            self.failed.store(true, Ordering::Release);
        } else {
            let mut notice = self.notice.lock().unwrap_or_else(PoisonError::into_inner);
            *notice = Some((kind, error.to_string()));
        }
    }

    fn take(&self) -> Option<String> {
        if !self.failed.swap(false, Ordering::AcqRel) {
            return None;
        }
        let message = self
            .message
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        Some(message.unwrap_or_else(|| "audio stream error".into()))
    }

    fn take_notice(&self) -> Option<(cpal::ErrorKind, String)> {
        self.notice
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

struct Input {
    _stream: Stream,
    queue: Arc<Mutex<VecDeque<f32>>>,
    resampler: StereoResampler,
}

struct Output {
    _stream: Stream,
    queue: Arc<Mutex<VecDeque<f32>>>,
    resampler: StereoResampler,
    frame_values: usize,
    underruns: Arc<AtomicU64>,
}

fn retry_delay(failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(5);
    Duration::from_secs((1u64 << shift).min(30))
}

fn failure_warning_due(now: Instant, last_warn: Option<Instant>) -> bool {
    last_warn.is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(30))
}

impl AudioHandle {
    pub fn start(flags: AudioFlags) -> Self {
        let (incoming, incoming_rx) = unbounded_channel();
        let (outgoing_tx, outgoing) = unbounded_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let mut audio = Self {
            outgoing: Some(outgoing),
            incoming,
            stop,
            outgoing_tx,
            incoming_rx: Some(incoming_rx),
            flags,
            input: Side::new("Microphone"),
            output: Side::new("Speakers"),
            mic_frames: Arc::new(AtomicU64::new(0)),
            gate_open: Arc::new(AtomicU64::new(0)),
            encoded: Arc::new(AtomicU64::new(0)),
        };
        audio.spawn_input();
        audio.spawn_output();
        audio
    }

    pub fn take_outgoing(&mut self) -> Option<UnboundedReceiver<Outgoing>> {
        self.outgoing.take()
    }

    pub fn take_input_stats(&self) -> (u64, u64, u64) {
        (
            self.mic_frames.swap(0, Ordering::Relaxed),
            self.gate_open.swap(0, Ordering::Relaxed),
            self.encoded.swap(0, Ordering::Relaxed),
        )
    }

    pub fn maintain(&mut self) {
        if self.stop.load(Ordering::Acquire) {
            return;
        }
        self.maintain_input();
        self.maintain_output();
    }

    fn maintain_input(&mut self) {
        let generation = self.flags.input_generation();
        if self.input.device_generation != generation {
            self.restart_input(generation);
        }
        let now = Instant::now();
        if self
            .input
            .worker
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
        {
            match self
                .input
                .worker
                .take()
                .and_then(|worker| worker.join().ok())
            {
                Some(exit) => {
                    if let Some(incoming) = exit.incoming {
                        self.incoming_rx = Some(incoming);
                    }
                    if exit.stopped {
                        return;
                    }
                    if let Some(reason) = exit.failure {
                        self.input.failed(now, reason);
                    } else {
                        self.input.recovered(now);
                    }
                }
                None => self.input.failed(now, "audio thread panicked".into()),
            }
        }
        if self.input.ready(Instant::now()) {
            self.spawn_input();
        }
        let now = Instant::now();
        if self.input.running_due(now) {
            self.input.mark_running();
        }
    }

    fn maintain_output(&mut self) {
        let generation = self.flags.output_generation();
        if self.output.device_generation != generation {
            self.restart_output(generation);
        }
        let now = Instant::now();
        if self
            .output
            .worker
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
        {
            match self
                .output
                .worker
                .take()
                .and_then(|worker| worker.join().ok())
            {
                Some(exit) => {
                    match exit.incoming {
                        Some(incoming) => self.incoming_rx = Some(incoming),
                        None => self.reset_incoming(),
                    }
                    if exit.stopped {
                        return;
                    }
                    if let Some(reason) = exit.failure {
                        self.output.failed(now, reason);
                    } else {
                        self.output.recovered(now);
                    }
                }
                None => {
                    self.reset_incoming();
                    self.output.failed(now, "audio thread panicked".into());
                }
            }
        }
        if self.output.ready(Instant::now()) {
            self.spawn_output();
        }
        let now = Instant::now();
        if self.output.running_due(now) {
            self.output.mark_running();
        }
    }

    fn restart_input(&mut self, generation: u64) {
        self.input.stop.store(true, Ordering::Release);
        if let Some(worker) = self.input.worker.take() {
            let _ = worker.join();
        }
        self.input.device_generation = generation;
        self.input.started_at = Instant::now();
        self.input.next_attempt = Instant::now();
        self.input.failures = 0;
        self.input.logged_running = false;
        self.input.last_warn = None;
    }

    fn restart_output(&mut self, generation: u64) {
        self.output.stop.store(true, Ordering::Release);
        if let Some(worker) = self.output.worker.take() {
            match worker.join() {
                Ok(exit) => match exit.incoming {
                    Some(incoming) => self.incoming_rx = Some(incoming),
                    None => self.reset_incoming(),
                },
                Err(_) => self.reset_incoming(),
            }
        }
        self.output.device_generation = generation;
        self.output.started_at = Instant::now();
        self.output.next_attempt = Instant::now();
        self.output.failures = 0;
        self.output.logged_running = false;
        self.output.last_warn = None;
    }

    fn reset_incoming(&mut self) {
        let (incoming, incoming_rx) = unbounded_channel();
        self.incoming = incoming;
        self.incoming_rx = Some(incoming_rx);
    }

    fn spawn_input(&mut self) {
        let generation = self.flags.input_generation();
        let (started, active, side_stop) = self.input.begin_spawn(generation);
        let device = self.flags.input_device();
        let outgoing = self.outgoing_tx.clone();
        let flags = self.flags.clone();
        let stop = self.stop.clone();
        let mic_frames = self.mic_frames.clone();
        let gate_open = self.gate_open.clone();
        let encoded = self.encoded.clone();
        let worker = std::thread::Builder::new()
            .name("zcord-voice-in".into())
            .spawn(move || {
                run_input(
                    outgoing, flags, stop, side_stop, device, started, active, mic_frames,
                    gate_open, encoded,
                )
            })
            .map_err(|error| format!("thread: {error}"));
        match worker {
            Ok(worker) => self.input.worker = Some(worker),
            Err(reason) => self.input.failed(Instant::now(), reason),
        }
    }

    fn spawn_output(&mut self) {
        if self.incoming_rx.is_none() {
            self.reset_incoming();
        }
        let Some(incoming) = self.incoming_rx.take() else {
            return;
        };
        let holder = Arc::new(Mutex::new(Some(incoming)));
        let generation = self.flags.output_generation();
        let (started, active, side_stop) = self.output.begin_spawn(generation);
        let device = self.flags.output_device();
        let thread_holder = holder.clone();
        let flags = self.flags.clone();
        let stop = self.stop.clone();
        let worker = std::thread::Builder::new()
            .name("zcord-voice-out".into())
            .spawn(move || {
                let incoming = thread_holder
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                match incoming {
                    Some(incoming) => {
                        run_output(incoming, flags, stop, side_stop, device, started, active)
                    }
                    None => SideExit {
                        incoming: None,
                        failure: Some("incoming audio receiver missing".into()),
                        stopped: false,
                    },
                }
            });
        match worker {
            Ok(worker) => self.output.worker = Some(worker),
            Err(error) => {
                self.incoming_rx = holder.lock().unwrap_or_else(PoisonError::into_inner).take();
                self.output
                    .failed(Instant::now(), format!("thread: {error}"));
            }
        }
    }
}

impl Drop for AudioHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.input.stop.store(true, Ordering::Release);
        self.output.stop.store(true, Ordering::Release);
        self.flags.clear_input_level();
        self.flags.set_gate_open(false);
        let _ = self.input.worker.take();
        let _ = self.output.worker.take();
    }
}

fn open_input(
    device: Option<&cpal::Device>,
    failure: StreamFailure,
) -> Result<Option<Input>, String> {
    let Some(device) = device else {
        return Ok(None);
    };
    let config = device
        .default_input_config()
        .map_err(|error| format!("configuration: {error}"))?;
    let channels = usize::from(config.channels());
    let rate = config.sample_rate();
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let stream = build_input(device, &config, queue.clone(), failure)
        .map_err(|error| format!("build: {error}"))?;
    stream.play().map_err(|error| format!("start: {error}"))?;
    Ok(Some(Input {
        _stream: stream,
        queue,
        resampler: StereoResampler::new(rate, SAMPLE_RATE, channels, 2),
    }))
}

fn open_output(
    device: Option<&cpal::Device>,
    flags: AudioFlags,
    failure: StreamFailure,
) -> Result<Option<Output>, String> {
    let Some(device) = device else {
        return Ok(None);
    };
    let config = device
        .default_output_config()
        .map_err(|error| format!("configuration: {error}"))?;
    let channels = usize::from(config.channels());
    let rate = config.sample_rate();
    let queue = Arc::new(Mutex::new(VecDeque::new()));
    let underruns = Arc::new(AtomicU64::new(0));
    let stream = build_output(
        device,
        &config,
        queue.clone(),
        flags,
        failure,
        underruns.clone(),
    )
    .map_err(|error| format!("build: {error}"))?;
    stream.play().map_err(|error| format!("start: {error}"))?;
    Ok(Some(Output {
        _stream: stream,
        queue,
        resampler: StereoResampler::new(SAMPLE_RATE, rate, 2, channels),
        frame_values: frame_values(rate, channels),
        underruns,
    }))
}

fn send_stop(outgoing: &UnboundedSender<Outgoing>, speaking: &mut bool) {
    if *speaking {
        for event in stop_sequence() {
            let _ = outgoing.send(event);
        }
        *speaking = false;
    }
}

#[cfg(test)]
mod tests;
