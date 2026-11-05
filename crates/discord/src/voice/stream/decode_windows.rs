use super::{DecodeError, Decoder, Picture};
use std::mem::ManuallyDrop;
use windows::Win32::{
    Media::MediaFoundation::*,
    System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    },
};

const MAX_ACCESS_UNIT: usize = 2 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const TICKS_PER_PICTURE: i64 = 333_333;
const START_CODE: [u8; 4] = [0, 0, 0, 1];

struct Com;

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct Foundation;

impl Drop for Foundation {
    fn drop(&mut self) {
        let _ = unsafe { MFShutdown() };
    }
}

#[derive(Clone, Copy)]
struct OutputFormat {
    width: u32,
    height: u32,
    stride: u32,
    crop: (u32, u32, u32, u32),
}

pub(super) fn decoder() -> Result<Box<dyn Decoder>, String> {
    WindowsDecoder::open()
        .map(|decoder| Box::new(decoder) as Box<dyn Decoder>)
        .map_err(|reason| reason.to_owned())
}

// The transform is created in an MTA and used only on the single MTA decode thread.
unsafe impl Send for WindowsDecoder {}

struct WindowsDecoder {
    transform: IMFTransform,
    output: Option<OutputFormat>,
    pictures: i64,
    provides_samples: bool,
    // Fields drop in declaration order: release the transform before MFShutdown/COM.
    _foundation: Foundation,
    _com: Com,
}

impl WindowsDecoder {
    fn open() -> Result<Self, &'static str> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|_| "Could not start Media Foundation")?;
        let com = Com;
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .map_err(|_| "Could not start Media Foundation")?;
        let foundation = Foundation;
        let transform: IMFTransform =
            unsafe { CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER) }
                .map_err(|_| "The H264 decoder is unavailable")?;
        if let Ok(attributes) = unsafe { transform.GetAttributes() } {
            let _ = unsafe { attributes.SetUINT32(&CODECAPI_AVLowLatencyMode, 1) };
        }
        let input =
            unsafe { MFCreateMediaType() }.map_err(|_| "Could not configure the decoder")?;
        unsafe {
            input
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(|_| "Could not configure the decoder")?;
            input
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
                .map_err(|_| "Could not configure the decoder")?;
            transform
                .SetInputType(0, &input, 0)
                .map_err(|_| "Could not configure the decoder")?;
        }
        let mut decoder = Self {
            transform,
            output: None,
            pictures: 0,
            provides_samples: false,
            _foundation: foundation,
            _com: com,
        };
        decoder.negotiate_output()?;
        unsafe {
            decoder
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(|_| "Could not start the decoder")?;
            decoder
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(|_| "Could not start the decoder")?;
        }
        Ok(decoder)
    }

    fn negotiate_output(&mut self) -> Result<(), &'static str> {
        unsafe {
            let mut index = 0;
            loop {
                let candidate = match self.transform.GetOutputAvailableType(0, index) {
                    Ok(candidate) => candidate,
                    Err(_) => break,
                };
                index += 1;
                if candidate
                    .GetGUID(&MF_MT_SUBTYPE)
                    .is_ok_and(|subtype| subtype == MFVideoFormat_NV12)
                    && self.transform.SetOutputType(0, &candidate, 0).is_ok()
                {
                    break;
                }
            }
            let current = self
                .transform
                .GetOutputCurrentType(0)
                .map_err(|_| "The decoder did not expose NV12 output")?;
            let size = current
                .GetUINT64(&MF_MT_FRAME_SIZE)
                .map_err(|_| "The decoder did not expose a frame size")?;
            let (width, height) = ((size >> 32) as u32, size as u32);
            if width == 0 || height == 0 {
                return Err("The decoder did not expose a frame size");
            }
            let stride = current
                .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                .ok()
                .filter(|stride| *stride >= width)
                .unwrap_or(width);
            let mut crop = (0, 0, width & !1, height & !1);
            let mut bytes = [0u8; std::mem::size_of::<MFVideoArea>()];
            let mut written = 0;
            if current
                .GetBlob(
                    &MF_MT_MINIMUM_DISPLAY_APERTURE,
                    &mut bytes,
                    Some(&mut written),
                )
                .is_ok()
                && written as usize == bytes.len()
            {
                let area = std::ptr::read_unaligned(bytes.as_ptr().cast::<MFVideoArea>());
                let (x, y) = (
                    area.OffsetX.value.max(0) as u32,
                    area.OffsetY.value.max(0) as u32,
                );
                let (w, h) = (area.Area.cx.max(0) as u32, area.Area.cy.max(0) as u32);
                if w >= 2 && h >= 2 && x + w <= width && y + h <= height {
                    crop = if x % 2 == 0 && y % 2 == 0 {
                        (x, y, w & !1, h & !1)
                    } else {
                        (0, 0, width & !1, height & !1)
                    };
                }
            }
            if u64::from(stride) * u64::from(height) * 3 / 2 > MAX_OUTPUT_BYTES as u64 {
                return Err("The decoder produced an oversized picture");
            }
            let info = self
                .transform
                .GetOutputStreamInfo(0)
                .map_err(|_| "Could not read the decoder output")?;
            self.provides_samples = info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32
                    | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32)
                != 0;
            self.output = Some(OutputFormat {
                width,
                height,
                stride,
                crop,
            });
            Ok(())
        }
    }

    fn drain(&mut self) -> Result<Vec<Picture>, DecodeError> {
        let mut pictures = Vec::new();
        loop {
            let format = self.output.ok_or(DecodeError::Failed)?;
            unsafe {
                let own = if self.provides_samples {
                    None
                } else {
                    let info = self
                        .transform
                        .GetOutputStreamInfo(0)
                        .map_err(|_| DecodeError::Failed)?;
                    if info.cbSize as usize > MAX_OUTPUT_BYTES {
                        return Err(DecodeError::Failed);
                    }
                    let buffer =
                        MFCreateMemoryBuffer(info.cbSize).map_err(|_| DecodeError::Failed)?;
                    let sample = MFCreateSample().map_err(|_| DecodeError::Failed)?;
                    sample.AddBuffer(&buffer).map_err(|_| DecodeError::Failed)?;
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
                let provided = ManuallyDrop::into_inner(output.pSample);
                drop(ManuallyDrop::into_inner(output.pEvents));
                let sample = match result {
                    Ok(()) => provided.or(own),
                    Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                    Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                        self.negotiate_output().map_err(|_| DecodeError::Failed)?;
                        continue;
                    }
                    Err(_) => return Err(DecodeError::Failed),
                };
                let Some(sample) = sample else {
                    break;
                };
                pictures.push(sample_picture(&sample, format)?);
            }
        }
        Ok(pictures)
    }
}

impl Decoder for WindowsDecoder {
    fn decode(&mut self, access_unit: &[u8]) -> Result<Vec<Picture>, DecodeError> {
        if access_unit.is_empty() || access_unit.len() > MAX_ACCESS_UNIT {
            return Err(DecodeError::Failed);
        }
        unsafe {
            let length = u32::try_from(access_unit.len()).map_err(|_| DecodeError::Failed)?;
            let buffer = MFCreateMemoryBuffer(length).map_err(|_| DecodeError::Failed)?;
            let mut data = std::ptr::null_mut();
            let mut capacity = 0;
            buffer
                .Lock(&mut data, Some(&mut capacity), None)
                .map_err(|_| DecodeError::Failed)?;
            if data.is_null() || (capacity as usize) < access_unit.len() {
                let _ = buffer.Unlock();
                return Err(DecodeError::Failed);
            }
            std::ptr::copy_nonoverlapping(access_unit.as_ptr(), data, access_unit.len());
            buffer.Unlock().map_err(|_| DecodeError::Failed)?;
            buffer
                .SetCurrentLength(length)
                .map_err(|_| DecodeError::Failed)?;
            let sample = MFCreateSample().map_err(|_| DecodeError::Failed)?;
            sample.AddBuffer(&buffer).map_err(|_| DecodeError::Failed)?;
            self.pictures += 1;
            sample
                .SetSampleTime(self.pictures * TICKS_PER_PICTURE)
                .map_err(|_| DecodeError::Failed)?;
            sample
                .SetSampleDuration(TICKS_PER_PICTURE)
                .map_err(|_| DecodeError::Failed)?;
            match self.transform.ProcessInput(0, &sample, 0) {
                Ok(()) => {}
                Err(error) if error.code() == MF_E_NOTACCEPTING => {
                    self.drain()?;
                    self.transform
                        .ProcessInput(0, &sample, 0)
                        .map_err(|_| DecodeError::Failed)?;
                }
                Err(_) => return Err(DecodeError::Failed),
            }
        }
        self.drain()
    }
}

fn sample_picture(sample: &IMFSample, format: OutputFormat) -> Result<Picture, DecodeError> {
    unsafe {
        if sample.GetTotalLength().map_err(|_| DecodeError::Failed)? as usize > MAX_OUTPUT_BYTES {
            return Err(DecodeError::Failed);
        }
        let buffer = sample
            .ConvertToContiguousBuffer()
            .map_err(|_| DecodeError::Failed)?;
        let mut data = std::ptr::null_mut();
        let mut capacity = 0;
        let mut length = 0;
        buffer
            .Lock(&mut data, Some(&mut capacity), Some(&mut length))
            .map_err(|_| DecodeError::Failed)?;
        let bytes = if length > capacity
            || length as usize > MAX_OUTPUT_BYTES
            || (length != 0 && data.is_null())
        {
            Err(DecodeError::Failed)
        } else {
            Ok(std::slice::from_raw_parts(data, length as usize).to_vec())
        };
        buffer.Unlock().map_err(|_| DecodeError::Failed)?;
        pack_nv12(&bytes?, format)
    }
}

fn pack_nv12(bytes: &[u8], format: OutputFormat) -> Result<Picture, DecodeError> {
    let (stride, height) = (format.stride as usize, format.height as usize);
    let (x, y, width, out_height) = (
        format.crop.0 as usize,
        format.crop.1 as usize,
        format.crop.2 as usize,
        format.crop.3 as usize,
    );
    if width == 0 || out_height == 0 {
        return Err(DecodeError::Failed);
    }
    if width > 1920 || out_height > 1080 {
        return Err(DecodeError::TooLarge);
    }
    let luma_len = stride * height;
    if bytes.len() < luma_len + stride * height.div_ceil(2)
        || x + width > format.width as usize
        || y + out_height > height
    {
        return Err(DecodeError::Failed);
    }
    let (luma, chroma) = bytes.split_at(luma_len);
    let mut picture = Picture {
        width: width as u32,
        height: out_height as u32,
        stride: width as u32,
        y: Vec::with_capacity(width * out_height),
        uv: Vec::with_capacity(width * out_height.div_ceil(2)),
    };
    for row in 0..out_height {
        let start = (y + row) * stride + x;
        picture.y.extend_from_slice(&luma[start..start + width]);
    }
    for row in 0..out_height.div_ceil(2) {
        let start = (y / 2 + row) * stride + x;
        picture.uv.extend_from_slice(&chroma[start..start + width]);
    }
    Ok(picture)
}

#[allow(dead_code)]
fn _start_code(access_unit: &[u8]) -> bool {
    access_unit.starts_with(&START_CODE)
}
