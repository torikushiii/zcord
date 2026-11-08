use super::devices::pick;
use super::pipeline::{
    DecoderState, Denoiser, add_pcm, apply_gain, dbfs, mix, needs_mix, output_sample,
    playback_target,
};
use super::streams::queue_input_samples;
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
