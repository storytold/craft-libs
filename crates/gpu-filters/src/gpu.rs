//! The filters as wgpu compute passes on a device the caller holds.
//!
//! Images move as storage buffers of packed RGBA8 words, so the size limits are the device's
//! buffer limits, checked before anything is allocated. Every device call runs inside error scopes:
//! a validation, out-of-memory or internal error comes back as [`Error::Device`], never as a panic
//! from wgpu's uncaptured-error handler.

use std::sync::Mutex;

use crate::spec::{self, Error, Filter};

const BLUR_WGSL: &str = include_str!("shaders/blur.wgsl");
const LUT_WGSL: &str = include_str!("shaders/lut.wgsl");
const COMPOSITE_WGSL: &str = include_str!("shaders/composite.wgsl");

/// The side of a workgroup in the shaders.
const GROUP: u32 = 16;

struct Pipelines {
    blur_h: wgpu::ComputePipeline,
    blur_v: wgpu::ComputePipeline,
    lut: wgpu::ComputePipeline,
    over: wgpu::ComputePipeline,
}

impl Pipelines {
    fn new(device: &wgpu::Device) -> Pipelines {
        let module = |label: &str, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(source.into()) })
        };
        let blur = module("craft.filters blur", BLUR_WGSL);
        let lut = module("craft.filters lut", LUT_WGSL);
        let composite = module("craft.filters composite", COMPOSITE_WGSL);
        let pipeline = |module: &wgpu::ShaderModule, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        Pipelines {
            blur_h: pipeline(&blur, "blur_h"),
            blur_v: pipeline(&blur, "blur_v"),
            lut: pipeline(&lut, "lut"),
            over: pipeline(&composite, "over"),
        }
    }
}

fn device_error(e: impl std::fmt::Display) -> Error {
    Error::Device(e.to_string())
}

/// The filters' compute pipelines on one device, made on first use.
///
/// [`GpuFilters::apply`] uploads the inputs, records and submits the passes and waits for the
/// result, so it is native only: on `wasm32` a buffer map completes only from the browser's event
/// loop. It gives the bytes [`crate::cpu::apply`] gives, within the tolerances the crate README
/// states.
pub struct GpuFilters {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: Mutex<Option<Pipelines>>,
}

impl GpuFilters {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> GpuFilters {
        GpuFilters { device, queue, pipelines: Mutex::new(None) }
    }

    /// The largest buffer the device lets a filter bind: the smaller of its buffer size and
    /// storage binding limits.
    pub fn buffer_limit(&self) -> u64 {
        let limits = self.device.limits();
        limits.max_buffer_size.min(limits.max_storage_buffer_binding_size)
    }

    /// `filter` applied to `inputs` on the device, with the inputs and the result as tightly
    /// packed `width × height` RGBA8 images (see [`crate::cpu::apply`]).
    pub fn apply(&self, filter: &Filter, width: u32, height: u32, inputs: &[&[u8]]) -> Result<Vec<u8>, Error> {
        let pixels = spec::check(filter, width, height, inputs)?;
        let needed = filter.largest_buffer(pixels);
        let limit = self.buffer_limit();
        if needed > limit {
            return Err(Error::BufferLimit { needed, limit });
        }
        let groups = self.device.limits().max_compute_workgroups_per_dimension;
        if width.div_ceil(GROUP) > groups || height.div_ceil(GROUP) > groups {
            return Err(Error::TooLarge { width, height });
        }
        // The pipelines leave the cache while a run uses them and go back only when the device
        // reported no error, so pipelines that failed are not used again.
        let cached = self.pipelines.lock().map_err(|_| device_error("the pipeline cache lock is poisoned"))?.take();
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let pipelines = cached.unwrap_or_else(|| Pipelines::new(&self.device));
        let readback = self.record(&pipelines, filter, width, height, inputs);
        // Every scope is popped on every path, the last pushed first.
        let errors = [pollster::block_on(internal.pop()), pollster::block_on(validation.pop()), pollster::block_on(oom.pop())];
        if let Some(e) = errors.into_iter().flatten().next() {
            return Err(device_error(e));
        }
        if let Ok(mut slot) = self.pipelines.lock() {
            *slot = Some(pipelines);
        }
        let readback = readback?;
        let (tx, rx) = std::sync::mpsc::channel();
        readback.map_async(wgpu::MapMode::Read, .., move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(device_error)?;
        rx.try_recv().map_err(|_| device_error("the read-back map did not complete"))?.map_err(device_error)?;
        let bytes = (pixels * 4) as usize;
        let mut out: Vec<u8> = spec::zeroed(bytes)?;
        {
            let mapped = readback.get_mapped_range(..).map_err(device_error)?;
            let result = mapped.get(..bytes).ok_or_else(|| device_error("the read-back buffer is shorter than the image"))?;
            out.copy_from_slice(result);
        }
        readback.unmap();
        Ok(out)
    }

    fn storage(&self, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, bytes);
        buffer
    }

    fn empty(&self, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
    }

    fn bind(&self, pipeline: &wgpu::ComputePipeline, entries: &[(u32, &wgpu::Buffer)]) -> wgpu::BindGroup {
        let entries: Vec<wgpu::BindGroupEntry<'_>> =
            entries.iter().map(|(binding, buffer)| wgpu::BindGroupEntry { binding: *binding, resource: buffer.as_entire_binding() }).collect();
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &entries })
    }

    /// Upload the inputs, record and submit the passes and the copy into a read-back buffer, and
    /// return that buffer.
    fn record(&self, pipelines: &Pipelines, filter: &Filter, width: u32, height: u32, inputs: &[&[u8]]) -> Result<wgpu::Buffer, Error> {
        let image = u64::from(width) * u64::from(height) * 4;
        let (radius, extra) = match filter {
            Filter::GaussianBlur { sigma } => ((spec::kernel(*sigma)?.len() as u32).saturating_sub(1), 0),
            Filter::Lut(_) => (0, 0),
            Filter::Composite { opacity } => (0, spec::opacity_weight(*opacity)),
        };
        let params: Vec<u8> = [width, height, radius, extra].iter().flat_map(|v| v.to_le_bytes()).collect();
        let uniform = self.empty("craft.filters params", params.len() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        self.queue.write_buffer(&uniform, 0, &params);
        let dst = self.empty("craft.filters out", image, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("craft.filters") });
        let dispatch = |encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline, group: &wgpu::BindGroup| {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.dispatch_workgroups(width.div_ceil(GROUP), height.div_ceil(GROUP), 1);
        };
        match (filter, inputs) {
            (Filter::GaussianBlur { sigma }, [src]) => {
                let weights: Vec<u8> = spec::kernel(*sigma)?.iter().flat_map(|w| w.to_le_bytes()).collect();
                let weights = self.storage("craft.filters weights", &weights);
                let src = self.storage("craft.filters in", src);
                let mid = self.empty("craft.filters mid", image * 2, wgpu::BufferUsages::STORAGE);
                let h = self.bind(&pipelines.blur_h, &[(0, &uniform), (1, &weights), (2, &src), (3, &mid)]);
                let v = self.bind(&pipelines.blur_v, &[(0, &uniform), (1, &weights), (3, &mid), (4, &dst)]);
                dispatch(&mut encoder, &pipelines.blur_h, &h);
                dispatch(&mut encoder, &pipelines.blur_v, &v);
            }
            (Filter::Lut(lut), [src]) => {
                let tables: Vec<u8> = lut.tables.iter().flatten().flat_map(|&v| u32::from(v).to_le_bytes()).collect();
                let tables = self.storage("craft.filters tables", &tables);
                let src = self.storage("craft.filters in", src);
                let group = self.bind(&pipelines.lut, &[(0, &uniform), (1, &tables), (2, &src), (3, &dst)]);
                dispatch(&mut encoder, &pipelines.lut, &group);
            }
            (Filter::Composite { .. }, [backdrop, layer]) => {
                let backdrop = self.storage("craft.filters backdrop", backdrop);
                let layer = self.storage("craft.filters layer", layer);
                let group = self.bind(&pipelines.over, &[(0, &uniform), (1, &backdrop), (2, &layer), (3, &dst)]);
                dispatch(&mut encoder, &pipelines.over, &group);
            }
            _ => return Err(Error::Inputs { expected: filter.inputs(), got: inputs.len() }),
        }
        let readback = self.empty("craft.filters read-back", image, wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ);
        encoder.copy_buffer_to_buffer(&dst, 0, &readback, 0, image);
        self.queue.submit([encoder.finish()]);
        Ok(readback)
    }
}
