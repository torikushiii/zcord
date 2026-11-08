use super::devices::preferred_device;
use super::pipeline::{Denoiser, LevelGuard, apply_gain, dbfs, output_sample};
use super::*;

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
