//! The filters as a `craft-gpu` job: [`FilterProvider`] answers [`GpuJob`]s whose provider id is
//! [`PROVIDER_ID`], dispatched through a [`craft_gpu::ProviderRegistry`].
//!
//! A job's inputs are RGBA8 buffers of `width × height × 4` bytes in the context: one image, or
//! for [`Filter::Composite`] the backdrop and then the layer. Its parameters are
//! [`FilterJob::params`]. Its outcome has one output, a new buffer holding the filtered image. The
//! image and the largest buffer the filter makes are held to the context's
//! [`DeviceCaps`](craft_gpu::DeviceCaps) buffer limits on every context, so a job the null
//! context runs also runs on a device with the same caps.

use craft_gpu::{GpuContext, GpuFault, GpuJob, GpuJobOutcome, GpuJobParams, GpuProvider, HandleId};

use crate::cpu;
use crate::spec::{self, Error, Filter, Lut};

/// The provider id of [`FilterProvider`].
pub const PROVIDER_ID: &str = "craft.filters";

/// The longest parameter text a job may carry; a lookup-table job, the longest, needs about 2100
/// bytes.
const MAX_PARAMS: usize = 4096;

/// Every parameter key, in the order [`FilterJob::params`] writes them.
const KEYS: [&str; 9] = ["op", "w", "h", "sigma", "opacity", "r", "g", "b", "a"];
const OP: usize = 0;
const W: usize = 1;
const H: usize = 2;
const SIGMA: usize = 3;
const OPACITY: usize = 4;
const TABLES: [usize; 4] = [5, 6, 7, 8];

/// One `craft.filters` job: a filter and the size of its images.
///
/// The parameters are ASCII `key:value` fields separated by `;`: `op` (`blur`, `lut` or `over`),
/// `w` and `h` in decimal digits, then `sigma` for `blur`, `opacity` for `over`, or `r`, `g`, `b`
/// and `a` for `lut`, each table as 512 hexadecimal digits. For example
/// `op:blur;w:1920;h:1080;sigma:2.5`. A missing, repeated, unknown or inapplicable key, or a value
/// out of range (a NaN or negative sigma, an opacity above 1), is [`GpuFault::InvalidJob`].
#[derive(Clone, Debug, PartialEq)]
pub struct FilterJob {
    pub filter: Filter,
    pub width: u32,
    pub height: u32,
}

fn invalid(message: impl std::fmt::Display) -> GpuFault {
    GpuFault::InvalidJob(format!("{PROVIDER_ID}: {message}"))
}

/// A filter error as a fault: device errors and failed allocations are [`GpuFault::Operation`],
/// the rest describe a job that cannot run and are [`GpuFault::InvalidJob`].
fn fault(e: Error) -> GpuFault {
    match e {
        Error::Device(_) | Error::Memory { .. } => GpuFault::Operation(format!("{PROVIDER_ID}: {e}")),
        _ => invalid(e),
    }
}

fn hex(table: &[u8; 256]) -> String {
    table.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 256]> {
    let bytes = text.as_bytes();
    if bytes.len() != 512 {
        return None;
    }
    let digit = |b: u8| (b as char).to_digit(16);
    let mut table = [0u8; 256];
    for (v, [hi, lo]) in table.iter_mut().zip(bytes.as_chunks::<2>().0) {
        *v = (digit(*hi)? * 16 + digit(*lo)?) as u8;
    }
    Some(table)
}

impl FilterJob {
    pub fn new(filter: Filter, width: u32, height: u32) -> FilterJob {
        FilterJob { filter, width, height }
    }

    /// The job's parameters.
    pub fn params(&self) -> GpuJobParams {
        let (w, h) = (self.width, self.height);
        let text = match &self.filter {
            Filter::GaussianBlur { sigma } => format!("op:blur;w:{w};h:{h};sigma:{sigma}"),
            Filter::Composite { opacity } => format!("op:over;w:{w};h:{h};opacity:{opacity}"),
            Filter::Lut(lut) => {
                let [r, g, b, a] = &lut.tables;
                format!("op:lut;w:{w};h:{h};r:{};g:{};b:{};a:{}", hex(r), hex(g), hex(b), hex(a))
            }
        };
        GpuJobParams(text.into_bytes())
    }

    /// A `craft.filters` job on the buffers `inputs`.
    pub fn job(&self, inputs: &[HandleId]) -> GpuJob {
        GpuJob { provider_id: PROVIDER_ID.into(), params: self.params(), inputs: inputs.to_vec(), outputs: Vec::new() }
    }

    /// Read parameters written by [`FilterJob::params`].
    pub fn parse(params: &GpuJobParams) -> Result<FilterJob, GpuFault> {
        if params.0.len() > MAX_PARAMS {
            return Err(invalid(format!("the parameters are {} bytes, more than {MAX_PARAMS}", params.0.len())));
        }
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
        let bad = |i: usize, value: &str| invalid(format!("`{value}` is not a value of `{}`", KEYS[i]));
        let side = |i: usize| -> Result<u32, GpuFault> {
            let v = get(i)?;
            if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad(i, v));
            }
            v.parse().map_err(|_| bad(i, v))
        };
        let number = |i: usize| -> Result<f32, GpuFault> {
            let v = get(i)?;
            v.parse::<f32>().map_err(|_| bad(i, v))
        };
        let (filter, keys): (Filter, &[usize]) = match get(OP)? {
            "blur" => (Filter::GaussianBlur { sigma: number(SIGMA)? }, &[SIGMA]),
            "over" => (Filter::Composite { opacity: number(OPACITY)? }, &[OPACITY]),
            "lut" => {
                let mut tables = [[0u8; 256]; 4];
                for (table, &i) in tables.iter_mut().zip(&TABLES) {
                    let v = get(i)?;
                    *table = unhex(v).ok_or_else(|| invalid(format!("`{}` is not 512 hexadecimal digits", KEYS[i])))?;
                }
                (Filter::Lut(Box::new(Lut { tables })), &TABLES)
            }
            v => return Err(bad(OP, v)),
        };
        for (i, value) in values.iter().enumerate() {
            if value.is_some() && ![OP, W, H].contains(&i) && !keys.contains(&i) {
                return Err(invalid(format!("`{}` does not apply to `{}`", KEYS[i], get(OP)?)));
            }
        }
        filter.validate().map_err(fault)?;
        Ok(FilterJob { filter, width: side(W)?, height: side(H)? })
    }
}

/// The `craft.filters` provider.
///
/// [`FilterProvider::new`] filters with [`cpu::apply`] on any context: the null context in tests
/// and CI, or a host with no device. [`FilterProvider::with_device`] filters with the compute
/// passes on a device the caller holds, such as the one it already draws with; the inputs and the
/// output still move through the context's buffers. Both give the same image within the
/// tolerances the crate README states.
pub struct FilterProvider {
    device: Option<device::Runner>,
}

impl Default for FilterProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FilterProvider {
    /// A provider that filters on the CPU.
    pub fn new() -> FilterProvider {
        FilterProvider { device: None }
    }

    /// A provider that filters with the compute passes on `device`. Each job waits for the device
    /// to finish, so this is not available on `wasm32`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue) -> FilterProvider {
        FilterProvider { device: Some(device::Runner(crate::gpu::GpuFilters::new(device, queue))) }
    }

    /// Whether jobs run on a device.
    pub fn has_device(&self) -> bool {
        self.device.is_some()
    }

    fn apply(&self, job: &FilterJob, inputs: &[&[u8]]) -> Result<Vec<u8>, Error> {
        match &self.device {
            Some(d) => d.apply(job, inputs),
            None => cpu::apply(&job.filter, job.width, job.height, inputs),
        }
    }
}

impl GpuProvider for FilterProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn run(&self, ctx: &mut dyn GpuContext, job: &GpuJob) -> Result<GpuJobOutcome, GpuFault> {
        let spec = FilterJob::parse(&job.params)?;
        if job.inputs.len() != spec.filter.inputs() {
            return Err(fault(Error::Inputs { expected: spec.filter.inputs(), got: job.inputs.len() }));
        }
        if !job.outputs.is_empty() {
            return Err(invalid("makes its own output buffer, so the job names no outputs"));
        }
        let pixels = spec::pixels(spec.width, spec.height).map_err(fault)?;
        // The largest buffer a device run makes is held to these caps on every context, so the
        // null context refuses the jobs a device with the same caps refuses.
        let caps = ctx.caps();
        let limit = caps.max_buffer_size.min(caps.max_storage_buffer_binding_size);
        let needed = spec.filter.largest_buffer(pixels);
        if needed > limit {
            return Err(fault(Error::BufferLimit { needed, limit }));
        }
        // Every input's size is checked before any bytes are read.
        let expected = pixels * 4;
        for input in &job.inputs {
            let bytes = ctx.buffer_len(*input)? as u64;
            if bytes != expected {
                return Err(fault(Error::Source { width: spec.width, height: spec.height, expected, bytes }));
            }
        }
        let mut images = Vec::with_capacity(job.inputs.len());
        for input in &job.inputs {
            images.push(ctx.read_buffer(*input)?);
        }
        let inputs: Vec<&[u8]> = images.iter().map(Vec::as_slice).collect();
        let out = self.apply(&spec, &inputs).map_err(fault)?;
        let output = ctx.upload_buffer("craft.filters.out", &out)?;
        Ok(GpuJobOutcome { outputs: vec![output], data: None })
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod device {
    use crate::gpu::GpuFilters;
    use crate::spec::Error;

    use super::FilterJob;

    pub(super) struct Runner(pub(super) GpuFilters);

    impl Runner {
        pub(super) fn apply(&self, job: &FilterJob, inputs: &[&[u8]]) -> Result<Vec<u8>, Error> {
            self.0.apply(&job.filter, job.width, job.height, inputs)
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod device {
    use crate::spec::Error;

    use super::FilterJob;

    /// No device runs jobs on `wasm32`: [`super::FilterProvider::with_device`] is native only.
    pub(super) enum Runner {}

    impl Runner {
        pub(super) fn apply(&self, _job: &FilterJob, _inputs: &[&[u8]]) -> Result<Vec<u8>, Error> {
            match *self {}
        }
    }
}
