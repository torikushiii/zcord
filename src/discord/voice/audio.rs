use cpal::{
    FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use opus::{Application, Bitrate, Channels, Decoder, Encoder};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
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
const VAD_HANGOVER: Duration = Duration::from_millis(300);
const PLAYBACK_LIMIT: usize = FRAME_VALUES * 10;
const MAX_OPUS_PAYLOAD: usize = 1275;

pub(super) struct Flags {
    pub mute: AtomicBool,
    pub deaf: AtomicBool,
}

#[derive(Clone)]
pub(super) struct AudioFlags(Arc<Flags>);

impl AudioFlags {
    pub fn new() -> Self {
        Self(Arc::new(Flags {
            mute: AtomicBool::new(false),
            deaf: AtomicBool::new(false),
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
            started: Arc::new(AtomicBool::new(false)),
            active: Arc::new(AtomicBool::new(false)),
            started_at: Instant::now(),
            logged_running: false,
            failures: 0,
            next_attempt: Instant::now(),
            last_warn: None,
        }
    }

    fn begin_spawn(&mut self) -> (Arc<AtomicBool>, Arc<AtomicBool>) {
        self.started = Arc::new(AtomicBool::new(false));
        self.active = Arc::new(AtomicBool::new(false));
        self.started_at = Instant::now();
        self.logged_running = false;
        (self.started.clone(), self.active.clone())
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
        cpal::ErrorKind::Xrun
            | cpal::ErrorKind::DeviceChanged
            | cpal::ErrorKind::RealtimeDenied
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
            let mut current = self
                .message
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *current = Some(error.to_string());
            self.failed.store(true, Ordering::Release);
        } else {
            let mut notice = self
                .notice
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
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

fn input_rebuild_needed(
    stream_error: bool,
    has_input: bool,
    input_available: bool,
) -> bool {
    stream_error || has_input != input_available
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
        let now = Instant::now();
        if self.input.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            match self.input.worker.take().and_then(|worker| worker.join().ok()) {
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
        let now = Instant::now();
        if self.output.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            match self.output.worker.take().and_then(|worker| worker.join().ok()) {
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

    fn reset_incoming(&mut self) {
        let (incoming, incoming_rx) = unbounded_channel();
        self.incoming = incoming;
        self.incoming_rx = Some(incoming_rx);
    }

    fn spawn_input(&mut self) {
        let (started, active) = self.input.begin_spawn();
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
                    outgoing,
                    flags,
                    stop,
                    started,
                    active,
                    mic_frames,
                    gate_open,
                    encoded,
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
        let (started, active) = self.output.begin_spawn();
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
                    Some(incoming) => run_output(incoming, flags, stop, started, active),
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
                self.incoming_rx = holder
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                self.output
                    .failed(Instant::now(), format!("thread: {error}"));
            }
        }
    }
}

impl Drop for AudioHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
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
    stream
        .play()
        .map_err(|error| format!("start: {error}"))?;
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
    stream
        .play()
        .map_err(|error| format!("start: {error}"))?;
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

fn run_input(
    outgoing: UnboundedSender<Outgoing>,
    flags: AudioFlags,
    stop: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    mic_frames: Arc<AtomicU64>,
    gate_open: Arc<AtomicU64>,
    encoded: Arc<AtomicU64>,
) -> SideExit {
    let host = cpal::default_host();
    let failure = StreamFailure::default();
    let device = host.default_input_device();
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
    let mut gate = VoiceGate::default();
    let mut speaking = false;
    let mut next_input_check = Instant::now() + Duration::from_secs(3);
    let mut last_xrun: Option<Instant> = None;

    while !stop.load(Ordering::Acquire) {
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
            let frame: Vec<f32> = capture_48k.drain(..FRAME_VALUES).collect();
            mic_frames.fetch_add(1, Ordering::Relaxed);
            let muted = flags.muted() || flags.deafened();
            let open = if muted {
                gate.close();
                false
            } else {
                gate.update(&frame, Instant::now())
            };
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

        let now = Instant::now();
        if now >= next_input_check {
            next_input_check = now + Duration::from_secs(3);
            let available = host
                .default_input_device()
                .is_some_and(|device| device.default_input_config().is_ok());
            if input_rebuild_needed(false, input.is_some(), available) {
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
    started: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
) -> SideExit {
    let host = cpal::default_host();
    let failure = StreamFailure::default();
    let device = host.default_output_device();
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
    let mut packets = 0u64;
    let mut decoded = 0u64;
    let mut audible = 0u64;
    let mut last_underruns = 0u64;
    let mut next_report = Instant::now() + Duration::from_secs(5);
    let mut last_xrun: Option<Instant> = None;

    while !stop.load(Ordering::Acquire) {
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

        if let Some(reason) = failure.take() {
            return SideExit {
                incoming: Some(incoming),
                failure: Some(reason),
                stopped: false,
            };
        }

        let now = Instant::now();
        decoders.retain(|_, state| state.jitter.fresh(now));
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
                let mixed = mix(
                    &mut decoders,
                    flags.deafened(),
                    now,
                    &mut decoded,
                    &mut audible,
                );
                let mut converted = Vec::new();
                output.resampler.push(&mixed, &mut converted);
                let mut queue = output
                    .queue
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
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

fn mix(
    decoders: &mut HashMap<u32, DecoderState>,
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
        return mix;
    }
    for state in decoders.values_mut() {
        let pcm = match state.jitter.take(now) {
            Ready::Frame(packet) => decode(&mut state.decoder, &packet),
            Ready::Gap => decode(&mut state.decoder, &[]),
            Ready::Wait => None,
        };
        let Some(pcm) = pcm else {
            continue;
        };
        *decoded = decoded.saturating_add(1);
        for (index, sample) in pcm.iter().take(FRAME_VALUES).enumerate() {
            mix[index] += f32::from(*sample) / f32::from(i16::MAX);
        }
    }
    for sample in &mut mix {
        *sample = sample.clamp(-1.0, 1.0);
    }
    if mix.iter().any(|sample| *sample != 0.0) {
        *audible = audible.saturating_add(1);
    }
    mix
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
    pub fn update(&mut self, frame: &[f32], now: Instant) -> bool {
        let rms = (frame.iter().map(|sample| sample * sample).sum::<f32>()
            / frame.len().max(1) as f32)
            .sqrt();
        let db = if rms <= 1e-7 {
            f32::NEG_INFINITY
        } else {
            20.0 * rms.log10()
        };
        if db >= VAD_THRESHOLD_DB {
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
mod tests {
    use super::*;

    fn capture(input: &[f32], rate: u32, channels: usize) -> Vec<f32> {
        let mut resampler = StereoResampler::new(rate, SAMPLE_RATE, channels, 2);
        let mut output = Vec::new();
        resampler.push(input, &mut output);
        output
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
    fn input_rebuild_uses_stream_errors_and_input_availability() {
        assert!(!input_rebuild_needed(false, false, false));
        assert!(!input_rebuild_needed(false, true, true));
        assert!(input_rebuild_needed(true, false, false));
        assert!(input_rebuild_needed(true, true, true));
        assert!(input_rebuild_needed(false, false, true));
        assert!(input_rebuild_needed(false, true, false));
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
        assert!(!gate.update(&quiet, start));
        assert!(gate.update(&loud, start));
        assert!(gate.update(&quiet, start + Duration::from_millis(200)));
        assert!(!gate.update(&quiet, start + Duration::from_millis(301)));
        gate.close();
        assert!(!gate.update(&quiet, start + Duration::from_millis(400)));
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
}
