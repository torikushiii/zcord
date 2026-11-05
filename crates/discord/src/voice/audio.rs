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

static MISSING_DEVICES: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
    /// The system's full name for the device, which zcord saves to open it again.
    pub name: String,
    /// The name Discord shows: the adapter, such as "UGREEN HiTune Max5c".
    pub label: String,
    /// The endpoint on it, such as "Headphones", when the system names one.
    pub detail: Option<String>,
    pub kind: AudioDeviceKind,
}

/// What a device is, for its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioDeviceKind {
    Bluetooth,
    /// Sound through a monitor or TV, over HDMI or DisplayPort.
    Display,
    Headphones,
    Microphone,
    Speaker,
    Other,
}

#[derive(Clone, Debug, Default)]
pub struct AudioDevices {
    pub inputs: Vec<AudioDevice>,
    pub outputs: Vec<AudioDevice>,
    /// What the system default currently is, shown under "Windows Default".
    pub default_input: Option<AudioDevice>,
    pub default_output: Option<AudioDevice>,
}

pub fn audio_devices() -> AudioDevices {
    let host = cpal::default_host();
    AudioDevices {
        inputs: host.input_devices().map(device_list).unwrap_or_default(),
        outputs: host.output_devices().map(device_list).unwrap_or_default(),
        default_input: host.default_input_device().as_ref().and_then(describe),
        default_output: host.default_output_device().as_ref().and_then(describe),
    }
}

fn device_list(devices: impl Iterator<Item = cpal::Device>) -> Vec<AudioDevice> {
    let mut list: Vec<AudioDevice> = Vec::new();
    for device in devices {
        let Some(device) = describe(&device) else {
            continue;
        };
        if !list.iter().any(|listed| listed.name == device.name) {
            list.push(device);
        }
    }
    list
}

fn describe(device: &cpal::Device) -> Option<AudioDevice> {
    let description = device.description().ok()?;
    let name = description.name().to_owned();
    let (label, detail) = split_name(&name);
    Some(AudioDevice {
        label,
        detail,
        kind: device_kind(description.interface_type(), description.device_type()),
        name,
    })
}

/// Windows names an endpoint "Headphones (UGREEN HiTune Max5c)"; Discord shows the part in
/// brackets as the name and the rest under it.
fn split_name(name: &str) -> (String, Option<String>) {
    let parts = name
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
        .filter(|(endpoint, adapter)| !endpoint.is_empty() && !adapter.is_empty());
    match parts {
        Some((endpoint, adapter)) => (adapter.to_owned(), Some(endpoint.to_owned())),
        None => (name.to_owned(), None),
    }
}

fn device_kind(interface: cpal::InterfaceType, kind: cpal::DeviceType) -> AudioDeviceKind {
    use cpal::{DeviceType, InterfaceType};
    match (interface, kind) {
        (InterfaceType::Bluetooth, _) => AudioDeviceKind::Bluetooth,
        (InterfaceType::Hdmi | InterfaceType::DisplayPort, _) => AudioDeviceKind::Display,
        (
            _,
            DeviceType::Headphones
            | DeviceType::Headset
            | DeviceType::Earpiece
            | DeviceType::Handset
            | DeviceType::HearingAid,
        ) => AudioDeviceKind::Headphones,
        (_, DeviceType::Microphone) => AudioDeviceKind::Microphone,
        (_, DeviceType::Speaker) => AudioDeviceKind::Speaker,
        _ => AudioDeviceKind::Other,
    }
}

pub(super) struct Flags {
    pub mute: AtomicBool,
    pub deaf: AtomicBool,
    pub noise_suppression: AtomicBool,
    input_gain: AtomicU32,
    output_gain: AtomicU32,
    stream_volume: AtomicU32,
    sensitivity: AtomicU32,
    input_level: AtomicU32,
    gate_open: AtomicBool,
    input_device: Mutex<Option<String>>,
    output_device: Mutex<Option<String>>,
    input_generation: AtomicU64,
    output_generation: AtomicU64,
    stream_audio: Mutex<VecDeque<StreamPacket>>,
    stream_generation: AtomicU64,
}

#[derive(Clone)]
pub(crate) struct AudioFlags(Arc<Flags>);

struct StreamPacket {
    sequence: u16,
    opus: Vec<u8>,
}

impl AudioFlags {
    pub fn new() -> Self {
        Self(Arc::new(Flags {
            mute: AtomicBool::new(false),
            deaf: AtomicBool::new(false),
            noise_suppression: AtomicBool::new(true),
            input_gain: AtomicU32::new(1.0f32.to_bits()),
            output_gain: AtomicU32::new(1.0f32.to_bits()),
            stream_volume: AtomicU32::new(1.0f32.to_bits()),
            sensitivity: AtomicU32::new(f32::NAN.to_bits()),
            input_level: AtomicU32::new(f32::NAN.to_bits()),
            gate_open: AtomicBool::new(false),
            input_device: Mutex::new(None),
            output_device: Mutex::new(None),
            input_generation: AtomicU64::new(0),
            output_generation: AtomicU64::new(0),
            stream_audio: Mutex::new(VecDeque::new()),
            stream_generation: AtomicU64::new(0),
        }))
    }

    pub fn set(&self, mute: bool, deaf: bool) {
        self.0.mute.store(mute, Ordering::Relaxed);
        self.0.deaf.store(deaf, Ordering::Relaxed);
    }

    fn muted(&self) -> bool {
        self.0.mute.load(Ordering::Relaxed)
    }

    fn deafened(&self) -> bool {
        self.0.deaf.load(Ordering::Relaxed)
    }

    pub fn set_noise_suppression(&self, enabled: bool) {
        self.0.noise_suppression.store(enabled, Ordering::Relaxed);
    }

    pub fn noise_suppression(&self) -> bool {
        self.0.noise_suppression.load(Ordering::Relaxed)
    }

    pub fn set_input_gain(&self, gain: f32) {
        store_f32(&self.0.input_gain, gain.clamp(0.0, 2.0));
    }

    fn input_gain(&self) -> f32 {
        load_f32(&self.0.input_gain)
    }

    pub fn set_output_gain(&self, gain: f32) {
        store_f32(&self.0.output_gain, gain.clamp(0.0, 2.0));
    }

    fn output_gain(&self) -> f32 {
        load_f32(&self.0.output_gain)
    }

    pub fn set_stream_volume(&self, gain: f32) {
        if gain.is_finite() {
            store_f32(&self.0.stream_volume, gain.clamp(0.0, 2.0));
        }
    }

    fn stream_volume(&self) -> f32 {
        load_f32(&self.0.stream_volume).clamp(0.0, 2.0)
    }

    pub(super) fn begin_stream_audio(&self) -> u64 {
        let mut queue = self
            .0
            .stream_audio
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        queue.clear();
        self.0
            .stream_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    pub(super) fn push_stream_audio(&self, generation: u64, sequence: u16, opus: Vec<u8>) {
        let mut queue = match self.0.stream_audio.try_lock() {
            Ok(queue) => queue,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return,
        };
        if self.0.stream_generation.load(Ordering::Acquire) != generation {
            return;
        }
        if queue.len() >= STREAM_AUDIO_LIMIT {
            queue.pop_front();
        }
        queue.push_back(StreamPacket { sequence, opus });
    }

    fn take_stream_audio(&self, packets: &mut Vec<StreamPacket>) {
        let mut queue = self
            .0
            .stream_audio
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        packets.extend(queue.drain(..));
    }

    pub(super) fn clear_stream_audio(&self) {
        let mut queue = self
            .0
            .stream_audio
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        queue.clear();
        self.0.stream_generation.fetch_add(1, Ordering::AcqRel);
    }

    fn stream_generation(&self) -> u64 {
        self.0.stream_generation.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn queued_stream_audio(&self) -> usize {
        self.0
            .stream_audio
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub fn set_sensitivity(&self, threshold: Option<f32>) {
        let value = threshold.map_or(f32::NAN, |db| db.clamp(-100.0, 0.0));
        store_f32(&self.0.sensitivity, value);
    }

    fn sensitivity(&self) -> Option<f32> {
        let value = load_f32(&self.0.sensitivity);
        (!value.is_nan()).then_some(value)
    }

    fn set_input_level(&self, db: f32) {
        store_f32(&self.0.input_level, db);
    }

    fn clear_input_level(&self) {
        store_f32(&self.0.input_level, f32::NAN);
    }

    pub fn input_level(&self) -> Option<f32> {
        let value = load_f32(&self.0.input_level);
        (!value.is_nan()).then_some(value)
    }

    fn set_gate_open(&self, open: bool) {
        self.0.gate_open.store(open, Ordering::Relaxed);
    }

    pub fn gate_open(&self) -> bool {
        self.0.gate_open.load(Ordering::Relaxed)
    }

    pub fn set_input_device(&self, name: Option<String>) {
        store_device(&self.0.input_device, &self.0.input_generation, name);
    }

    fn input_device(&self) -> Option<String> {
        self.0
            .input_device
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn input_generation(&self) -> u64 {
        self.0.input_generation.load(Ordering::Acquire)
    }

    pub fn set_output_device(&self, name: Option<String>) {
        store_device(&self.0.output_device, &self.0.output_generation, name);
    }

    fn output_device(&self) -> Option<String> {
        self.0
            .output_device
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn output_generation(&self) -> u64 {
        self.0.output_generation.load(Ordering::Acquire)
    }
}

fn store_f32(atom: &AtomicU32, value: f32) {
    atom.store(value.to_bits(), Ordering::Relaxed);
}

fn load_f32(atom: &AtomicU32) -> f32 {
    f32::from_bits(atom.load(Ordering::Relaxed))
}

fn store_device(slot: &Mutex<Option<String>>, generation: &AtomicU64, name: Option<String>) {
    let mut current = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if *current == name {
        return;
    }
    *current = name;
    generation.fetch_add(1, Ordering::AcqRel);
}

fn pick(names: &[String], wanted: Option<&str>) -> Option<usize> {
    let wanted = wanted?;
    names.iter().position(|name| name == wanted)
}

fn named_devices(devices: impl Iterator<Item = cpal::Device>) -> Vec<(cpal::Device, String)> {
    let mut named = Vec::new();
    for device in devices {
        if let Ok(description) = device.description() {
            named.push((device, description.name().to_owned()));
        }
    }
    named
}

fn matching_device(host: &cpal::Host, wanted: &str, input: bool) -> Option<cpal::Device> {
    let named = if input {
        named_devices(host.input_devices().ok()?)
    } else {
        named_devices(host.output_devices().ok()?)
    };
    let names: Vec<String> = named.iter().map(|(_, name)| name.clone()).collect();
    let index = pick(&names, Some(wanted))?;
    named.into_iter().nth(index).map(|(device, _)| device)
}

fn log_missing_device(label: &str, name: &str) {
    let mut missing = MISSING_DEVICES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if missing.iter().any(|existing| existing == name) {
        return;
    }
    missing.push(name.to_owned());
    log::info!("{label} device {name:?} not found; using default");
}

fn preferred_device(
    host: &cpal::Host,
    wanted: Option<&str>,
    input: bool,
    log_missing: bool,
) -> Option<cpal::Device> {
    if let Some(wanted) = wanted
        && let Some(device) = matching_device(host, wanted, input)
    {
        return Some(device);
    }
    if let Some(wanted) = wanted
        && log_missing
    {
        log_missing_device(if input { "Microphone" } else { "Speakers" }, wanted);
    }
    if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    }
}

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

#[derive(Default)]
struct Denoiser {
    state: Option<Box<DenoiseState<'static>>>,
    primed: bool,
}

impl Denoiser {
    fn process(&mut self, frame: &mut [f32], enabled: bool) -> Option<f32> {
        if !enabled {
            self.state = None;
            self.primed = false;
            return None;
        }

        if self.state.is_none() {
            self.state = Some(DenoiseState::new());
            self.primed = false;
        }
        let Some(state) = self.state.as_mut() else {
            return None;
        };
        // nnnoiseless expects i16-scaled samples in 480-sample blocks, and its
        // first block fades in: https://docs.rs/nnnoiseless/0.5.2/nnnoiseless/struct.DenoiseState.html
        let mut mono = [0.0; RNNOISE_FRAME_SAMPLES];
        let mut denoised = [0.0; RNNOISE_FRAME_SAMPLES];
        let mut voice_probability = 0.0f32;
        for block in frame.chunks_exact_mut(RNNOISE_FRAME_SAMPLES * 2) {
            for (sample, pair) in mono.iter_mut().zip(block.chunks_exact(2)) {
                *sample = (pair[0] + pair[1]) * 0.5 * f32::from(i16::MAX);
            }
            let probability = state.process_frame(&mut denoised, &mono);
            if self.primed {
                voice_probability = voice_probability.max(probability);
                for (pair, sample) in block.chunks_exact_mut(2).zip(denoised) {
                    let value = (sample / f32::from(i16::MAX)).clamp(-1.0, 1.0);
                    pair.fill(value);
                }
            } else {
                block.fill(0.0);
                self.primed = true;
            }
        }
        Some(voice_probability)
    }
}

struct LevelGuard(AudioFlags);

impl Drop for LevelGuard {
    fn drop(&mut self) {
        self.0.clear_input_level();
        self.0.set_gate_open(false);
    }
}

fn apply_gain(frame: &mut [f32], gain: f32) {
    for sample in frame {
        *sample = (*sample * gain).clamp(-1.0, 1.0);
    }
}

fn output_sample(sample: f32, gain: f32) -> f32 {
    (sample * gain).clamp(-1.0, 1.0)
}

fn dbfs(frame: &[f32]) -> f32 {
    let rms = (frame.iter().map(|sample| sample * sample).sum::<f32>() / frame.len().max(1) as f32)
        .sqrt();
    if rms <= 1e-7 {
        -100.0
    } else {
        (20.0 * rms.log10()).max(-100.0)
    }
}

fn run_input(
    outgoing: UnboundedSender<Outgoing>,
    flags: AudioFlags,
    stop: Arc<AtomicBool>,
    side_stop: Arc<AtomicBool>,
    device_name: Option<String>,
    started: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    mic_frames: Arc<AtomicU64>,
    gate_open: Arc<AtomicU64>,
    encoded: Arc<AtomicU64>,
) -> SideExit {
    let _level = LevelGuard(flags.clone());
    let host = cpal::default_host();
    let failure = StreamFailure::default();
    let device = preferred_device(&host, device_name.as_deref(), true, true);
    let mut input = match open_input(device.as_ref(), failure.clone()) {
        Ok(input) => input,
        Err(reason) => {
            return SideExit {
                incoming: None,
                failure: Some(reason),
                stopped: false,
            };
        }
    };
    active.store(input.is_some(), Ordering::Release);
    started.store(true, Ordering::Release);

    let mut encoder = Encoder::new(SAMPLE_RATE, Channels::Stereo, Application::Voip).ok();
    if let Some(encoder) = &mut encoder {
        let _ = encoder.set_bitrate(Bitrate::Bits(64_000));
    }
    let mut capture_48k = Vec::new();
    let mut denoiser = Denoiser::default();
    let mut gate = VoiceGate::default();
    let mut speaking = false;
    let mut next_input_check = Instant::now() + Duration::from_secs(3);
    let mut last_xrun: Option<Instant> = None;

    while !stop.load(Ordering::Acquire) && !side_stop.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(5));

        let now = Instant::now();
        if let Some((kind, text)) = failure.take_notice() {
            if kind == cpal::ErrorKind::Xrun {
                if last_xrun.is_none_or(|last| {
                    now.saturating_duration_since(last) >= Duration::from_secs(10)
                }) {
                    log::debug!("Microphone xrun: {text}");
                    last_xrun = Some(now);
                }
            } else {
                log::debug!("Microphone stream notice: {text}");
            }
        }

        if let Some(input) = &mut input {
            let mut raw = input.queue.lock().unwrap_or_else(PoisonError::into_inner);
            if !raw.is_empty() {
                let samples: Vec<f32> = raw.drain(..).collect();
                drop(raw);
                input.resampler.push(&samples, &mut capture_48k);
            }
        }

        while capture_48k.len() >= FRAME_VALUES {
            let mut frame: Vec<f32> = capture_48k.drain(..FRAME_VALUES).collect();
            mic_frames.fetch_add(1, Ordering::Relaxed);
            apply_gain(&mut frame, flags.input_gain());
            flags.set_input_level(dbfs(&frame));
            let muted = flags.muted() || flags.deafened();
            let voice_probability =
                denoiser.process(&mut frame, !muted && flags.noise_suppression());
            let open = if muted {
                gate.close();
                false
            } else {
                gate.update_with_threshold(
                    &frame,
                    voice_probability,
                    Instant::now(),
                    flags.sensitivity(),
                )
            };
            flags.set_gate_open(open);
            if open && !speaking {
                let _ = outgoing.send(Outgoing::Speaking(true));
            }
            if open {
                gate_open.fetch_add(1, Ordering::Relaxed);
                if let Some(encoder) = &mut encoder
                    && let Some(packet) = encode(encoder, &frame)
                {
                    encoded.fetch_add(1, Ordering::Relaxed);
                    let _ = outgoing.send(Outgoing::Frame(packet));
                }
            } else if speaking {
                send_stop(&outgoing, &mut speaking);
            }
            speaking = open;
        }

        if let Some(reason) = failure.take() {
            send_stop(&outgoing, &mut speaking);
            return SideExit {
                incoming: None,
                failure: Some(reason),
                stopped: false,
            };
        }

        // A running microphone is rebuilt only on stream errors. Probing the device
        // while it streams can fail spuriously (RPC_E_CHANGED_MODE after a Bluetooth reconnect)
        // and would tear down a working stream every few seconds.
        let now = Instant::now();
        if input.is_none() && now >= next_input_check {
            next_input_check = now + Duration::from_secs(3);
            let available = preferred_device(&host, device_name.as_deref(), true, false)
                .is_some_and(|device| device.default_input_config().is_ok());
            if available {
                send_stop(&outgoing, &mut speaking);
                return SideExit {
                    incoming: None,
                    failure: None,
                    stopped: false,
                };
            }
        }
    }

    send_stop(&outgoing, &mut speaking);
    SideExit {
        incoming: None,
        failure: None,
        stopped: true,
    }
}

fn run_output(
    mut incoming: UnboundedReceiver<Incoming>,
    flags: AudioFlags,
    stop: Arc<AtomicBool>,
    side_stop: Arc<AtomicBool>,
    device_name: Option<String>,
    started: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
) -> SideExit {
    let host = cpal::default_host();
    let failure = StreamFailure::default();
    let device = preferred_device(&host, device_name.as_deref(), false, true);
    let mut output = match open_output(device.as_ref(), flags.clone(), failure.clone()) {
        Ok(output) => output,
        Err(reason) => {
            return SideExit {
                incoming: Some(incoming),
                failure: Some(reason),
                stopped: false,
            };
        }
    };
    active.store(output.is_some(), Ordering::Release);
    started.store(true, Ordering::Release);

    let mut decoders: HashMap<u32, DecoderState> = HashMap::new();
    let mut stream: Option<DecoderState> = None;
    let mut stream_generation = flags.stream_generation();
    let mut stream_packets = Vec::new();
    let mut packets = 0u64;
    let mut decoded = 0u64;
    let mut audible = 0u64;
    let mut last_underruns = 0u64;
    let mut next_report = Instant::now() + Duration::from_secs(5);
    let mut last_xrun: Option<Instant> = None;

    while !stop.load(Ordering::Acquire) && !side_stop.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(5));

        let now = Instant::now();
        if let Some((kind, text)) = failure.take_notice() {
            if kind == cpal::ErrorKind::Xrun {
                if last_xrun.is_none_or(|last| {
                    now.saturating_duration_since(last) >= Duration::from_secs(10)
                }) {
                    log::debug!("Speakers xrun: {text}");
                    last_xrun = Some(now);
                }
            } else {
                log::debug!("Speakers stream notice: {text}");
            }
        }

        while let Ok(packet) = incoming.try_recv() {
            packets = packets.saturating_add(1);
            decoders
                .entry(packet.ssrc)
                .or_insert_with(DecoderState::new)
                .jitter
                .push(packet.sequence, packet.opus);
        }

        let generation = flags.stream_generation();
        if generation != stream_generation {
            stream = None;
            stream_generation = generation;
        }
        stream_packets.clear();
        flags.take_stream_audio(&mut stream_packets);
        if !stream_packets.is_empty() {
            let state = stream.get_or_insert_with(DecoderState::new);
            for packet in stream_packets.drain(..) {
                state.jitter.push(packet.sequence, packet.opus);
            }
        }

        if let Some(reason) = failure.take() {
            return SideExit {
                incoming: Some(incoming),
                failure: Some(reason),
                stopped: false,
            };
        }

        let now = Instant::now();
        decoders.retain(|_, state| state.jitter.fresh(now));
        if stream
            .as_ref()
            .is_some_and(|state| !state.jitter.fresh(now))
        {
            stream = None;
        }
        if let Some(output) = &mut output {
            let target = playback_target(output.frame_values);
            let limit = PLAYBACK_LIMIT.max(target.saturating_mul(2));
            for _ in 0..3 {
                let queued = output
                    .queue
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .len();
                if !needs_mix(queued, target) {
                    break;
                }
                let mut mixed = mix(
                    &mut decoders,
                    &mut stream,
                    flags.stream_volume(),
                    flags.deafened(),
                    now,
                    &mut decoded,
                    &mut audible,
                );
                let gain = flags.output_gain();
                for sample in &mut mixed {
                    *sample = output_sample(*sample, gain);
                }
                let mut converted = Vec::new();
                output.resampler.push(&mixed, &mut converted);
                let mut queue = output.queue.lock().unwrap_or_else(PoisonError::into_inner);
                queue.extend(converted);
                while queue.len() > limit {
                    queue.pop_front();
                }
            }
        }

        if now >= next_report {
            let (queue_len, underruns) = match &output {
                Some(output) => {
                    let total = output.underruns.load(Ordering::Relaxed);
                    let delta = total.saturating_sub(last_underruns);
                    last_underruns = total;
                    let queue_len = output
                        .queue
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .len();
                    (queue_len, delta)
                }
                None => (0, 0),
            };
            log::debug!(
                "playback: packets={packets} decoded={decoded} audible={audible} queue={queue_len} underruns={underruns}"
            );
            packets = 0;
            decoded = 0;
            audible = 0;
            next_report = now + Duration::from_secs(5);
        }
    }

    SideExit {
        incoming: Some(incoming),
        failure: None,
        stopped: true,
    }
}

fn frame_values(rate: u32, channels: usize) -> usize {
    rate as usize / 50 * channels
}

fn playback_target(frame_values: usize) -> usize {
    frame_values.saturating_mul(3)
}

fn needs_mix(queued: usize, target: usize) -> bool {
    queued < target
}

fn encode(encoder: &mut Encoder, frame: &[f32]) -> Option<Vec<u8>> {
    let mut pcm = vec![0i16; FRAME_VALUES];
    for (sample, value) in pcm.iter_mut().zip(frame) {
        *sample = (value.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
    }
    encoder
        .encode_vec(&pcm, MAX_OPUS_PAYLOAD)
        .ok()
        .filter(|packet| !packet.is_empty() && packet.len() <= MAX_OPUS_PAYLOAD)
}

fn decode(decoder: &mut Option<Decoder>, packet: &[u8]) -> Option<Vec<i16>> {
    let decoder = decoder.as_mut()?;
    let mut pcm = vec![0i16; MAX_OPUS_FRAME * 2];
    let samples = decoder.decode(packet, &mut pcm, false).ok()?;
    pcm.truncate(samples * 2);
    Some(pcm)
}

fn add_pcm(mix: &mut [f32], pcm: &[i16], gain: f32) {
    for (index, sample) in pcm.iter().take(mix.len()).enumerate() {
        mix[index] += f32::from(*sample) / f32::from(i16::MAX) * gain;
    }
}

fn mix(
    decoders: &mut HashMap<u32, DecoderState>,
    stream: &mut Option<DecoderState>,
    stream_gain: f32,
    deafened: bool,
    now: Instant,
    decoded: &mut u64,
    audible: &mut u64,
) -> Vec<f32> {
    let mut mix = vec![0.0; FRAME_VALUES];
    if deafened {
        for state in decoders.values_mut() {
            state.jitter.discard();
        }
        if let Some(state) = stream {
            state.jitter.discard();
        }
        return mix;
    }
    for state in decoders.values_mut() {
        mix_state(&mut mix, state, 1.0, now, decoded);
    }
    if let Some(state) = stream {
        mix_state(&mut mix, state, stream_gain, now, decoded);
    }
    for sample in &mut mix {
        *sample = sample.clamp(-1.0, 1.0);
    }
    if mix.iter().any(|sample| *sample != 0.0) {
        *audible = audible.saturating_add(1);
    }
    mix
}

fn mix_state(
    mix: &mut [f32],
    state: &mut DecoderState,
    gain: f32,
    now: Instant,
    decoded: &mut u64,
) {
    let pcm = match state.jitter.take(now) {
        Ready::Frame(packet) => decode(&mut state.decoder, &packet),
        Ready::Gap => decode(&mut state.decoder, &[]),
        Ready::Wait => None,
    };
    let Some(pcm) = pcm else {
        return;
    };
    *decoded = decoded.saturating_add(1);
    add_pcm(mix, &pcm, gain);
}

struct DecoderState {
    decoder: Option<Decoder>,
    jitter: JitterBuffer,
}

impl DecoderState {
    fn new() -> Self {
        Self {
            decoder: Decoder::new(SAMPLE_RATE, Channels::Stereo).ok(),
            jitter: JitterBuffer::default(),
        }
    }
}

enum Ready {
    Frame(Vec<u8>),
    Gap,
    Wait,
}

#[derive(Default)]
struct JitterBuffer {
    next: Option<u16>,
    frames: BTreeMap<u16, Vec<u8>>,
    last_arrival: Option<Instant>,
}

impl JitterBuffer {
    fn push(&mut self, sequence: u16, packet: Vec<u8>) {
        let now = Instant::now();
        self.last_arrival = Some(now);
        let next = *self.next.get_or_insert(sequence);
        if sequence.wrapping_sub(next) >= 0x8000 {
            return;
        }
        self.frames.insert(sequence, packet);
        if self.frames.len() > 16 {
            let lowest = self.frames.keys().next().copied();
            self.frames.clear();
            self.next = lowest.map(|sequence| sequence.wrapping_add(1));
        }
    }

    fn fresh(&self, now: Instant) -> bool {
        self.last_arrival
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(5))
    }

    fn take(&mut self, now: Instant) -> Ready {
        let Some(next) = self.next else {
            return Ready::Wait;
        };
        if let Some(packet) = self.frames.remove(&next) {
            self.next = Some(next.wrapping_add(1));
            return Ready::Frame(packet);
        }
        if let Some(lowest) = self.frames.keys().next().copied() {
            let waited = self
                .last_arrival
                .is_some_and(|last| now.duration_since(last) >= Duration::from_millis(60));
            if self.frames.len() >= 3 || waited {
                self.next = Some(lowest.wrapping_add(1));
                if let Some(packet) = self.frames.remove(&lowest) {
                    return Ready::Frame(packet);
                }
            }
            return Ready::Gap;
        }
        Ready::Wait
    }

    fn discard(&mut self) {
        self.frames.clear();
        self.next = None;
    }
}

#[derive(Default)]
pub(super) struct VoiceGate {
    open: bool,
    last_voice: Option<Instant>,
}

impl VoiceGate {
    #[cfg(test)]
    pub fn update(&mut self, frame: &[f32], voice_probability: Option<f32>, now: Instant) -> bool {
        self.update_with_threshold(frame, voice_probability, now, None)
    }

    pub fn update_with_threshold(
        &mut self,
        frame: &[f32],
        voice_probability: Option<f32>,
        now: Instant,
        threshold_db: Option<f32>,
    ) -> bool {
        let rms = (frame.iter().map(|sample| sample * sample).sum::<f32>()
            / frame.len().max(1) as f32)
            .sqrt();
        let db = if rms <= 1e-7 {
            f32::NEG_INFINITY
        } else {
            20.0 * rms.log10()
        };
        let threshold = threshold_db.map_or(VAD_THRESHOLD_DB, |db| db.clamp(-100.0, 0.0));
        let voiced =
            voice_probability.is_none_or(|probability| probability >= VOICE_PROBABILITY_THRESHOLD);
        if db >= threshold && voiced {
            self.open = true;
            self.last_voice = Some(now);
        } else if self.open
            && self
                .last_voice
                .is_none_or(|last| now.duration_since(last) >= VAD_HANGOVER)
        {
            self.open = false;
        }
        self.open
    }

    pub fn close(&mut self) {
        self.open = false;
    }
}

pub(super) fn stop_sequence() -> [Outgoing; 6] {
    [
        Outgoing::Frame(super::dave::SILENCE.to_vec()),
        Outgoing::Frame(super::dave::SILENCE.to_vec()),
        Outgoing::Frame(super::dave::SILENCE.to_vec()),
        Outgoing::Frame(super::dave::SILENCE.to_vec()),
        Outgoing::Frame(super::dave::SILENCE.to_vec()),
        Outgoing::Speaking(false),
    ]
}

pub struct MicTest {
    stop: Arc<AtomicBool>,
    flags: AudioFlags,
    worker: Option<JoinHandle<()>>,
}

impl MicTest {
    pub fn start(
        input: Option<String>,
        output: Option<String>,
        input_gain: f32,
        output_volume: f32,
        noise_suppression: bool,
    ) -> Result<MicTest, String> {
        let flags = AudioFlags::new();
        flags.set_input_gain(input_gain);
        flags.set_output_gain(output_volume);
        flags.set_noise_suppression(noise_suppression);
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let worker_stop = stop.clone();
        let worker_flags = flags.clone();
        let worker = std::thread::Builder::new()
            .name("zcord-mic-test".into())
            .spawn(move || run_mic_test(input, output, worker_flags, worker_stop, ready_tx))
            .map_err(|error| format!("Microphone test thread: {error}"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                stop,
                flags,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                stop.store(true, Ordering::Release);
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                let _ = worker.join();
                Err("Microphone test stopped before opening the devices".into())
            }
        }
    }

    pub fn level(&self) -> Option<f32> {
        self.flags.input_level()
    }

    pub fn gate_open(&self) -> bool {
        self.flags.gate_open()
    }

    pub fn set_input_gain(&self, gain: f32) {
        self.flags.set_input_gain(gain);
    }

    pub fn set_output_volume(&self, volume: f32) {
        self.flags.set_output_gain(volume);
    }

    pub fn set_input_sensitivity(&self, threshold: Option<f32>) {
        self.flags.set_sensitivity(threshold);
    }

    pub fn set_noise_suppression(&self, enabled: bool) {
        self.flags.set_noise_suppression(enabled);
    }
}

impl Drop for MicTest {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.flags.clear_input_level();
        self.flags.set_gate_open(false);
    }
}

fn run_mic_test(
    input_name: Option<String>,
    output_name: Option<String>,
    flags: AudioFlags,
    stop: Arc<AtomicBool>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let opened = (|| -> Result<(Input, Output, StreamFailure, StreamFailure), String> {
        let host = cpal::default_host();
        let input_device = preferred_device(&host, input_name.as_deref(), true, true)
            .ok_or_else(|| "No microphone is available".to_owned())?;
        let output_device = preferred_device(&host, output_name.as_deref(), false, true)
            .ok_or_else(|| "No speakers are available".to_owned())?;
        let input_failure = StreamFailure::default();
        let output_failure = StreamFailure::default();
        let input = open_input(Some(&input_device), input_failure.clone())
            .map_err(|error| format!("Microphone: {error}"))?
            .ok_or_else(|| "No microphone is available".to_owned())?;
        let output = open_output(Some(&output_device), flags.clone(), output_failure.clone())
            .map_err(|error| format!("Speakers: {error}"))?
            .ok_or_else(|| "No speakers are available".to_owned())?;
        Ok((input, output, input_failure, output_failure))
    })();
    let (mut input, mut output, input_failure, output_failure) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    let _level = LevelGuard(flags.clone());
    let mut capture_48k = Vec::new();
    let mut denoiser = Denoiser::default();
    let mut gate = VoiceGate::default();
    let mut last_xrun: Option<Instant> = None;

    while !stop.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(5));
        let now = Instant::now();
        if let Some((kind, text)) = input_failure.take_notice() {
            if kind == cpal::ErrorKind::Xrun {
                if last_xrun.is_none_or(|last| {
                    now.saturating_duration_since(last) >= Duration::from_secs(10)
                }) {
                    log::debug!("Microphone test xrun: {text}");
                    last_xrun = Some(now);
                }
            } else {
                log::debug!("Microphone test stream notice: {text}");
            }
        }
        if let Some((kind, text)) = output_failure.take_notice() {
            if kind == cpal::ErrorKind::Xrun {
                log::debug!("Microphone test speaker xrun: {text}");
            } else {
                log::debug!("Microphone test speaker notice: {text}");
            }
        }
        if input_failure.take().is_some() || output_failure.take().is_some() {
            break;
        }
        {
            let mut raw = input.queue.lock().unwrap_or_else(PoisonError::into_inner);
            if !raw.is_empty() {
                let samples: Vec<f32> = raw.drain(..).collect();
                drop(raw);
                input.resampler.push(&samples, &mut capture_48k);
            }
        }

        while capture_48k.len() >= FRAME_VALUES {
            let mut frame: Vec<f32> = capture_48k.drain(..FRAME_VALUES).collect();
            apply_gain(&mut frame, flags.input_gain());
            flags.set_input_level(dbfs(&frame));
            let probability = denoiser.process(&mut frame, flags.noise_suppression());
            let open = gate.update_with_threshold(
                &frame,
                probability,
                Instant::now(),
                flags.sensitivity(),
            );
            flags.set_gate_open(open);
            let mut converted = Vec::new();
            output.resampler.push(&frame, &mut converted);
            let gain = flags.output_gain();
            for sample in &mut converted {
                *sample = output_sample(*sample, gain);
            }
            let mut queue = output.queue.lock().unwrap_or_else(PoisonError::into_inner);
            queue.extend(converted);
            let limit = PLAYBACK_LIMIT.max(output.frame_values.saturating_mul(2));
            while queue.len() > limit {
                queue.pop_front();
            }
        }
    }
}

struct StereoResampler {
    input_rate: u32,
    output_rate: u32,
    input_channels: usize,
    output_channels: usize,
    frames: VecDeque<[f32; 2]>,
    base: u64,
    total_in: u64,
    total_out: u64,
}

impl StereoResampler {
    fn new(
        input_rate: u32,
        output_rate: u32,
        input_channels: usize,
        output_channels: usize,
    ) -> Self {
        Self {
            input_rate,
            output_rate,
            input_channels: input_channels.max(1),
            output_channels: output_channels.max(1),
            frames: VecDeque::new(),
            base: 0,
            total_in: 0,
            total_out: 0,
        }
    }

    fn push(&mut self, input: &[f32], output: &mut Vec<f32>) {
        for frame in input.chunks(self.input_channels) {
            let left = frame[0];
            let right = frame.get(1).copied().unwrap_or(left);
            self.frames.push_back([left, right]);
        }
        self.total_in += (input.len() / self.input_channels) as u64;
        let target = (self.total_in as f64 * f64::from(self.output_rate)
            / f64::from(self.input_rate))
        .round() as u64;
        while self.total_out < target {
            let source =
                self.total_out as f64 * f64::from(self.input_rate) / f64::from(self.output_rate);
            let index = source.floor() as u64;
            let fraction = (source - index as f64) as f32;
            let relative = index.saturating_sub(self.base) as usize;
            if relative >= self.frames.len() {
                break;
            }
            let first = self.frames[relative];
            let second = self.frames.get(relative + 1).copied().unwrap_or(first);
            let left = first[0] + (second[0] - first[0]) * fraction;
            let right = first[1] + (second[1] - first[1]) * fraction;
            push_frame(output, left, right, self.output_channels);
            self.total_out += 1;
        }
        let next = (self.total_out as f64 * f64::from(self.input_rate)
            / f64::from(self.output_rate))
        .floor() as u64;
        while self.base < next && !self.frames.is_empty() {
            self.frames.pop_front();
            self.base += 1;
        }
    }
}

fn push_frame(output: &mut Vec<f32>, left: f32, right: f32, channels: usize) {
    match channels {
        1 => output.push((left + right) * 0.5),
        2 => {
            output.push(left);
            output.push(right);
        }
        channels => {
            output.push(left);
            output.push(right);
            output.resize(output.len() + channels - 2, 0.0);
        }
    }
}

fn build_input(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
    failure: StreamFailure,
) -> Result<Stream, cpal::Error> {
    let stream_config = config.config();
    match config.sample_format() {
        SampleFormat::F32 => build_input_typed::<f32>(device, stream_config, queue, failure),
        SampleFormat::F64 => build_input_typed::<f64>(device, stream_config, queue, failure),
        SampleFormat::I16 => build_input_typed::<i16>(device, stream_config, queue, failure),
        SampleFormat::I32 => build_input_typed::<i32>(device, stream_config, queue, failure),
        SampleFormat::I64 => build_input_typed::<i64>(device, stream_config, queue, failure),
        SampleFormat::I8 => build_input_typed::<i8>(device, stream_config, queue, failure),
        SampleFormat::U16 => build_input_typed::<u16>(device, stream_config, queue, failure),
        SampleFormat::U32 => build_input_typed::<u32>(device, stream_config, queue, failure),
        SampleFormat::U64 => build_input_typed::<u64>(device, stream_config, queue, failure),
        SampleFormat::U8 => build_input_typed::<u8>(device, stream_config, queue, failure),
        _ => Err(cpal::Error::from(cpal::ErrorKind::UnsupportedConfig)),
    }
}

fn queue_input_samples<T>(queue: &Mutex<VecDeque<f32>>, data: &[T])
where
    T: Sample,
    f32: FromSample<T>,
{
    // Preserve the device rate and channel layout; the input worker resamples once.
    queue
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .extend(data.iter().map(|sample| f32::from_sample(*sample)));
}

fn build_input_typed<T>(
    device: &cpal::Device,
    config: StreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
    failure: StreamFailure,
) -> Result<Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            queue_input_samples(&queue, data);
        },
        move |error| {
            failure.report(error);
        },
        None,
    )
}

fn build_output(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
    flags: AudioFlags,
    failure: StreamFailure,
    underruns: Arc<AtomicU64>,
) -> Result<Stream, cpal::Error> {
    let channels = usize::from(config.channels());
    let stream_config = config.config();
    match config.sample_format() {
        SampleFormat::F32 => build_output_typed::<f32>(
            device,
            stream_config,
            queue,
            channels,
            flags,
            failure,
            underruns,
        ),
        SampleFormat::F64 => build_output_typed::<f64>(
            device,
            stream_config,
            queue,
            channels,
            flags,
            failure,
            underruns,
        ),
        SampleFormat::I16 => build_output_typed::<i16>(
            device,
            stream_config,
            queue,
            channels,
            flags,
            failure,
            underruns,
        ),
        SampleFormat::I32 => build_output_typed::<i32>(
            device,
            stream_config,
            queue,
            channels,
            flags,
            failure,
            underruns,
        ),
        SampleFormat::I64 => build_output_typed::<i64>(
            device,
            stream_config,
            queue,
            channels,
            flags,
            failure,
            underruns,
        ),
        SampleFormat::I8 => build_output_typed::<i8>(
            device,
            stream_config,
            queue,
            channels,
            flags,
            failure,
            underruns,
        ),
        SampleFormat::U16 => build_output_typed::<u16>(
            device,
            stream_config,
            queue,
            channels,
            flags,
            failure,
            underruns,
        ),
        SampleFormat::U32 => build_output_typed::<u32>(
            device,
            stream_config,
            queue,
            channels,
            flags,
            failure,
            underruns,
        ),
        SampleFormat::U64 => build_output_typed::<u64>(
            device,
            stream_config,
            queue,
            channels,
            flags,
            failure,
            underruns,
        ),
        SampleFormat::U8 => build_output_typed::<u8>(
            device,
            stream_config,
            queue,
            channels,
            flags,
            failure,
            underruns,
        ),
        _ => Err(cpal::Error::from(cpal::ErrorKind::UnsupportedConfig)),
    }
}

fn build_output_typed<T>(
    device: &cpal::Device,
    config: StreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
    channels: usize,
    flags: AudioFlags,
    failure: StreamFailure,
    underruns: Arc<AtomicU64>,
) -> Result<Stream, cpal::Error>
where
    T: SizedSample,
    T: FromSample<f32>,
{
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let mut queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
            let deafened = flags.deafened();
            if deafened {
                queue.clear();
            } else if queue.len() < data.len() {
                underruns.fetch_add(1, Ordering::Relaxed);
            }
            for frame in data.chunks_mut(channels) {
                for sample in frame {
                    let value = if deafened {
                        0.0
                    } else {
                        queue.pop_front().unwrap_or(0.0)
                    };
                    *sample = T::from_sample(value);
                }
            }
        },
        move |error| {
            failure.report(error);
        },
        None,
    )
}

#[cfg(test)]
mod device_name_tests {
    use super::split_name;

    #[test]
    fn windows_names_split_into_adapter_and_endpoint() {
        assert_eq!(
            split_name("Headphones (UGREEN HiTune Max5c)"),
            (
                "UGREEN HiTune Max5c".to_owned(),
                Some("Headphones".to_owned())
            )
        );
        assert_eq!(
            split_name("24V2W1G5 (2- High Definition Audio Device)"),
            (
                "2- High Definition Audio Device".to_owned(),
                Some("24V2W1G5".to_owned())
            )
        );
        assert_eq!(
            split_name("Digital Audio (S/PDIF) (High Definition Audio Device)"),
            (
                "High Definition Audio Device".to_owned(),
                Some("Digital Audio (S/PDIF)".to_owned())
            )
        );
        assert_eq!(split_name("default"), ("default".to_owned(), None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(input: &[f32], rate: u32, channels: usize) -> Vec<f32> {
        let mut resampler = StereoResampler::new(rate, SAMPLE_RATE, channels, 2);
        let mut output = Vec::new();
        resampler.push(input, &mut output);
        output
    }

    fn noise_frame(state: &mut u32) -> Vec<f32> {
        (0..FRAME_VALUES)
            .map(|_| {
                *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let unit = (*state as f64 / u32::MAX as f64) * 2.0 - 1.0;
                (unit * 0.1732) as f32
            })
            .collect()
    }

    fn rms(frame: &[f32]) -> f32 {
        (frame.iter().map(|sample| sample * sample).sum::<f32>() / frame.len() as f32).sqrt()
    }

    #[test]
    fn denoiser_reduces_synthetic_noise_after_warm_up() {
        let mut denoiser = Denoiser::default();
        let mut lcg = 0x1234_5678;
        let mut input_energy = 0.0f64;
        let mut output_energy = 0.0f64;
        let mut frames = 0u32;
        for index in 0..60 {
            let mut frame = noise_frame(&mut lcg);
            if index >= 50 {
                input_energy += f64::from(rms(&frame)).powi(2);
            }
            let _ = denoiser.process(&mut frame, true);
            if index >= 50 {
                output_energy += f64::from(rms(&frame)).powi(2);
                frames += 1;
            }
        }
        let input_rms = (input_energy / f64::from(frames)).sqrt();
        let output_rms = (output_energy / f64::from(frames)).sqrt();
        println!(
            "denoiser RMS: input={input_rms:.6} output={output_rms:.6} ratio={:.3}",
            output_rms / input_rms
        );
        assert!(
            output_rms < 0.6 * input_rms,
            "input rms {input_rms:.6}, output rms {output_rms:.6}"
        );
    }

    #[test]
    fn denoiser_preserves_frame_length_and_channels() {
        let mut denoiser = Denoiser::default();
        let mut lcg = 0x0BAD_F00D;
        for _ in 0..10 {
            let mut frame = noise_frame(&mut lcg);
            assert!(denoiser.process(&mut frame, true).is_some());
            assert_eq!(frame.len(), FRAME_VALUES);
            assert!(frame.chunks_exact(2).all(|pair| pair[0] == pair[1]));
        }
    }

    #[test]
    fn denoiser_disabled_leaves_frame_unchanged() {
        let mut denoiser = Denoiser::default();
        let mut lcg = 0xCAFE_BABE;
        let mut frame = noise_frame(&mut lcg);
        let original = frame.clone();
        assert_eq!(denoiser.process(&mut frame, false), None);
        assert_eq!(frame, original);
    }

    #[test]
    fn microphone_queue_keeps_device_samples_for_one_worker_resample() {
        for rate in [44_100, 48_000, 96_000] {
            for channels in [1, 2] {
                let input_frames = rate as usize / 50;
                let input: Vec<i16> = (0..input_frames * channels)
                    .map(|index| ((index % 200) as i16 - 100) * 100)
                    .collect();
                let queue = Mutex::new(VecDeque::new());
                for chunk in input.chunks(channels * 147) {
                    queue_input_samples(&queue, chunk);
                }
                let samples: Vec<_> = queue.lock().unwrap().drain(..).collect();
                let expected: Vec<_> = input
                    .iter()
                    .map(|sample| f32::from_sample(*sample))
                    .collect();
                assert_eq!(samples, expected, "rate={rate} channels={channels}");

                let output = capture(&samples, rate, channels);
                assert_eq!(
                    output.len(),
                    FRAME_VALUES,
                    "rate={rate} channels={channels}"
                );
                if channels == 1 {
                    assert!(output.chunks_exact(2).all(|frame| frame[0] == frame[1]));
                }
            }
        }
    }

    #[test]
    fn only_recoverable_cpal_errors_are_not_fatal() {
        for kind in [
            cpal::ErrorKind::Xrun,
            cpal::ErrorKind::DeviceChanged,
            cpal::ErrorKind::RealtimeDenied,
        ] {
            assert!(!is_fatal(kind));
        }
        for kind in [
            cpal::ErrorKind::StreamInvalidated,
            cpal::ErrorKind::DeviceNotAvailable,
            cpal::ErrorKind::HostUnavailable,
            cpal::ErrorKind::UnsupportedConfig,
            cpal::ErrorKind::Other,
        ] {
            assert!(is_fatal(kind));
        }
    }

    #[test]
    fn stream_failure_separates_fatal_errors_from_notices() {
        let failure = StreamFailure::default();
        assert!(failure.take().is_none());
        assert!(failure.take_notice().is_none());

        let error = cpal::Error::from(cpal::ErrorKind::Xrun);
        let text = error.to_string();
        failure.report(error);
        assert!(failure.take().is_none());
        let (kind, notice) = failure.take_notice().expect("non-fatal notice");
        assert_eq!(kind, cpal::ErrorKind::Xrun);
        assert_eq!(notice, text);

        let error = cpal::Error::from(cpal::ErrorKind::DeviceNotAvailable);
        let text = error.to_string();
        failure.report(error);
        assert_eq!(failure.take(), Some(text));
        assert!(failure.take_notice().is_none());
    }

    #[test]
    fn retry_backoff_and_warning_throttle_are_capped() {
        for (failures, seconds) in [(1, 1), (2, 2), (3, 4), (4, 8), (5, 16), (6, 30), (7, 30)] {
            assert_eq!(retry_delay(failures), Duration::from_secs(seconds));
        }
        let start = Instant::now();
        assert!(failure_warning_due(start, None));
        assert!(!failure_warning_due(
            start + Duration::from_secs(29),
            Some(start)
        ));
        assert!(failure_warning_due(
            start + Duration::from_secs(30),
            Some(start)
        ));
        assert!(failure_warning_due(
            start + Duration::from_secs(60),
            Some(start)
        ));
    }

    #[test]
    fn forty_four_one_mono_becomes_forty_eight_stereo_frames() {
        let input: Vec<f32> = (0..44_100).map(|index| index as f32 / 44_100.0).collect();
        let output = capture(&input, 44_100, 1);
        assert_eq!(output.len(), 48_000 * 2);
        for frame in output.chunks_exact(2) {
            assert_eq!(frame[0], frame[1]);
        }
        let first = output[0];
        let last = *output.last().unwrap();
        assert!(last > first);
    }

    #[test]
    fn forty_eight_stereo_passes_through_unchanged() {
        let input: Vec<f32> = (0..1920).map(|index| index as f32).collect();
        assert_eq!(capture(&input, 48_000, 2), input);
        let output = capture(&input, 48_000, 1);
        assert_eq!(output.len(), input.len() * 2);
        for pair in output.chunks_exact(2) {
            assert_eq!(pair[0], pair[1]);
        }
    }

    #[test]
    fn vad_opens_and_hangs_over() {
        let start = Instant::now();
        let loud = vec![0.01; FRAME_VALUES];
        let quiet = vec![0.0001; FRAME_VALUES];
        let mut gate = VoiceGate::default();
        assert!(!gate.update(&quiet, None, start));
        assert!(gate.update(&loud, None, start));
        assert!(gate.update(&quiet, None, start + Duration::from_millis(200)));
        assert!(!gate.update(&quiet, None, start + Duration::from_millis(301)));
        gate.close();
        assert!(!gate.update(&quiet, None, start + Duration::from_millis(400)));
    }

    #[test]
    fn voice_probability_closes_gate_on_loud_frames() {
        let start = Instant::now();
        let loud = vec![0.01; FRAME_VALUES];
        let mut gate = VoiceGate::default();
        assert!(!gate.update(&loud, Some(0.0), start));
        assert!(gate.update(&loud, Some(VOICE_PROBABILITY_THRESHOLD), start));
        assert!(gate.update(&loud, Some(0.0), start + Duration::from_millis(299)));
        assert!(!gate.update(&loud, Some(0.0), start + Duration::from_millis(300)));

        let mut gate = VoiceGate::default();
        assert!(gate.update(&loud, None, start));
    }

    #[test]
    fn playback_pacing_targets_three_queued_frames() {
        let frame = frame_values(48_000, 2);
        assert_eq!(frame, 1_920);
        assert_eq!(playback_target(frame), 5_760);
        assert!(needs_mix(0, playback_target(frame)));
        assert!(needs_mix(5_759, playback_target(frame)));
        assert!(!needs_mix(5_760, playback_target(frame)));
    }

    #[test]
    fn stopping_sends_five_silence_frames_before_speaking_off() {
        let sequence = stop_sequence();
        assert_eq!(sequence.len(), 6);
        for frame in &sequence[..5] {
            assert!(matches!(frame, Outgoing::Frame(data) if data == &[0xF8, 0xFF, 0xFE]));
        }
        assert!(matches!(sequence[5], Outgoing::Speaking(false)));
    }

    #[test]
    fn gain_and_output_volume_multiply_and_clamp() {
        let mut frame = vec![0.25, -0.5, 2.0];
        apply_gain(&mut frame, 2.0);
        assert_eq!(frame, vec![0.5, -1.0, 1.0]);
        assert_eq!(output_sample(0.25, 2.0), 0.5);
        assert_eq!(output_sample(-0.75, 2.0), -1.0);
    }

    #[test]
    fn volume_and_sensitivity_flags_clamp() {
        let flags = AudioFlags::new();
        flags.set_input_gain(9.0);
        assert_eq!(flags.input_gain(), 2.0);
        flags.set_output_gain(-1.0);
        assert_eq!(flags.output_gain(), 0.0);
        flags.set_sensitivity(Some(12.0));
        assert_eq!(flags.sensitivity(), Some(0.0));
        flags.set_sensitivity(Some(-150.0));
        assert_eq!(flags.sensitivity(), Some(-100.0));
        flags.set_sensitivity(None);
        assert_eq!(flags.sensitivity(), None);
    }

    #[test]
    fn input_level_is_dbfs_and_silence_clamps_to_minus_100() {
        let flags = AudioFlags::new();
        assert_eq!(flags.input_level(), None);
        let frame = vec![0.5; FRAME_VALUES];
        let level = dbfs(&frame);
        assert!((level - 20.0 * 0.5f32.log10()).abs() < 1e-5);
        flags.set_input_level(level);
        assert_eq!(flags.input_level(), Some(level));
        assert_eq!(dbfs(&vec![0.0; FRAME_VALUES]), -100.0);
        flags.clear_input_level();
        assert_eq!(flags.input_level(), None);
    }

    #[test]
    fn configured_sensitivity_replaces_the_default_threshold() {
        let start = Instant::now();
        let loud = vec![0.01; FRAME_VALUES];
        let mut gate = VoiceGate::default();
        assert!(!gate.update_with_threshold(&loud, None, start, Some(-30.0)));
        assert!(gate.update_with_threshold(&loud, None, start, Some(-45.0)));

        let mut gate = VoiceGate::default();
        assert!(gate.update_with_threshold(&loud, None, start, None));
    }

    #[test]
    fn device_pick_requires_an_exact_name() {
        let names = vec!["Microphone A".to_owned(), "Microphone B".to_owned()];
        assert_eq!(pick(&names, Some("Microphone B")), Some(1));
        assert_eq!(pick(&names, Some("Microphone C")), None);
        assert_eq!(pick(&names, None), None);
    }

    #[test]
    fn stream_volume_clamps_and_ignores_non_finite_gain() {
        let flags = AudioFlags::new();
        assert_eq!(flags.stream_volume(), 1.0);
        flags.set_stream_volume(3.0);
        assert_eq!(flags.stream_volume(), 2.0);
        flags.set_stream_volume(-1.0);
        assert_eq!(flags.stream_volume(), 0.0);
        flags.set_stream_volume(0.5);
        flags.set_stream_volume(f32::NAN);
        assert_eq!(flags.stream_volume(), 0.5);
        flags.set_stream_volume(f32::INFINITY);
        assert_eq!(flags.stream_volume(), 0.5);
    }

    #[test]
    fn stream_audio_queue_is_bounded_and_cleared() {
        let flags = AudioFlags::new();
        let generation = flags.begin_stream_audio();
        for sequence in 0..STREAM_AUDIO_LIMIT as u16 + 4 {
            flags.push_stream_audio(generation, sequence, vec![sequence as u8]);
        }
        let mut packets = Vec::new();
        flags.take_stream_audio(&mut packets);
        assert_eq!(packets.len(), STREAM_AUDIO_LIMIT);
        assert_eq!(packets.first().unwrap().sequence, 4);
        assert_eq!(
            packets.last().unwrap().sequence,
            STREAM_AUDIO_LIMIT as u16 + 3
        );

        let next = flags.begin_stream_audio();
        flags.push_stream_audio(generation, 1, vec![1]);
        flags.push_stream_audio(next, 1, vec![1]);
        let mut packets = Vec::new();
        flags.take_stream_audio(&mut packets);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].sequence, 1);
        flags.clear_stream_audio();
        assert_eq!(flags.stream_generation(), next + 1);
        let mut packets = Vec::new();
        flags.take_stream_audio(&mut packets);
        assert!(packets.is_empty());
    }

    #[test]
    fn stream_gain_applies_to_mixed_samples() {
        let mut mixed = vec![0.25, 0.0];
        add_pcm(&mut mixed, &[i16::MAX, -i16::MAX], 0.5);
        assert!((mixed[0] - 0.75).abs() < 1e-6);
        assert!((mixed[1] + 0.5).abs() < 1e-6);
    }

    #[test]
    fn stream_audio_is_audible_and_deafen_silences_it() {
        let mut encoder = Encoder::new(SAMPLE_RATE, Channels::Stereo, Application::Audio).unwrap();
        let pcm = vec![8_000i16; FRAME_VALUES];
        let opus = encoder.encode_vec(&pcm, MAX_OPUS_PAYLOAD).unwrap();
        let now = Instant::now();
        let mut decoded = 0;
        let mut audible = 0;
        let mut state = DecoderState::new();
        state.jitter.push(1, opus.clone());
        let mixed = mix(
            &mut HashMap::new(),
            &mut Some(state),
            0.5,
            false,
            now,
            &mut decoded,
            &mut audible,
        );
        assert!(decoded > 0);
        assert!(audible > 0);
        assert!(mixed.iter().any(|sample| *sample != 0.0));

        let mut state = DecoderState::new();
        state.jitter.push(1, opus);
        decoded = 0;
        audible = 0;
        let mixed = mix(
            &mut HashMap::new(),
            &mut Some(state),
            0.5,
            true,
            now,
            &mut decoded,
            &mut audible,
        );
        assert!(mixed.iter().all(|sample| *sample == 0.0));
    }
}
