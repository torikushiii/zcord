use super::*;

pub(super) struct StereoResampler {
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
    pub(super) fn new(
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

    pub(super) fn push(&mut self, input: &[f32], output: &mut Vec<f32>) {
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

pub(super) fn build_input(
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

pub(super) fn queue_input_samples<T>(queue: &Mutex<VecDeque<f32>>, data: &[T])
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

pub(super) fn build_output(
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
