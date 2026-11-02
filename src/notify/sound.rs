use std::{
    f32::consts::TAU,
    fs,
    io::{Cursor, ErrorKind},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use cpal::{
    FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use futures::StreamExt;
use sha2::{Digest, Sha256};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as SymphoniaError,
    formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

const RATE: u32 = 48_000;
const CHIME_MS: u32 = 180;
const MIN_INTERVAL: Duration = Duration::from_secs(2);
const DISCORD_URL: &str = "https://discord.com/assets/3ed22d14f3c30bc4.mp3";
const DISCORD_SHA256: &str = "31ad0482eee7770597b8aa723a80fd041ade0b076679b12293664f1f1777211b";
const MAX_DOWNLOAD: usize = 512 * 1024;

struct Pcm {
    samples: Vec<f32>,
    channels: usize,
    rate: u32,
}

struct Gate {
    last_started: Option<Instant>,
    playing: bool,
}

static GATE: Mutex<Gate> = Mutex::new(Gate {
    last_started: None,
    playing: false,
});
static DISCORD: OnceLock<Option<Arc<Pcm>>> = OnceLock::new();
static DOWNLOAD_STARTED: AtomicBool = AtomicBool::new(false);

pub fn play(volume: f32) {
    let volume = volume.clamp(0.0, 1.0);
    if !claim_playback(Instant::now()) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("zcord-notification-sound".into())
        .spawn(move || {
            let _playing = PlayingGuard;
            let clip = discord_clip().unwrap_or_else(|| Arc::new(chime()));
            if let Err(error) = play_clip(&clip, volume) {
                log::warn!("could not play notification sound: {error}");
            }
        });
    if let Err(error) = spawned {
        release_playback();
        log::warn!("could not start notification sound thread: {error}");
    }
}

fn claim_playback(now: Instant) -> bool {
    let mut gate = GATE.lock().unwrap_or_else(|error| error.into_inner());
    let elapsed = gate
        .last_started
        .map(|last| now.saturating_duration_since(last));
    if gate.playing || !rate_limit_allows(elapsed) {
        return false;
    }
    gate.playing = true;
    gate.last_started = Some(now);
    true
}

fn release_playback() {
    GATE.lock()
        .unwrap_or_else(|error| error.into_inner())
        .playing = false;
}

fn rate_limit_allows(elapsed: Option<Duration>) -> bool {
    elapsed.is_none_or(|elapsed| elapsed >= MIN_INTERVAL)
}

struct PlayingGuard;

impl Drop for PlayingGuard {
    fn drop(&mut self) {
        release_playback();
    }
}

fn discord_path() -> Option<PathBuf> {
    Some(
        dirs::cache_dir()?
            .join("zcord")
            .join("sounds")
            .join("message.mp3"),
    )
}

fn discord_clip() -> Option<Arc<Pcm>> {
    if let Some(clip) = DISCORD.get() {
        return clip.clone();
    }
    let Some(path) = discord_path() else {
        log::warn!("could not locate the notification sound cache folder");
        return None;
    };
    if !path.is_file() {
        start_download(&path);
        return None;
    }
    let clip = fs::read(&path)
        .ok()
        .and_then(|bytes| decode_mp3(&bytes))
        .map(Arc::new);
    if clip.is_none() {
        log::warn!("could not decode cached notification sound");
    }
    let _ = DISCORD.set(clip.clone());
    clip
}

fn start_download(path: &Path) {
    if DOWNLOAD_STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let path = path.to_path_buf();
    let spawned = std::thread::Builder::new()
        .name("zcord-sound-download".into())
        .spawn(move || match download(&path) {
            Ok(bytes) => {
                let clip = decode_mp3(&bytes).map(Arc::new);
                if clip.is_none() {
                    log::warn!("could not decode downloaded notification sound");
                }
                let _ = DISCORD.set(clip);
            }
            Err(error) => log::warn!("could not download notification sound: {error}"),
        });
    if let Err(error) = spawned {
        log::warn!("could not start notification sound download: {error}");
    }
}

fn download(path: &Path) -> Result<Vec<u8>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(fetch(path))
}

async fn fetch(path: &Path) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(DISCORD_URL)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOWNLOAD as u64)
    {
        return Err("notification sound is larger than the download limit".to_string());
    }

    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        if body.len() + chunk.len() > MAX_DOWNLOAD {
            return Err("notification sound is larger than the download limit".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    if !verify_sha256(&body, DISCORD_SHA256) {
        return Err("notification sound failed its checksum".to_string());
    }

    let folder = path
        .parent()
        .ok_or_else(|| "notification sound has no parent folder".to_string())?;
    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("mp3.part");
    fs::write(&temporary, &body).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        error.to_string()
    })?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(body)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn verify_sha256(bytes: &[u8], expected: &str) -> bool {
    sha256_hex(bytes).eq_ignore_ascii_case(expected)
}

fn decode_mp3(bytes: &[u8]) -> Option<Pcm> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let mut format = probed.format;
    let track = format.default_track()?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;

    let mut samples = Vec::new();
    let mut channels = 0;
    let mut rate = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                break;
            }
            Err(_) => return None,
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(_) => return None,
        };
        let spec = *decoded.spec();
        channels = spec.channels.count();
        rate = spec.rate;
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }

    (channels > 0 && rate > 0 && !samples.is_empty()).then_some(Pcm {
        samples,
        channels,
        rate,
    })
}

fn chime() -> Pcm {
    let frames = (RATE as f32 * CHIME_MS as f32 / 1000.0).round() as usize;
    let half = frames / 2;
    let attack = (RATE as f32 * 0.008).round() as usize;
    let mut samples = Vec::with_capacity(frames);
    for frame in 0..frames {
        let (offset, frequency, length) = if frame < half {
            (0, 880.0, half)
        } else {
            (half, 1174.66, frames - half)
        };
        let index = frame - offset;
        let local = index as f32 / length as f32;
        let attack_gain = if index < attack {
            0.5 - 0.5 * (std::f32::consts::PI * index as f32 / attack as f32).cos()
        } else {
            1.0
        };
        let decay_gain = (1.0 - local).powi(2);
        let time = index as f32 / RATE as f32;
        let wave = (TAU * frequency * time).sin() + 0.3 * (TAU * 2.0 * frequency * time).sin();
        samples.push(wave * attack_gain * decay_gain);
    }
    let peak = samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    if peak > 0.0 {
        let gain = 0.3 / peak;
        for sample in &mut samples {
            *sample *= gain;
        }
    }
    Pcm {
        samples,
        channels: 1,
        rate: RATE,
    }
}

fn resample(samples: &[f32], channels: usize, from_rate: u32, to_rate: u32) -> Vec<f32> {
    if channels == 0 || from_rate == 0 || to_rate == 0 {
        return Vec::new();
    }
    let input_frames = samples.len() / channels;
    if input_frames == 0 {
        return Vec::new();
    }
    if from_rate == to_rate {
        return samples.to_vec();
    }
    let output_frames = (input_frames as f64 * to_rate as f64 / from_rate as f64).round() as usize;
    let step = from_rate as f64 / to_rate as f64;
    let mut output = Vec::with_capacity(output_frames * channels);
    for frame in 0..output_frames {
        let position = frame as f64 * step;
        let start = position.floor() as usize;
        let fraction = (position - start as f64) as f32;
        let start = start.min(input_frames - 1);
        let end = (start + 1).min(input_frames - 1);
        for channel in 0..channels {
            let first = samples[start * channels + channel];
            let second = samples[end * channels + channel];
            output.push(first + (second - first) * fraction);
        }
    }
    output
}

fn remix(samples: &[f32], source_channels: usize, target_channels: usize) -> Vec<f32> {
    if source_channels == 0 || target_channels == 0 {
        return Vec::new();
    }
    let frames = samples.len() / source_channels;
    let mut output = Vec::with_capacity(frames * target_channels);
    for frame in 0..frames {
        let input = &samples[frame * source_channels..(frame + 1) * source_channels];
        for channel in 0..target_channels {
            let value = if target_channels == 1 && source_channels > 1 {
                input.iter().sum::<f32>() / source_channels as f32
            } else if source_channels == 1 {
                input[0]
            } else if channel < source_channels {
                input[channel]
            } else {
                input[0]
            };
            output.push(value);
        }
    }
    output
}

fn play_clip(clip: &Pcm, volume: f32) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no default output device".to_string())?;
    let supported = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let rate = config.sample_rate;
    let channels = config.channels as usize;
    if rate == 0 || channels == 0 {
        return Err("invalid output configuration".to_string());
    }

    let resampled = resample(&clip.samples, clip.channels, clip.rate, rate);
    let output = remix(&resampled, clip.channels, channels);
    let frames = output.len() / channels;
    let stream = match format {
        SampleFormat::I8 => play_stream::<i8>(&device, config, output, channels, volume)?,
        SampleFormat::I16 => play_stream::<i16>(&device, config, output, channels, volume)?,
        SampleFormat::I24 => play_stream::<cpal::I24>(&device, config, output, channels, volume)?,
        SampleFormat::I32 => play_stream::<i32>(&device, config, output, channels, volume)?,
        SampleFormat::I64 => play_stream::<i64>(&device, config, output, channels, volume)?,
        SampleFormat::U8 => play_stream::<u8>(&device, config, output, channels, volume)?,
        SampleFormat::U16 => play_stream::<u16>(&device, config, output, channels, volume)?,
        SampleFormat::U24 => play_stream::<cpal::U24>(&device, config, output, channels, volume)?,
        SampleFormat::U32 => play_stream::<u32>(&device, config, output, channels, volume)?,
        SampleFormat::U64 => play_stream::<u64>(&device, config, output, channels, volume)?,
        SampleFormat::F32 => play_stream::<f32>(&device, config, output, channels, volume)?,
        SampleFormat::F64 => play_stream::<f64>(&device, config, output, channels, volume)?,
        _ => return Err(format!("unsupported output sample format {format:?}")),
    };
    stream.play().map_err(|error| error.to_string())?;
    std::thread::sleep(
        Duration::from_secs_f64(frames as f64 / rate as f64) + Duration::from_millis(100),
    );
    Ok(())
}

fn play_stream<T>(
    device: &cpal::Device,
    config: StreamConfig,
    samples: Vec<f32>,
    channels: usize,
    volume: f32,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let mut cursor = 0;
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                for frame in output.chunks_mut(channels) {
                    for (channel, sample) in frame.iter_mut().enumerate() {
                        let value = samples.get(cursor + channel).copied().unwrap_or(0.0) * volume;
                        *sample = T::from_sample(value);
                    }
                    cursor += channels;
                }
            },
            |error| log::warn!("notification sound stream error: {error}"),
            None,
        )
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chime_is_a_short_soft_two_note_signal() {
        let chime = chime();
        assert_eq!(chime.channels, 1);
        assert_eq!(chime.rate, 48_000);
        assert_eq!(chime.samples.len(), 8_640);
        let peak = chime
            .samples
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!((peak - 0.3).abs() < 1e-5);
        assert!(chime.samples.iter().all(|sample| sample.abs() <= 1.0));
        assert!(chime.samples[0].abs() < 1e-6);
        assert!(chime.samples[chime.samples.len() - 1].abs() < 1e-6);
    }

    #[test]
    fn resampling_preserves_duration_and_interpolates() {
        let ramp = [0.0, 1.0, 2.0];
        let doubled = resample(&ramp, 1, 2, 4);
        assert_eq!(doubled.len(), 6);
        assert!((doubled[1] - 0.5).abs() < 1e-6);
        assert!((doubled[2] - 1.0).abs() < 1e-6);
        assert!((doubled[3] - 1.5).abs() < 1e-6);
        assert!((doubled[4] - 2.0).abs() < 1e-6);
        assert!((doubled[5] - 2.0).abs() < 1e-6);

        let halved = resample(&[0.5; 480], 1, 48_000, 24_000);
        assert_eq!(halved.len(), 240);
        assert!(halved.iter().all(|sample| *sample == 0.5));
        assert_eq!(resample(&ramp, 1, 48_000, 48_000), ramp);
    }

    #[test]
    fn checksum_rejects_wrong_bytes() {
        let expected = sha256_hex(b"expected notification sound");
        assert!(verify_sha256(b"expected notification sound", &expected));
        assert!(!verify_sha256(b"wrong notification sound", &expected));
        assert!(!verify_sha256(
            b"expected notification sound",
            DISCORD_SHA256
        ));
    }

    #[test]
    fn rate_limit_allows_one_sound_every_two_seconds() {
        assert!(rate_limit_allows(None));
        assert!(!rate_limit_allows(Some(Duration::from_millis(1_999))));
        assert!(rate_limit_allows(Some(Duration::from_secs(2))));
        assert!(rate_limit_allows(Some(Duration::from_secs(10))));
    }
}
