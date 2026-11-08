use super::{AudioFrame, LiveShare};
use opus::{Application, Bitrate, Channels, Encoder};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender, TrySendError},
    },
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Media::Audio::{
            AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
            AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
            ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
            IActivateAudioInterfaceCompletionHandler,
            IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient,
            PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
        },
        System::{
            Com::{
                BLOB, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
                StructuredStorage::{
                    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
                },
            },
            Threading::{CreateEventW, WaitForSingleObject},
            Variant::VT_BLOB,
        },
    },
    core::{HRESULT, Interface, Ref, implement},
};

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const FRAME_VALUES: usize = RATE as usize / 50 * CHANNELS as usize;
const FRAME_SAMPLES: u64 = RATE as u64 / 50;
const BITRATE: i32 = 64_000;
const MAX_OPUS_PACKET: usize = 1275;
const BUFFER: Duration = Duration::from_millis(100);
const STOP_CHECK_MS: u32 = 100;
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;

pub(crate) fn spawn(
    frames: SyncSender<AudioFrame>,
    stop: Arc<AtomicBool>,
    live: Arc<LiveShare>,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name("stream-audio".into())
        .spawn(move || {
            if let Err(reason) = run(&frames, &stop, &live) {
                log::warn!("Stream audio stopped: {reason}");
            }
        })
        .map_err(|_| "Could not start the stream audio thread".to_owned())?;
    Ok(())
}

struct Com(bool);

impl Com {
    fn join() -> Self {
        Self(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

struct Event(HANDLE);

impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn run(frames: &SyncSender<AudioFrame>, stop: &AtomicBool, live: &LiveShare) -> Result<(), String> {
    let _com = Com::join();
    let client = activate(std::process::id())?;
    let block = CHANNELS * 4;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT,
        nChannels: CHANNELS,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * u32::from(block),
        nBlockAlign: block,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    let capture: IAudioCaptureClient = unsafe {
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                (BUFFER.as_nanos() / 100) as i64,
                0,
                &raw const format,
                None,
            )
            .map_err(|error| format!("Windows refused the sound format: {error}"))?;
        client
            .GetService()
            .map_err(|error| format!("No sound capture: {error}"))?
    };
    let event = Event(
        unsafe { CreateEventW(None, false, false, None) }
            .map_err(|error| format!("No sound event: {error}"))?,
    );
    unsafe {
        client
            .SetEventHandle(event.0)
            .map_err(|error| format!("No sound event: {error}"))?;
        client
            .Start()
            .map_err(|error| format!("The sound wouldn't start: {error}"))?;
    }
    let result = hear(&capture, &event, frames, stop, live);
    unsafe {
        let _ = client.Stop();
    }
    result
}

fn hear(
    capture: &IAudioCaptureClient,
    event: &Event,
    frames: &SyncSender<AudioFrame>,
    stop: &AtomicBool,
    live: &LiveShare,
) -> Result<(), String> {
    let mut encoder = Encoder::new(RATE, Channels::Stereo, Application::Audio)
        .map_err(|error| format!("No Opus encoder: {error}"))?;
    let _ = encoder.set_bitrate(Bitrate::Bits(BITRATE));
    let mut pending: Vec<f32> = Vec::with_capacity(FRAME_VALUES * 8);
    let (mut heard, mut pending_start) = (0u64, 0u64);
    let mut pcm = vec![0i16; FRAME_VALUES];
    let gone = |error: windows::core::Error| format!("The sound went away: {error}");
    while !stop.load(Ordering::Acquire) {
        let waited = unsafe { WaitForSingleObject(event.0, STOP_CHECK_MS) };
        if waited != WAIT_OBJECT_0 && waited != WAIT_TIMEOUT {
            return Err("The sound stopped".into());
        }
        while unsafe { capture.GetNextPacketSize() }.map_err(gone)? > 0 {
            let (mut data, mut count, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
            unsafe {
                capture
                    .GetBuffer(&mut data, &mut count, &mut flags, None, None)
                    .map_err(gone)?;
                let values = count as usize * usize::from(CHANNELS);
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null();
                if silent {
                    pending.resize(pending.len() + values, 0.);
                } else {
                    // Read as bytes: the engine doesn't promise float alignment.
                    let bytes = std::slice::from_raw_parts(data, values * 4);
                    pending.extend(bytes.chunks_exact(4).map(|sample| {
                        f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]])
                    }));
                }
                capture.ReleaseBuffer(count).map_err(gone)?;
                heard += u64::from(count);
            }
        }
        if !live.audio.load(Ordering::Acquire) {
            pending.clear();
            pending_start = heard;
        }
        while pending.len() >= FRAME_VALUES {
            for (sample, value) in pcm.iter_mut().zip(pending.drain(..FRAME_VALUES)) {
                *sample = (value.clamp(-1., 1.) * f32::from(i16::MAX)) as i16;
            }
            let opus = encoder
                .encode_vec(&pcm, MAX_OPUS_PACKET)
                .map_err(|error| format!("Opus encoding failed: {error}"))?;
            let sample = pending_start;
            pending_start += FRAME_SAMPLES;
            match frames.try_send(AudioFrame { opus, sample }) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => return Ok(()),
            }
        }
    }
    Ok(())
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(mpsc::Sender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(
        &self,
        _: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let _ = self.0.send(());
        Ok(())
    }
}

fn activate(process: u32) -> Result<IAudioClient, String> {
    let mut parameters = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: process,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // Never cleared: clearing would free the parameters it points to, which it doesn't own.
    let variant = std::mem::ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of_val(&parameters) as u32,
                        pBlobData: (&raw mut parameters).cast(),
                    },
                },
                ..Default::default()
            }),
        },
    });
    let (done, activated) = mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(done).into();
    let refused = |error: windows::core::Error| {
        format!(
            "Windows won't share other applications' sound (needs Windows 10 2004 or later): {error}"
        )
    };
    unsafe {
        let operation = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&raw const *variant),
            &handler,
        )
        .map_err(refused)?;
        activated
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "Windows didn't answer the sound request".to_owned())?;
        let (mut result, mut client) = (HRESULT(0), None);
        operation
            .GetActivateResult(&mut result, &mut client)
            .map_err(refused)?;
        result.ok().map_err(refused)?;
        client
            .ok_or_else(|| "Windows gave no sound stream".to_owned())?
            .cast()
            .map_err(refused)
    }
}
