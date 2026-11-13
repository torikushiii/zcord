use super::super::h264;
use std::{
    mem::ManuallyDrop,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Media::MediaFoundation::*,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize,
            },
            Variant::VARIANT,
        },
    },
    core::Interface,
};

const MAX_BYTES: usize = 2 * 1024 * 1024 + 64 * 1024;
const MAX_BUFFER_BYTES: usize = 16 * 1024 * 1024;
const UNAVAILABLE: &str = "Windows video encoding is unavailable";
const FAILED: &str = "Windows video encoding failed";

struct Runtime;

impl Runtime {
    fn open() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|_| UNAVAILABLE.to_owned())?;
            if MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).is_err() {
                CoUninitialize();
                return Err(UNAVAILABLE.to_owned());
            }
        }
        Ok(Self)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = unsafe { MFShutdown() };
        unsafe { CoUninitialize() };
    }
}

struct Activated(Option<IMFActivate>);

impl Drop for Activated {
    fn drop(&mut self) {
        if let Some(activate) = self.0.take() {
            let _ = unsafe { activate.ShutdownObject() };
        }
    }
}

pub(crate) struct Encoder {
    transform: IMFTransform,
    codec: ICodecAPI,
    events: Option<IMFMediaEventGenerator>,
    async_mft: bool,
    _activate: Activated,
    _runtime: Runtime,
    width: u32,
    height: u32,
    frame: i64,
    duration: i64,
    need_input: usize,
    have_output: usize,
    provides_samples: bool,
}

impl Encoder {
    pub(crate) fn new(bitrate: u32, width: u32, height: u32, fps: u32) -> Result<Self, String> {
        let format = Format { width, height, fps };
        let runtime = Runtime::open()?;
        let (opened, software) = match hardware_encoder() {
            Ok(activate) => (Opened::Activate(activate), false),
            Err(_) => (software_encoder()?, true),
        };
        log::info!(
            "stream encoder: {} H.264, {width}x{height} at {fps} fps, {} kbps",
            if software { "software" } else { "hardware" },
            bitrate / 1000,
        );
        let activate = match opened {
            Opened::Activate(activate) => Some(activate),
            Opened::Transform(transform) => {
                return Self::configure(transform, None, software, runtime, bitrate, format);
            }
        };
        let transform: IMFTransform = unsafe {
            activate
                .as_ref()
                .expect("activation")
                .ActivateObject()
                .map_err(|_| UNAVAILABLE.to_owned())?
        };
        Self::configure(transform, activate, software, runtime, bitrate, format)
    }

    fn configure(
        transform: IMFTransform,
        activate: Option<IMFActivate>,
        software: bool,
        runtime: Runtime,
        bitrate: u32,
        format: Format,
    ) -> Result<Self, String> {
        let attributes =
            unsafe { transform.GetAttributes() }.map_err(|_| UNAVAILABLE.to_owned())?;
        let async_mft =
            unsafe { attributes.GetUINT32(&MF_TRANSFORM_ASYNC) }.unwrap_or(0) != 0 && !software;
        if async_mft {
            unsafe {
                attributes
                    .SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)
                    .map_err(|_| UNAVAILABLE.to_owned())?;
            }
        }
        let _ = unsafe { attributes.SetUINT32(&MF_LOW_LATENCY, 1) };
        let codec: ICodecAPI = transform.cast().map_err(|_| UNAVAILABLE.to_owned())?;
        unsafe {
            let _ = codec.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
            let _ = codec.SetValue(
                &CODECAPI_AVEncCommonRateControlMode,
                &VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32),
            );
            let _ = codec.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(bitrate));
            let _ = codec.SetValue(
                &CODECAPI_AVEncMPVDefaultBPictureCount,
                &VARIANT::from(0_u32),
            );
            let _ = codec.SetValue(&CODECAPI_AVEncMPVGOPSize, &VARIANT::from(format.fps * 3));
        }
        let output = video_type(MFVideoFormat_H264, format)?;
        unsafe {
            output
                .SetUINT32(&MF_MT_AVG_BITRATE, bitrate)
                .map_err(|_| UNAVAILABLE.to_owned())?;
            let _ = output.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Main.0 as u32);
            transform
                .SetOutputType(0, &output, 0)
                .map_err(|_| UNAVAILABLE.to_owned())?;
            let input = video_type(MFVideoFormat_NV12, format)?;
            input
                .SetUINT32(&MF_MT_DEFAULT_STRIDE, format.width)
                .map_err(|_| UNAVAILABLE.to_owned())?;
            transform
                .SetInputType(0, &input, 0)
                .map_err(|_| UNAVAILABLE.to_owned())?;
        }
        let info =
            unsafe { transform.GetOutputStreamInfo(0) }.map_err(|_| UNAVAILABLE.to_owned())?;
        let provides_samples = info.dwFlags
            & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32
                | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32)
            != 0;
        if !provides_samples && (info.cbSize == 0 || info.cbSize as usize > MAX_BUFFER_BYTES) {
            return Err(UNAVAILABLE.to_owned());
        }
        let events = if async_mft {
            Some(
                transform
                    .cast::<IMFMediaEventGenerator>()
                    .map_err(|_| UNAVAILABLE.to_owned())?,
            )
        } else {
            None
        };
        unsafe {
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(|_| UNAVAILABLE.to_owned())?;
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(|_| UNAVAILABLE.to_owned())?;
        }
        Ok(Self {
            transform,
            codec,
            events,
            async_mft,
            _activate: Activated(activate),
            _runtime: runtime,
            width: format.width,
            height: format.height,
            frame: 0,
            duration: 10_000_000 / i64::from(format.fps.max(1)),
            need_input: 0,
            have_output: 0,
            provides_samples,
        })
    }

    pub(crate) fn set_bitrate(&mut self, bitrate: u32) -> Result<(), String> {
        unsafe {
            self.codec
                .SetValue(&CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(bitrate))
                .map_err(|_| FAILED.to_owned())
        }
    }

    pub(crate) fn encode(
        &mut self,
        nv12: &[u8],
        force_keyframe: bool,
    ) -> Result<(Vec<u8>, bool), String> {
        let expected = self.width as usize * self.height as usize * 3 / 2;
        if nv12.len() != expected {
            return Err(FAILED.to_owned());
        }
        if force_keyframe {
            unsafe {
                self.codec
                    .SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(1_u32))
                    .map_err(|_| FAILED.to_owned())?;
            }
        }
        if self.async_mft {
            self.wait_for_input()?;
        }
        unsafe {
            let length = u32::try_from(nv12.len()).map_err(|_| FAILED.to_owned())?;
            let buffer = MFCreateMemoryBuffer(length).map_err(|_| FAILED.to_owned())?;
            let mut data = std::ptr::null_mut();
            let mut capacity = 0;
            buffer
                .Lock(&mut data, Some(&mut capacity), None)
                .map_err(|_| FAILED.to_owned())?;
            if data.is_null() || capacity < length {
                let _ = buffer.Unlock();
                return Err(FAILED.to_owned());
            }
            std::ptr::copy_nonoverlapping(nv12.as_ptr(), data, nv12.len());
            buffer.Unlock().map_err(|_| FAILED.to_owned())?;
            buffer
                .SetCurrentLength(length)
                .map_err(|_| FAILED.to_owned())?;
            let sample = MFCreateSample().map_err(|_| FAILED.to_owned())?;
            sample.AddBuffer(&buffer).map_err(|_| FAILED.to_owned())?;
            sample
                .SetSampleTime(self.frame * self.duration)
                .map_err(|_| FAILED.to_owned())?;
            sample
                .SetSampleDuration(self.duration)
                .map_err(|_| FAILED.to_owned())?;
            self.frame += 1;
            self.transform
                .ProcessInput(0, &sample, 0)
                .map_err(|_| FAILED.to_owned())?;
        }
        if self.async_mft {
            self.wait_for_output()?;
        }
        let Some(data) = self.take_output()? else {
            return Ok((Vec::new(), false));
        };
        let keyframe = h264::is_keyframe(&data);
        Ok((data, keyframe))
    }

    fn wait_for_input(&mut self) -> Result<(), String> {
        self.wait_until(|encoder| encoder.need_input != 0)?;
        self.need_input -= 1;
        Ok(())
    }

    fn wait_for_output(&mut self) -> Result<(), String> {
        self.wait_until(|encoder| encoder.have_output != 0)?;
        self.have_output -= 1;
        Ok(())
    }

    fn wait_until(&mut self, ready: impl Fn(&Self) -> bool) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(250);
        while !ready(self) {
            let event = unsafe {
                self.events
                    .as_ref()
                    .expect("asynchronous event queue")
                    .GetEvent(MF_EVENT_FLAG_NO_WAIT)
            };
            match event {
                Ok(event) => unsafe {
                    if event.GetStatus().map_err(|_| FAILED.to_owned())?.is_err() {
                        return Err(FAILED.to_owned());
                    }
                    match event.GetType().map_err(|_| FAILED.to_owned())? as i32 {
                        kind if kind == METransformNeedInput.0 => self.need_input += 1,
                        kind if kind == METransformHaveOutput.0 => self.have_output += 1,
                        kind if kind == MEError.0 => return Err(FAILED.to_owned()),
                        _ => {}
                    }
                },
                Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => {
                    if Instant::now() >= deadline {
                        return Err(FAILED.to_owned());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(_) => return Err(FAILED.to_owned()),
            }
        }
        Ok(())
    }

    fn take_output(&mut self) -> Result<Option<Vec<u8>>, String> {
        unsafe {
            let own = if self.provides_samples {
                None
            } else {
                let info = self
                    .transform
                    .GetOutputStreamInfo(0)
                    .map_err(|_| FAILED.to_owned())?;
                if info.cbSize == 0 || info.cbSize as usize > MAX_BUFFER_BYTES {
                    return Err(FAILED.to_owned());
                }
                let buffer = MFCreateMemoryBuffer(info.cbSize).map_err(|_| FAILED.to_owned())?;
                let sample = MFCreateSample().map_err(|_| FAILED.to_owned())?;
                sample.AddBuffer(&buffer).map_err(|_| FAILED.to_owned())?;
                Some(sample)
            };
            let mut output = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(own.clone()),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            };
            let mut status = 0;
            let result =
                self.transform
                    .ProcessOutput(0, std::slice::from_mut(&mut output), &mut status);
            let sample = ManuallyDrop::into_inner(output.pSample).or(own);
            drop(ManuallyDrop::into_inner(output.pEvents));
            match result {
                Ok(()) => {}
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => {
                    return Ok(None);
                }
                Err(_) => return Err(FAILED.to_owned()),
            }
            let Some(sample) = sample else {
                return Ok(None);
            };
            Ok(Some(sample_bytes(&sample)?))
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        }
    }
}

enum Opened {
    Activate(IMFActivate),
    Transform(IMFTransform),
}

fn hardware_encoder() -> Result<IMFActivate, String> {
    enumerate_encoder(MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_ASYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER)
}

fn software_encoder() -> Result<Opened, String> {
    if let Ok(activate) = enumerate_encoder(MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER) {
        return Ok(Opened::Activate(activate));
    }
    let transform: IMFTransform =
        unsafe { CoCreateInstance(&CLSID_MSH264EncoderMFT, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| UNAVAILABLE.to_owned())?;
    Ok(Opened::Transform(transform))
}

fn enumerate_encoder(flags: MFT_ENUM_FLAG) -> Result<IMFActivate, String> {
    unsafe {
        let input = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_NV12,
        };
        let output = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_H264,
        };
        let mut entries = std::ptr::null_mut();
        let mut count = 0;
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            flags,
            Some(&input),
            Some(&output),
            &mut entries,
            &mut count,
        )
        .map_err(|_| UNAVAILABLE.to_owned())?;
        if entries.is_null() || count == 0 {
            if !entries.is_null() {
                CoTaskMemFree(Some(entries.cast()));
            }
            return Err(UNAVAILABLE.to_owned());
        }
        let entries_slice = std::slice::from_raw_parts_mut(entries, count as usize);
        let selected = entries_slice.first_mut().and_then(Option::take);
        for entry in entries_slice {
            *entry = None;
        }
        CoTaskMemFree(Some(entries.cast()));
        selected.ok_or_else(|| UNAVAILABLE.to_owned())
    }
}

#[derive(Clone, Copy)]
struct Format {
    width: u32,
    height: u32,
    fps: u32,
}

fn video_type(subtype: windows::core::GUID, format: Format) -> Result<IMFMediaType, String> {
    unsafe {
        let media_type = MFCreateMediaType().map_err(|_| UNAVAILABLE.to_owned())?;
        media_type
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|_| UNAVAILABLE.to_owned())?;
        media_type
            .SetGUID(&MF_MT_SUBTYPE, &subtype)
            .map_err(|_| UNAVAILABLE.to_owned())?;
        media_type
            .SetUINT64(
                &MF_MT_FRAME_SIZE,
                (u64::from(format.width) << 32) | u64::from(format.height),
            )
            .map_err(|_| UNAVAILABLE.to_owned())?;
        media_type
            .SetUINT64(&MF_MT_FRAME_RATE, u64::from(format.fps) << 32 | 1)
            .map_err(|_| UNAVAILABLE.to_owned())?;
        media_type
            .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, 1_u64 << 32 | 1)
            .map_err(|_| UNAVAILABLE.to_owned())?;
        media_type
            .SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .map_err(|_| UNAVAILABLE.to_owned())?;
        Ok(media_type)
    }
}

fn sample_bytes(sample: &IMFSample) -> Result<Vec<u8>, String> {
    unsafe {
        if sample.GetTotalLength().map_err(|_| FAILED.to_owned())? as usize > MAX_BYTES {
            return Err(FAILED.to_owned());
        }
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|_| FAILED.to_owned())?;
        let mut data = std::ptr::null_mut();
        let mut capacity = 0;
        let mut length = 0;
        buffer
            .Lock(&mut data, Some(&mut capacity), Some(&mut length))
            .map_err(|_| FAILED.to_owned())?;
        let result = if data.is_null() || length > capacity || length as usize > MAX_BYTES {
            Err(FAILED.to_owned())
        } else {
            Ok(std::slice::from_raw_parts(data, length as usize).to_vec())
        };
        buffer.Unlock().map_err(|_| FAILED.to_owned())?;
        result
    }
}
