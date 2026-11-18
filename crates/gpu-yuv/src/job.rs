//! The conversion as a `craft-gpu` job: [`YuvProvider`] answers [`GpuJob`]s whose provider id is
//! [`PROVIDER_ID`], dispatched through a [`craft_gpu::ProviderRegistry`].
//!
//! A job has one input, an RGBA8 buffer of `src_width × src_height × 4` bytes in the context, and
//! [`YuvJob::params`] as its parameters. Its outcome has one output: a new buffer that holds the
//! packed frame in [`FrameLayout`] order. Every side must be no larger than the context's
//! [`DeviceCaps::max_texture_dimension_2d`](craft_gpu::DeviceCaps), and the frame no larger than its
//! `max_buffer_size`, on every context, so that a job that runs on the null context also runs on a
//! device with the same caps.

use craft_gpu::{GpuContext, GpuFault, GpuJob, GpuJobOutcome, GpuJobParams, GpuProvider, HandleId};

use crate::cpu;
use crate::layout::FrameLayout;
use crate::spec::{Conversion, Error, Format, Matrix, Range};

/// The provider id of [`YuvProvider`].
pub const PROVIDER_ID: &str = "craft.yuv";

/// The parameter keys, in the order [`YuvJob::params`] writes them.
const KEYS: [&str; 7] = ["fmt", "matrix", "range", "sw", "sh", "w", "h"];

/// The parameters of one `craft.yuv` job: the conversion, the source size and the output size.
///
/// They are written as ASCII `key:value` fields separated by `;`, for example
/// `fmt:nv12;matrix:bt709;range:limited;sw:1920;sh:1080;w:1280;h:720`. `fmt` is `nv12`, `p010` or
/// `i420`; `matrix` is `bt601`, `bt709` or `bt2020`; `range` is `limited` or `full`; the sides are
/// decimal digits. A missing, repeated or unknown key, or a value outside these, is
/// [`GpuFault::InvalidJob`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct YuvJob {
    pub conversion: Conversion,
    pub src_width: u32,
    pub src_height: u32,
    pub width: u32,
    pub height: u32,
}

fn invalid(message: impl std::fmt::Display) -> GpuFault {
    GpuFault::InvalidJob(format!("{PROVIDER_ID}: {message}"))
}

/// A conversion error as a fault: device errors are [`GpuFault::Operation`], the rest describe a
/// job that cannot run and are [`GpuFault::InvalidJob`].
fn fault(e: Error) -> GpuFault {
    match e {
        Error::Device(_) => GpuFault::Operation(format!("{PROVIDER_ID}: {e}")),
        _ => invalid(e),
    }
}

fn format_token(f: Format) -> &'static str {
    match f {
        Format::Nv12 => "nv12",
        Format::P010 => "p010",
        Format::I420 => "i420",
    }
}

fn matrix_token(m: Matrix) -> &'static str {
    match m {
        Matrix::Bt601 => "bt601",
        Matrix::Bt709 => "bt709",
        Matrix::Bt2020 => "bt2020",
    }
}

fn range_token(r: Range) -> &'static str {
    match r {
        Range::Limited => "limited",
        Range::Full => "full",
    }
}

impl YuvJob {
    pub fn new(conversion: Conversion, src_width: u32, src_height: u32, width: u32, height: u32) -> YuvJob {
        YuvJob { conversion, src_width, src_height, width, height }
    }

    /// The job's parameters.
    pub fn params(&self) -> GpuJobParams {
        let c = self.conversion;
        let text = format!(
            "fmt:{};matrix:{};range:{};sw:{};sh:{};w:{};h:{}",
            format_token(c.format),
            matrix_token(c.matrix),
            range_token(c.range),
            self.src_width,
            self.src_height,
            self.width,
            self.height
        );
        GpuJobParams(text.into_bytes())
    }

    /// A `craft.yuv` job that converts the RGBA8 buffer `input`.
    pub fn job(&self, input: HandleId) -> GpuJob {
        GpuJob { provider_id: PROVIDER_ID.into(), params: self.params(), inputs: vec![input], outputs: Vec::new() }
    }

    /// Read parameters written by [`YuvJob::params`].
    pub fn parse(params: &GpuJobParams) -> Result<YuvJob, GpuFault> {
        let text = std::str::from_utf8(&params.0).map_err(|_| invalid("the parameters are not UTF-8"))?;
        let mut values: [Option<&str>; KEYS.len()] = [None; KEYS.len()];
        for field in text.split(';') {
            let (key, value) = field.split_once(':').ok_or_else(|| invalid(format!("`{field}` is not a key:value field")))?;
            let i = KEYS.iter().position(|k| *k == key).ok_or_else(|| invalid(format!("unknown key `{key}`")))?;
            if values[i].replace(value).is_some() {
                return Err(invalid(format!("`{key}` is given twice")));
            }
        }
        let get = |i: usize| values[i].ok_or_else(|| invalid(format!("`{}` is missing", KEYS[i])));
        let token = |i: usize, value: &str| invalid(format!("`{value}` is not a value of `{}`", KEYS[i]));
        let format = match get(0)? {
            "nv12" => Format::Nv12,
            "p010" => Format::P010,
            "i420" => Format::I420,
            v => return Err(token(0, v)),
        };
        let matrix = match get(1)? {
            "bt601" => Matrix::Bt601,
            "bt709" => Matrix::Bt709,
            "bt2020" => Matrix::Bt2020,
            v => return Err(token(1, v)),
        };
        let range = match get(2)? {
            "limited" => Range::Limited,
            "full" => Range::Full,
            v => return Err(token(2, v)),
        };
        let side = |i: usize| -> Result<u32, GpuFault> {
            let v = get(i)?;
            if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
                return Err(token(i, v));
            }
            v.parse().map_err(|_| token(i, v))
        };
        Ok(YuvJob {
            conversion: Conversion::new(format, matrix, range),
            src_width: side(3)?,
            src_height: side(4)?,
            width: side(5)?,
            height: side(6)?,
        })
    }
}

/// The `craft.yuv` provider.
///
/// [`YuvProvider::new`] converts with [`cpu::convert`], on any context: the null context in tests
/// and CI, or a context whose device a provider cannot reach. [`YuvProvider::with_device`]
/// converts with the render passes on a device the caller holds, such as the one it already draws
/// with; the RGBA8 input and the output frame still move through the context's buffers. Both give
/// the same frame within the tolerance the crate's tests state.
pub struct YuvProvider {
    device: Option<device::DeviceRunner>,
}

impl Default for YuvProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl YuvProvider {
    /// A provider that converts on the CPU.
    pub fn new() -> YuvProvider {
        YuvProvider { device: None }
    }

    /// A provider that converts with the render passes on `device`. Each job waits for the device
    /// to finish, so this is not available on `wasm32`, where a buffer map completes only from the
    /// browser's event loop.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue) -> YuvProvider {
        YuvProvider { device: Some(device::DeviceRunner::new(device, queue)) }
    }

    /// Whether jobs run on a device.
    pub fn has_device(&self) -> bool {
        self.device.is_some()
    }

    fn convert(&self, rgba: &[u8], job: &YuvJob) -> Result<Vec<u8>, Error> {
        match &self.device {
            Some(d) => d.convert(rgba, job),
            None => cpu::convert(rgba, job.src_width, job.src_height, job.conversion, job.width, job.height),
        }
    }
}

impl GpuProvider for YuvProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn run(&self, ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<GpuJobOutcome, GpuFault> {
        let spec = YuvJob::parse(&job.params)?;
        let [input] = job.inputs.as_slice() else {
            return Err(invalid(format!("expects one RGBA8 input, not {}", job.inputs.len())));
        };
        if !job.outputs.is_empty() {
            return Err(invalid("makes its own output buffer, so the job names no outputs"));
        }
        let YuvJob { conversion, src_width, src_height, width, height } = spec;
        let layout = FrameLayout::new(conversion.format, width, height).map_err(fault)?;
        let caps = ctx.caps();
        let limit = caps.max_texture_dimension_2d;
        for (w, h) in [(src_width, src_height), (width, height)] {
            if w > limit || h > limit {
                return Err(fault(Error::TooLarge { width: w, height: h, limit }));
            }
        }
        // The read-back buffer with its padded rows is the largest buffer a device run makes, so
        // the null context refuses the same jobs a device with these caps does.
        let needed = layout.buffer_size.max(layout.frame_bytes() as u64);
        if needed > caps.max_buffer_size {
            return Err(fault(Error::BufferLimit { needed, limit: caps.max_buffer_size }));
        }
        // The input's size is checked before its bytes are read.
        let bytes = ctx.buffer_len(*input)?;
        let expected = (src_width as usize).checked_mul(src_height as usize).and_then(|n| n.checked_mul(4));
        if src_width == 0 || src_height == 0 || expected != Some(bytes) {
            return Err(fault(Error::Source { width: src_width, height: src_height, bytes }));
        }
        let rgba = ctx.read_buffer(*input)?;
        let frame = self.convert(&rgba, &spec).map_err(fault)?;
        let output = ctx.upload_buffer("craft.yuv.frame", &frame)?;
        Ok(GpuJobOutcome { outputs: vec![output], data: None })
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod device {
    use std::sync::Mutex;

    use crate::gpu::{Converter, Planes};
    use crate::spec::{Conversion, Error};

    use super::YuvJob;

    /// A device, its queue, and the converters made on it so far, one per conversion.
    pub(super) struct DeviceRunner {
        device: wgpu::Device,
        queue: wgpu::Queue,
        converters: Mutex<Vec<Converter>>,
    }

    fn device_error(e: impl std::fmt::Display) -> Error {
        Error::Device(e.to_string())
    }

    impl DeviceRunner {
        pub(super) fn new(device: wgpu::Device, queue: wgpu::Queue) -> DeviceRunner {
            DeviceRunner { device, queue, converters: Mutex::new(Vec::new()) }
        }

        /// `rgba` converted on the device, as one packed frame. Errors the device reports go to
        /// error scopes and come back as [`Error::Device`].
        pub(super) fn convert(&self, rgba: &[u8], job: &YuvJob) -> Result<Vec<u8>, Error> {
            let (sw, sh) = (job.src_width, job.src_height);
            let limit = self.device.limits().max_texture_dimension_2d;
            if sw > limit || sh > limit {
                return Err(Error::TooLarge { width: sw, height: sh, limit });
            }
            // A converter leaves the cache while a job uses it, and goes back only when the device
            // reported no error, so a converter whose pipelines failed is not used again.
            let mut converters = self.converters.lock().map_err(|_| device_error("the converter cache lock is poisoned"))?;
            let cached = take(&mut converters, job.conversion);
            drop(converters);
            let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let converter = cached.unwrap_or_else(|| Converter::new(&self.device, job.conversion));
            let recorded = self.record(&converter, rgba, job);
            // Both scopes are popped on every path, so that none stays open on the device.
            let errors = [pollster::block_on(validation.pop()), pollster::block_on(oom.pop())];
            if let Some(e) = errors.into_iter().flatten().next() {
                return Err(device_error(e));
            }
            if let Ok(mut converters) = self.converters.lock() {
                converters.push(converter);
            }
            let (planes, buffer) = recorded?;
            let (tx, rx) = std::sync::mpsc::channel();
            buffer.map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = tx.send(result);
            });
            self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(device_error)?;
            rx.try_recv().map_err(|_| device_error("the read-back map did not complete"))?.map_err(device_error)?;
            let mapped = buffer.get_mapped_range(..).map_err(device_error)?;
            let mut frame = vec![0u8; planes.layout().frame_bytes()];
            let unpacked = planes.layout().unpack(&mapped, &mut frame);
            drop(mapped);
            buffer.unmap();
            unpacked.map(|()| frame)
        }

        /// Upload `rgba` and record and submit the passes and the copy into a read-back buffer.
        fn record(&self, converter: &Converter, rgba: &[u8], job: &YuvJob) -> Result<(Planes, wgpu::Buffer), Error> {
            let (sw, sh) = (job.src_width, job.src_height);
            let size = wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 };
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("craft.yuv source"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(sw * 4), rows_per_image: Some(sh) };
            self.queue.write_texture(texture.as_image_copy(), rgba, layout, size);
            let source = converter.bind(&self.device, &texture.create_view(&wgpu::TextureViewDescriptor::default()));
            let planes = converter.planes(&self.device, job.width, job.height)?;
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("craft.yuv read-back"),
                size: planes.layout().buffer_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("craft.yuv") });
            converter.draw(&mut encoder, &source, &planes)?;
            planes.copy_to_buffer(&mut encoder, &buffer, 0)?;
            self.queue.submit([encoder.finish()]);
            Ok((planes, buffer))
        }
    }

    /// Remove and return the converter of `conversion` from `converters`, if there is one.
    fn take(converters: &mut Vec<Converter>, conversion: Conversion) -> Option<Converter> {
        let i = converters.iter().position(|c| c.conversion() == conversion)?;
        Some(converters.swap_remove(i))
    }
}

#[cfg(target_arch = "wasm32")]
mod device {
    use crate::spec::Error;

    use super::YuvJob;

    /// No device runs jobs on `wasm32`: [`super::YuvProvider::with_device`] is native only.
    pub(super) enum DeviceRunner {}

    impl DeviceRunner {
        pub(super) fn convert(&self, _rgba: &[u8], _job: &YuvJob) -> Result<Vec<u8>, Error> {
            match *self {}
        }
    }
}
