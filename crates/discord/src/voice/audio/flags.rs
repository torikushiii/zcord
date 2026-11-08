use super::devices::store_device;
use super::*;

pub(in crate::voice) struct Flags {
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
pub(crate) struct AudioFlags(pub(super) Arc<Flags>);

pub(super) struct StreamPacket {
    pub(super) sequence: u16,
    pub(super) opus: Vec<u8>,
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

    pub(super) fn muted(&self) -> bool {
        self.0.mute.load(Ordering::Relaxed)
    }

    pub(super) fn deafened(&self) -> bool {
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

    pub(super) fn input_gain(&self) -> f32 {
        load_f32(&self.0.input_gain)
    }

    pub fn set_output_gain(&self, gain: f32) {
        store_f32(&self.0.output_gain, gain.clamp(0.0, 2.0));
    }

    pub(super) fn output_gain(&self) -> f32 {
        load_f32(&self.0.output_gain)
    }

    pub fn set_stream_volume(&self, gain: f32) {
        if gain.is_finite() {
            store_f32(&self.0.stream_volume, gain.clamp(0.0, 2.0));
        }
    }

    pub(super) fn stream_volume(&self) -> f32 {
        load_f32(&self.0.stream_volume).clamp(0.0, 2.0)
    }

    pub(in crate::voice) fn begin_stream_audio(&self) -> u64 {
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

    pub(in crate::voice) fn push_stream_audio(
        &self,
        generation: u64,
        sequence: u16,
        opus: Vec<u8>,
    ) {
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

    pub(super) fn take_stream_audio(&self, packets: &mut Vec<StreamPacket>) {
        let mut queue = self
            .0
            .stream_audio
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        packets.extend(queue.drain(..));
    }

    pub(in crate::voice) fn clear_stream_audio(&self) {
        let mut queue = self
            .0
            .stream_audio
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        queue.clear();
        self.0.stream_generation.fetch_add(1, Ordering::AcqRel);
    }

    pub(super) fn stream_generation(&self) -> u64 {
        self.0.stream_generation.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(in crate::voice) fn queued_stream_audio(&self) -> usize {
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

    pub(super) fn sensitivity(&self) -> Option<f32> {
        let value = load_f32(&self.0.sensitivity);
        (!value.is_nan()).then_some(value)
    }

    pub(super) fn set_input_level(&self, db: f32) {
        store_f32(&self.0.input_level, db);
    }

    pub(super) fn clear_input_level(&self) {
        store_f32(&self.0.input_level, f32::NAN);
    }

    pub fn input_level(&self) -> Option<f32> {
        let value = load_f32(&self.0.input_level);
        (!value.is_nan()).then_some(value)
    }

    pub(super) fn set_gate_open(&self, open: bool) {
        self.0.gate_open.store(open, Ordering::Relaxed);
    }

    pub fn gate_open(&self) -> bool {
        self.0.gate_open.load(Ordering::Relaxed)
    }

    pub fn set_input_device(&self, name: Option<String>) {
        store_device(&self.0.input_device, &self.0.input_generation, name);
    }

    pub(super) fn input_device(&self) -> Option<String> {
        self.0
            .input_device
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn input_generation(&self) -> u64 {
        self.0.input_generation.load(Ordering::Acquire)
    }

    pub fn set_output_device(&self, name: Option<String>) {
        store_device(&self.0.output_device, &self.0.output_generation, name);
    }

    pub(super) fn output_device(&self) -> Option<String> {
        self.0
            .output_device
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn output_generation(&self) -> u64 {
        self.0.output_generation.load(Ordering::Acquire)
    }
}

fn store_f32(atom: &AtomicU32, value: f32) {
    atom.store(value.to_bits(), Ordering::Relaxed);
}

fn load_f32(atom: &AtomicU32) -> f32 {
    f32::from_bits(atom.load(Ordering::Relaxed))
}
