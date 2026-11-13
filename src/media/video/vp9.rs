use std::{collections::VecDeque, marker::PhantomData, mem::ManuallyDrop, path::Path, rc::Rc};

use anyhow::{Context, Result, ensure};
use image::{
    RgbaImage,
    imageops::{FilterType, resize},
};
use windows::Win32::{Media::MediaFoundation::*, System::Com::*};

use super::{
    animation::{VideoRead, VideoReader, output_size},
    platform::{Com, Foundation, decode_permit},
    webm::{self, Matrix},
};

const MAX_WAITING: usize = 4;

struct Plane {
    time: Option<i64>,
    width: u32,
    height: u32,
    rows: usize,
    stride: usize,
    bytes: Vec<u8>,
}

impl Plane {
    fn luma(&self, x: usize, y: usize) -> u8 {
        self.bytes[y * self.stride + x]
    }

    fn chroma(&self, x: usize, y: usize) -> (u8, u8) {
        let offset = self.stride * self.rows + (y / 2) * self.stride + (x / 2) * 2;
        (self.bytes[offset], self.bytes[offset + 1])
    }
}

struct Decoder {
    transform: IMFTransform,
    visible: (u32, u32),
}

impl Decoder {
    fn new(width: u32, height: u32) -> Result<Self> {
        let info = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MFMediaType_Video,
            guidSubtype: MFVideoFormat_VP90,
        };
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        unsafe {
            MFTEnumEx(
                MFT_CATEGORY_VIDEO_DECODER,
                MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
                Some(&info),
                None,
                &mut list,
                &mut count,
            )?
        };
        let activate = if list.is_null() || count == 0 {
            None
        } else {
            // SAFETY: MFTEnumEx returned `count` initialized entries; each is released once and
            // the array itself is freed with CoTaskMemFree.
            unsafe {
                let first = (*list).clone();
                for index in 0..count as usize {
                    std::ptr::drop_in_place(list.add(index));
                }
                CoTaskMemFree(Some(list as *const _));
                first
            }
        };
        let activate = activate.context("no VP9 decoder is installed")?;
        let transform: IMFTransform = unsafe { activate.ActivateObject()? };
        let input = unsafe { MFCreateMediaType()? };
        unsafe {
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_VP90)?;
            input.SetUINT64(
                &MF_MT_FRAME_SIZE,
                (u64::from(width) << 32) | u64::from(height),
            )?;
            transform.SetInputType(0, &input, 0)?;
        }
        let decoder = Self {
            transform,
            visible: (width, height),
        };
        decoder.select_output()?;
        unsafe {
            decoder
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            decoder
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        }
        Ok(decoder)
    }

    fn select_output(&self) -> Result<()> {
        for index in 0..64 {
            let kind = unsafe { self.transform.GetOutputAvailableType(0, index)? };
            if unsafe { kind.GetGUID(&MF_MT_SUBTYPE)? } == MFVideoFormat_NV12 {
                unsafe { self.transform.SetOutputType(0, &kind, 0)? };
                return Ok(());
            }
        }
        anyhow::bail!("VP9 decoder has no NV12 output")
    }

    fn push(&self, data: &[u8], time: i64) -> Result<()> {
        let length = u32::try_from(data.len()).context("VP9 frame too large")?;
        let buffer = unsafe { MFCreateMemoryBuffer(length)? };
        let mut pointer = std::ptr::null_mut();
        unsafe { buffer.Lock(&mut pointer, None, None)? };
        ensure!(!pointer.is_null(), "VP9 input buffer is null");
        // SAFETY: the buffer was allocated with `length` bytes and is locked.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), pointer, data.len()) };
        unsafe {
            buffer.Unlock()?;
            buffer.SetCurrentLength(length)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(time)?;
            self.transform.ProcessInput(0, &sample, 0)?;
        }
        Ok(())
    }

    fn pull(&self, planes: &mut VecDeque<Plane>) -> Result<()> {
        loop {
            let info = unsafe { self.transform.GetOutputStreamInfo(0)? };
            let provided = info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32
                    | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32)
                != 0;
            let sample = if provided {
                None
            } else {
                let sample = unsafe { MFCreateSample()? };
                unsafe { sample.AddBuffer(&MFCreateMemoryBuffer(info.cbSize)?)? };
                Some(sample)
            };
            let mut buffers = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            }];
            let mut status = 0;
            let result = unsafe { self.transform.ProcessOutput(0, &mut buffers, &mut status) };
            // SAFETY: ProcessOutput hands back ownership of both fields; each is taken once.
            let sample = unsafe { ManuallyDrop::take(&mut buffers[0].pSample) };
            drop(unsafe { ManuallyDrop::take(&mut buffers[0].pEvents) });
            match result {
                Ok(()) => {
                    planes.push_back(self.plane(&sample.context("VP9 decoder gave no sample")?)?)
                }
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    self.select_output()?
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn plane(&self, sample: &IMFSample) -> Result<Plane> {
        let kind = unsafe { self.transform.GetOutputCurrentType(0)? };
        let size = unsafe { kind.GetUINT64(&MF_MT_FRAME_SIZE)? };
        let (coded_width, coded_height) = ((size >> 32) as u32, size as u32);
        let width = coded_width.min(self.visible.0);
        let height = coded_height.min(self.visible.1);
        ensure!(width > 0 && height > 0, "empty VP9 frame");
        let declared = unsafe { kind.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
            .ok()
            .and_then(|stride| usize::try_from(stride as i32).ok());
        let buffer = unsafe { sample.ConvertToContiguousBuffer()? };
        let mut pointer = std::ptr::null_mut();
        let mut length = 0u32;
        unsafe { buffer.Lock(&mut pointer, None, Some(&mut length))? };
        let bytes = if pointer.is_null() {
            Vec::new()
        } else {
            // SAFETY: the locked buffer holds `length` readable bytes until Unlock.
            unsafe { std::slice::from_raw_parts(pointer, length as usize).to_vec() }
        };
        unsafe { buffer.Unlock()? };
        let rows = coded_height as usize;
        let total_rows = rows + rows.div_ceil(2);
        let fits = |stride: usize| {
            stride >= (width as usize).next_multiple_of(2)
                && stride
                    .checked_mul(total_rows)
                    .is_some_and(|needed| needed <= bytes.len())
        };
        let stride = declared
            .filter(|stride| fits(*stride))
            .or_else(|| Some(bytes.len() / total_rows).filter(|stride| fits(*stride)))
            .context("VP9 frame buffer does not match its size")?;
        Ok(Plane {
            time: unsafe { sample.GetSampleTime() }.ok(),
            width,
            height,
            rows,
            stride,
            bytes,
        })
    }

    fn drain(&self, planes: &mut VecDeque<Plane>) -> Result<()> {
        unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)?;
            self.transform
                .ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)?;
        }
        self.pull(planes)
    }

    fn flush(&self) -> Result<()> {
        unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0)?;
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        }
        Ok(())
    }
}

fn compose(color: &Plane, alpha: Option<&Plane>, matrix: Matrix, full_range: bool) -> RgbaImage {
    let (red_v, green_u, green_v, blue_u) = match matrix {
        Matrix::Bt601 => (1.402, 0.344_136, 0.714_136, 1.772),
        Matrix::Bt709 => (1.5748, 0.187_324, 0.468_124, 1.8556),
    };
    let (scale, chroma_scale, offset) = if full_range {
        (1., 1., 0.)
    } else {
        (255. / 219., 255. / 224., 16.)
    };
    let width = color.width;
    let height = color.height;
    let mut image = RgbaImage::new(width, height);
    for (y, row) in image.rows_mut().enumerate() {
        for (x, pixel) in row.enumerate() {
            let luma = (f32::from(color.luma(x, y)) - offset) * scale;
            let (u, v) = color.chroma(x, y);
            let u = (f32::from(u) - 128.) * chroma_scale;
            let v = (f32::from(v) - 128.) * chroma_scale;
            let red = luma + red_v * v;
            let green = luma - green_u * u - green_v * v;
            let blue = luma + blue_u * u;
            let opacity = alpha
                .filter(|plane| (x as u32) < plane.width && (y as u32) < plane.height)
                .map_or(255, |plane| plane.luma(x, y));
            pixel.0 = [
                blue.clamp(0., 255.) as u8,
                green.clamp(0., 255.) as u8,
                red.clamp(0., 255.) as u8,
                opacity,
            ];
        }
    }
    image
}

pub(crate) struct AlphaReader {
    track: webm::Track,
    next: usize,
    drained: bool,
    target: (u32, u32),
    times: VecDeque<(i64, bool)>,
    colors: VecDeque<Plane>,
    alphas: VecDeque<Plane>,
    color: Decoder,
    alpha: Option<Decoder>,
    // Fields drop in declaration order: release MF objects before MFShutdown/COM.
    _foundation: Foundation,
    _com: Com,
    _thread: PhantomData<Rc<()>>,
}

impl AlphaReader {
    pub(crate) fn open(path: &Path, size: u32) -> Result<Self> {
        let _permit = decode_permit()?;
        let track = webm::demux(&std::fs::read(path)?)?;
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        let com = Com;
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL)? };
        let foundation = Foundation;
        let color = Decoder::new(track.width, track.height)?;
        let alpha = if track.has_alpha() {
            Some(Decoder::new(track.width, track.height)?)
        } else {
            None
        };
        let target =
            output_size(track.width, track.height, size, 4).context("invalid output dimensions")?;
        Ok(Self {
            track,
            next: 0,
            drained: false,
            target,
            times: VecDeque::new(),
            colors: VecDeque::new(),
            alphas: VecDeque::new(),
            color,
            alpha,
            _foundation: foundation,
            _com: com,
            _thread: PhantomData,
        })
    }

    pub(crate) fn rate(&self) -> Option<(u32, u32)> {
        let nanos = u32::try_from(self.track.frame_nanos()?).ok()?;
        Some((1_000_000_000, nanos.max(1)))
    }

    // Reference-only VP9 packets decode without output; queue position does not identify a frame.
    fn ready(&mut self) -> bool {
        let Some(time) = self.colors.front().map(|color| color.time) else {
            return false;
        };
        if let Some(time) = time {
            while self.times.len() > 1 && self.times.front().is_some_and(|(at, _)| *at < time) {
                self.times.pop_front();
            }
            while self
                .alphas
                .front()
                .is_some_and(|alpha| alpha.time.is_some_and(|at| at < time))
            {
                self.alphas.pop_front();
            }
        }
        let Some(&(_, has_alpha)) = self.times.front() else {
            return false;
        };
        !has_alpha || self.drained || !self.alphas.is_empty() || self.colors.len() > MAX_WAITING
    }

    fn feed(&mut self) -> Result<bool> {
        let Some(frame) = self.track.frames.get(self.next) else {
            if self.drained {
                return Ok(false);
            }
            self.drained = true;
            self.color.drain(&mut self.colors)?;
            if let Some(alpha) = &self.alpha {
                alpha.drain(&mut self.alphas)?;
            }
            return Ok(true);
        };
        self.next += 1;
        let time = (frame.time / 100) as i64;
        let alpha_data = self.alpha.as_ref().zip(frame.alpha.as_deref());
        self.times.push_back((time, alpha_data.is_some()));
        self.color.push(&frame.color, time)?;
        self.color.pull(&mut self.colors)?;
        if let Some((alpha, data)) = alpha_data {
            alpha.push(data, time)?;
            alpha.pull(&mut self.alphas)?;
        }
        Ok(true)
    }
}

impl VideoReader for AlphaReader {
    fn read(&mut self, pixels: bool) -> Result<VideoRead> {
        let _permit = decode_permit()?;
        while !self.ready() {
            if !self.feed()? {
                return Ok(VideoRead::End);
            }
        }
        let (input_time, has_alpha) = self.times.pop_front().unwrap_or_default();
        let color = self.colors.pop_front().context("missing VP9 frame")?;
        let timestamp = color.time.unwrap_or(input_time);
        let matches = |alpha: &Plane| match (alpha.time, color.time) {
            (Some(alpha), Some(color)) => alpha == color,
            _ => true,
        };
        let alpha = (has_alpha && self.alphas.front().is_some_and(matches))
            .then(|| self.alphas.pop_front())
            .flatten();
        let image = pixels.then(|| {
            let image = compose(
                &color,
                alpha.as_ref(),
                self.track.matrix,
                self.track.full_range,
            );
            if image.dimensions() == self.target {
                image
            } else {
                resize(&image, self.target.0, self.target.1, FilterType::Triangle)
            }
        });
        Ok(VideoRead::Sample {
            timestamp,
            duration: None,
            pixels: image,
        })
    }

    fn restart(&mut self) -> Result<()> {
        self.color.flush()?;
        if let Some(alpha) = &self.alpha {
            alpha.flush()?;
        }
        self.next = 0;
        self.drained = false;
        self.times.clear();
        self.colors.clear();
        self.alphas.clear();
        Ok(())
    }
}
