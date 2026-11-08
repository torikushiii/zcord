use super::devices::preferred_device;
use super::*;

#[derive(Default)]
pub(super) struct Denoiser {
    state: Option<Box<DenoiseState<'static>>>,
    primed: bool,
}

impl Denoiser {
    pub(super) fn process(&mut self, frame: &mut [f32], enabled: bool) -> Option<f32> {
        if !enabled {
            self.state = None;
            self.primed = false;
            return None;
        }

        if self.state.is_none() {
            self.state = Some(DenoiseState::new());
            self.primed = false;
        }
        let state = self.state.as_mut()?;
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

pub(super) struct LevelGuard(pub(super) AudioFlags);

impl Drop for LevelGuard {
    fn drop(&mut self) {
        self.0.clear_input_level();
        self.0.set_gate_open(false);
    }
}

pub(super) fn apply_gain(frame: &mut [f32], gain: f32) {
    for sample in frame {
        *sample = (*sample * gain).clamp(-1.0, 1.0);
    }
}

pub(super) fn output_sample(sample: f32, gain: f32) -> f32 {
    (sample * gain).clamp(-1.0, 1.0)
}

pub(super) fn dbfs(frame: &[f32]) -> f32 {
    let rms = (frame.iter().map(|sample| sample * sample).sum::<f32>() / frame.len().max(1) as f32)
        .sqrt();
    if rms <= 1e-7 {
        -100.0
    } else {
        (20.0 * rms.log10()).max(-100.0)
    }
}

pub(super) fn run_input(
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

pub(super) fn run_output(
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

pub(super) fn frame_values(rate: u32, channels: usize) -> usize {
    rate as usize / 50 * channels
}

pub(super) fn playback_target(frame_values: usize) -> usize {
    frame_values.saturating_mul(3)
}

pub(super) fn needs_mix(queued: usize, target: usize) -> bool {
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

pub(super) fn add_pcm(mix: &mut [f32], pcm: &[i16], gain: f32) {
    for (index, sample) in pcm.iter().take(mix.len()).enumerate() {
        mix[index] += f32::from(*sample) / f32::from(i16::MAX) * gain;
    }
}

pub(super) fn mix(
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

pub(super) struct DecoderState {
    decoder: Option<Decoder>,
    pub(super) jitter: JitterBuffer,
}

impl DecoderState {
    pub(super) fn new() -> Self {
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
pub(super) struct JitterBuffer {
    next: Option<u16>,
    frames: BTreeMap<u16, Vec<u8>>,
    last_arrival: Option<Instant>,
}

impl JitterBuffer {
    pub(super) fn push(&mut self, sequence: u16, packet: Vec<u8>) {
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
pub(in crate::voice) struct VoiceGate {
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

pub(in crate::voice) fn stop_sequence() -> [Outgoing; 6] {
    [
        Outgoing::Frame(crate::voice::dave::SILENCE.to_vec()),
        Outgoing::Frame(crate::voice::dave::SILENCE.to_vec()),
        Outgoing::Frame(crate::voice::dave::SILENCE.to_vec()),
        Outgoing::Frame(crate::voice::dave::SILENCE.to_vec()),
        Outgoing::Frame(crate::voice::dave::SILENCE.to_vec()),
        Outgoing::Speaking(false),
    ]
}
