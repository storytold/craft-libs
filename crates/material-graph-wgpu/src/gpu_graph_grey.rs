//! Packed grayscale graph passes. Whole images can feed later GPU nodes, while bounded stripes support larger jobs.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use craft_material_graph::accelerator::{GreyInput, GreyPass, ResidentGrey16, ResidentGreyPass, ResidentGreyPixels, packed_blend_mode};
use craft_material_graph::context::Cancel;
use craft_material_graph::tex::Grey16;
use wgpu::util::DeviceExt;

const BAND_BYTES: u64 = 8 << 20;
const WORKGROUP: u32 = 8;
const ENLARGE: u32 = 0;
const REDUCE: u32 = 1;
const LEVELS: u32 = 2;
const BLEND_OFFSET: u32 = 3;
const SHADER: &str = concat!(
    include_str!("shaders/graph_grey.wgsl"),
    "\n",
    include_str!("shaders/graph_compiled_grey.wgsl"),
    "\n",
    include_str!("shaders/graph_blend.wgsl")
);

pub(crate) struct GreyRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    memory_budget_bytes: fn() -> u64,
    pipeline: wgpu::ComputePipeline,
    resident_bytes: Arc<AtomicU64>,
}

impl GreyRasterizer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue, memory_budget_bytes: fn() -> u64) -> Result<Self, String> {
        let pipeline = crate::validation::scoped(device, "graph grayscale shader", || {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("graph grayscale"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("graph grayscale"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        })?;
        Ok(Self { device: device.clone(), queue: queue.clone(), memory_budget_bytes, pipeline, resident_bytes: Arc::default() })
    }

    fn resident_storage_limit(&self) -> u64 {
        let limits = self.device.limits();
        limits.max_buffer_size.min(limits.max_storage_buffer_binding_size)
    }

    /// Charge a storage buffer, which must also fit one storage binding, against the resident budget.
    pub(crate) fn reserve(&self, bytes: u64) -> Option<Arc<ResidentBytes>> {
        if bytes > self.resident_storage_limit() {
            return None;
        }
        self.reserve_texture(bytes)
    }

    /// Charge a texture against the resident budget. The caller checks the texture dimension limits.
    pub(crate) fn reserve_texture(&self, bytes: u64) -> Option<Arc<ResidentBytes>> {
        let storage = self.resident_storage_limit();
        let buffers = if self.device.adapter_info().device_type == wgpu::DeviceType::DiscreteGpu { 6 } else { 3 };
        let cap = ((self.memory_budget_bytes)() / 8).min(storage.saturating_mul(buffers));
        self.resident_bytes.try_update(Ordering::Relaxed, Ordering::Relaxed, |used| used.checked_add(bytes).filter(|&used| used <= cap)).ok()?;
        Some(Arc::new(ResidentBytes { total: self.resident_bytes.clone(), bytes }))
    }

    pub(crate) fn allocate_resident(&self, w: usize, h: usize) -> Result<Option<Arc<GreyDevicePixels>>, String> {
        let Some(bytes) = packed_bytes(w, h) else { return Ok(None) };
        let Some(reservation) = self.reserve(bytes) else { return Ok(None) };
        let buffer = crate::validation::scoped(&self.device, "resident graph grayscale buffer", || {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("resident graph grayscale buffer"),
                size: bytes,
                usage: RESIDENT_BUFFER_USAGE,
                mapped_at_creation: false,
            })
        })?;
        Ok(Some(Arc::new(self.resident_pixels(buffer, reservation))))
    }

    fn resident_pixels(&self, buffer: wgpu::Buffer, reservation: Arc<ResidentBytes>) -> GreyDevicePixels {
        GreyDevicePixels { device: self.device.clone(), queue: self.queue.clone(), buffer, reservation }
    }

    pub(crate) fn bind_input(&self, input: GreyInput<'_>) -> Result<Option<GreyDevicePixels>, String> {
        let host = match input {
            GreyInput::Host(host) => host,
            GreyInput::Device(image) => {
                return Ok(GreyDevicePixels::from_resident(image).filter(|pixels| pixels.device == self.device).cloned());
            }
        };
        if !packed_bytes(host.w, host.h).is_some_and(|bytes| bytes / 2 == host.px.len() as u64) {
            return Ok(None);
        }
        self.upload_pixels(bytemuck::cast_slice(&host.px))
    }

    pub(crate) fn upload_pixels(&self, pixels: &[u8]) -> Result<Option<GreyDevicePixels>, String> {
        let bytes = pixels.len() as u64;
        let Some(reservation) = self.reserve(bytes) else { return Ok(None) };
        let buffer = crate::validation::scoped(&self.device, "resident graph pixel upload", || {
            self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("resident graph pixel upload"),
                contents: pixels,
                usage: RESIDENT_BUFFER_USAGE,
            })
        })?;
        Ok(Some(self.resident_pixels(buffer, reservation)))
    }

    pub(crate) fn render_resident(&self, pass: ResidentGreyPass<'_>, cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        let (foreground, background, mask, side, mode, opacity, blend_alpha) = match pass {
            ResidentGreyPass::Enlarge { source, side } => (source, None, None, side, ENLARGE, 0.0, None),
            ResidentGreyPass::Reduce { source, side } => (source, None, None, side, REDUCE, 0.0, None),
            ResidentGreyPass::Levels { source, curve, div, alpha } => return self.render_levels(source, &curve, div, alpha, cancel),
            ResidentGreyPass::Blend { foreground, background, mask, mode, opacity, alpha } => {
                let (side, _) = grey_input_shape(&foreground, false);
                if !opacity.is_finite()
                    || !packed_blend_mode(mode)
                    || !craft_material_graph::tex::grey_alpha_ok(alpha)
                    || mask.as_ref().is_some_and(|mask| grey_input_shape(mask, false).0 != side)
                {
                    return Ok(None);
                }
                (foreground, Some(background), mask, side, BLEND_OFFSET + mode, opacity, Some(alpha))
            }
        };
        let (source_side, alpha) = grey_input_shape(&foreground, matches!(mode, ENLARGE | REDUCE));
        if cancel.is_cancelled() || !(2..=8192).contains(&side) || !side.is_power_of_two() || source_side == 0 {
            return Ok(None);
        }
        if let Some(background) = &background {
            if grey_input_shape(background, false).0 != side {
                return Ok(None);
            }
        } else if (mode == ENLARGE && source_side >= side) || (mode == REDUCE && source_side <= side) {
            return Ok(None);
        }
        let alpha = blend_alpha.unwrap_or(alpha);
        let limit = self.device.limits();
        let largest_output_side = if mode == REDUCE { source_side / 2 } else { side };
        if (largest_output_side as u32).div_ceil(WORKGROUP) > limit.max_compute_workgroups_per_dimension {
            return Ok(None);
        }
        // Poll without waiting so completed submissions can release reservations before trying to reserve more.
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let Some(front) = self.bind_input(foreground)? else { return Ok(None) };
        let back = match background {
            Some(image) => {
                let Some(bound) = self.bind_input(image)? else { return Ok(None) };
                Some(bound)
            }
            None => None,
        };
        let mask = match mask {
            Some(image) => {
                let Some(bound) = self.bind_input(image)? else { return Ok(None) };
                Some(bound)
            }
            None => None,
        };
        let mut stages = Vec::new();
        let next_side = |size| match mode {
            ENLARGE => size * 2,
            REDUCE => size / 2,
            _ => side,
        };
        let mut size = next_side(source_side);
        loop {
            let Some(output) = self.allocate_resident(size, size)? else { return Ok(None) };
            stages.push((size, output));
            if size == side {
                break;
            }
            size = next_side(size);
        }
        if cancel.is_cancelled() {
            return Ok(None);
        }

        crate::validation::scoped(&self.device, "resident graph grayscale pass", || {
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("resident graph grayscale") });
            // Resize an 8-bit source into 16-bit steps. Every subsequent pass reads those new steps.
            let source_scale = if foreground.info().2 == 255.0 { 257 } else { 1 };
            let mut input = (source_side, &front, source_scale);
            for (size, output) in &stages {
                let back = &back.as_ref().unwrap_or(input.1).buffer; // Resizing does not read this binding.
                self.encode_resident(&mut encoder, input, (back, mask.as_ref().map(|mask| &mask.buffer)), (*size, output), mode, opacity);
                input = (*size, output, 1);
            }
            self.queue.submit([encoder.finish()]);
        })?;
        front.retain_until_completed(&self.queue);
        if let Some(back) = back {
            back.retain_until_completed(&self.queue);
        }
        if let Some(mask) = mask {
            mask.retain_until_completed(&self.queue);
        }
        for (_, pixels) in &stages {
            pixels.retain_until_completed(&self.queue);
        }
        Ok(stages.pop().map(|(_, pixels)| ResidentGrey16 { w: side, h: side, div: 65535.0, alpha, pixels }))
    }

    fn render_levels(&self, source: GreyInput<'_>, curve: &[u16], div: f32, alpha: f32, cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        let (w, h, source_div, _) = source.info();
        let limit = self.device.limits().max_compute_workgroups_per_dimension as usize;
        if cancel.is_cancelled() || w != h || curve.len() != source_div as usize + 1 || w.div_ceil(WORKGROUP as usize) > limit {
            return Ok(None);
        }
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let Some(front) = self.bind_input(source)? else { return Ok(None) };
        let Some(output) = self.allocate_resident(w, h)? else { return Ok(None) };
        crate::validation::scoped(&self.device, "resident graph levels pass", || {
            let table = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("graph levels lookup table"),
                contents: bytemuck::cast_slice(curve),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("resident graph levels") });
            self.encode_resident(&mut encoder, (w, &front, 1), (&table, None), (w, &output), LEVELS, 0.0);
            self.queue.submit([encoder.finish()]);
        })?;
        front.retain_until_completed(&self.queue);
        output.retain_until_completed(&self.queue);
        Ok(Some(ResidentGrey16 { w, h, div, alpha, pixels: output }))
    }

    fn encode_resident(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        (source_side, front, source_scale): (usize, &GreyDevicePixels, u32),
        (back, mask): (&wgpu::Buffer, Option<&wgpu::Buffer>),
        (side, output): (usize, &GreyDevicePixels),
        mode: u32,
        opacity: f32,
    ) {
        let params = [side as u32, side as u32, source_side as u32, mode, opacity.to_bits(), 1, u32::from(mask.is_some()), source_scale];
        let uniform = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("resident graph grayscale parameters"),
            contents: bytemuck::cast_slice(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let entries = [&uniform, &front.buffer, back, &output.buffer, mask.unwrap_or(&front.buffer)]
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry { binding: binding as u32, resource: buffer.as_entire_binding() })
            .collect::<Vec<_>>();
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resident graph grayscale inputs"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &entries,
        });
        let side = side as u32;
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &binding, &[]);
        pass.dispatch_workgroups((side / 2).div_ceil(WORKGROUP), side.div_ceil(WORKGROUP), 1);
    }

    pub(crate) fn render(&self, pass: GreyPass<'_>, cancel: &Cancel) -> Result<Option<Grey16>, String> {
        let side = match &pass {
            GreyPass::Enlarge { side, .. } => *side,
            GreyPass::Blend { foreground, .. } => foreground.w,
        };
        if !(4..=8192).contains(&side) || !side.is_power_of_two() || cancel.is_cancelled() {
            return Ok(None);
        }
        let limit = self.device.limits();
        let budget = limit.max_buffer_size.min(limit.max_storage_buffer_binding_size).min(BAND_BYTES);
        // Doublings need an even band height so local row parity is also global row parity.
        let max_rows = (budget / (side as u64 * 2)).min(u64::from(limit.max_compute_workgroups_per_dimension * WORKGROUP));
        let band_rows = side.min(max_rows as usize / 2 * 2);
        if band_rows < 2 || (side as u32 / 2).div_ceil(WORKGROUP) > limit.max_compute_workgroups_per_dimension {
            return Ok(None);
        }
        let bands = Bands::new(self, band_rows, (side * band_rows * 2) as u64)?;
        match pass {
            GreyPass::Enlarge { source, side } => {
                if source.w < 2
                    || source.w >= side
                    || source.h != source.w
                    || !source.w.is_power_of_two()
                    || !matches!(source.div, 255.0 | 65535.0)
                    || source.px.len() != source.w * source.h
                {
                    return Ok(None);
                }
                let mut doubled = bands.run(source, None, 0, 0.0, source.w * 2, cancel)?;
                while doubled.w < side {
                    doubled = bands.run(&doubled, None, 0, 0.0, doubled.w * 2, cancel)?;
                }
                Ok(Some(doubled))
            }
            GreyPass::Blend { foreground, background, mode, opacity } => {
                if foreground.h != side || background.w != side || background.h != side || !matches!(mode, 3 | 6 | 32..=34) || !opacity.is_finite() {
                    return Ok(None);
                }
                bands.run(foreground, Some(background), BLEND_OFFSET + mode, opacity, side, cancel).map(Some)
            }
        }
    }
}

const RESIDENT_BUFFER_USAGE: wgpu::BufferUsages = wgpu::BufferUsages::STORAGE.union(wgpu::BufferUsages::COPY_SRC).union(wgpu::BufferUsages::COPY_DST);

fn packed_bytes(w: usize, h: usize) -> Option<u64> {
    // A row is addressable both as pairs of u16 on the GPU and by byte offset when read back.
    if w == 0 || h == 0 || !w.is_multiple_of(2) {
        return None;
    }
    (w as u64).checked_mul(h as u64)?.checked_mul(2)
}

fn grey_input_shape(input: &GreyInput<'_>, accepts_8_bit: bool) -> (usize, f32) {
    let (w, h, div, alpha) = input.info();
    (if (2..=8192).contains(&w) && w == h && w.is_power_of_two() && (div == 65535.0 || accepts_8_bit && div == 255.0) { w } else { 0 }, alpha)
}

#[derive(Debug)]
pub(crate) struct ResidentBytes {
    total: Arc<AtomicU64>,
    bytes: u64,
}

impl Drop for ResidentBytes {
    fn drop(&mut self) {
        self.total.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

/// GPU storage for a grayscale node, in row order with two u16 samples per word.
#[derive(Clone, Debug)]
pub(crate) struct GreyDevicePixels {
    pub(crate) buffer: wgpu::Buffer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    reservation: Arc<ResidentBytes>,
}

impl GreyDevicePixels {
    pub(crate) fn from_resident(image: &ResidentGrey16) -> Option<&Self> {
        Self::from_pixels(image.pixels.as_ref(), packed_bytes(image.w, image.h)?)
    }

    pub(crate) fn from_pixels(image: &dyn ResidentGreyPixels, bytes: u64) -> Option<&Self> {
        let pixels = image.as_any().downcast_ref::<Self>()?;
        (pixels.buffer.size() == bytes).then_some(pixels)
    }

    pub(crate) fn belongs_to(&self, device: &wgpu::Device) -> bool {
        &self.device == device
    }

    pub(crate) fn retain_until_completed(&self, queue: &wgpu::Queue) {
        let reservation = self.reservation.clone();
        queue.on_submitted_work_done(move || drop(reservation));
    }
}

impl ResidentGreyPixels for GreyDevicePixels {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn read(&self, cancel: &Cancel) -> Result<Vec<u16>, String> {
        if cancel.is_cancelled() {
            return Err("GPU grayscale readback was cancelled".into());
        }
        let size = self.buffer.size();
        let band = size.min(BAND_BYTES);
        let staging = crate::validation::scoped(&self.device, "resident graph grayscale readback", || {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("resident graph grayscale readback"),
                size: band,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })?;
        let mut pixels = Vec::with_capacity((size / 2) as usize);
        for offset in (0..size).step_by(band as usize) {
            if cancel.is_cancelled() {
                return Err("GPU grayscale readback was cancelled".into());
            }
            let bytes = (size - offset).min(band);
            let submission = crate::validation::scoped(&self.device, "resident graph grayscale copy", || {
                let mut encoder =
                    self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("resident graph grayscale copy") });
                encoder.copy_buffer_to_buffer(&self.buffer, offset, &staging, 0, bytes);
                self.queue.submit([encoder.finish()])
            })?;
            read_words(&self.device, &staging, bytes, submission, &mut pixels)?;
        }
        Ok(pixels)
    }
}

fn read_words(
    device: &wgpu::Device,
    staging: &wgpu::Buffer,
    bytes: u64,
    submission: wgpu::SubmissionIndex,
    pixels: &mut Vec<u16>,
) -> Result<(), String> {
    let slice = staging.slice(..bytes);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).map_err(|e| e.to_string())?;
    receiver.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let copied = (|| {
        let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
        pixels.extend_from_slice(bytemuck::try_cast_slice::<u8, u16>(&data).map_err(|e| e.to_string())?);
        Ok::<(), String>(())
    })();
    staging.unmap();
    copied
}

struct Bands<'a> {
    renderer: &'a GreyRasterizer,
    rows: usize,
    inputs: [wgpu::Buffer; 2],
    result: wgpu::Buffer,
    readback: wgpu::Buffer,
    uniform: wgpu::Buffer,
    binding: wgpu::BindGroup,
}

impl<'a> Bands<'a> {
    fn new(renderer: &'a GreyRasterizer, rows: usize, bytes: u64) -> Result<Self, String> {
        let device = &renderer.device;
        crate::validation::scoped(device, "graph grayscale buffers", || {
            let buffer =
                |label, size, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
            let input_usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
            let inputs = [buffer("graph grey foreground", bytes, input_usage), buffer("graph grey background", bytes, input_usage)];
            let result = buffer("graph grey output", bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
            let readback = buffer("graph grey readback", bytes, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
            let uniform = buffer("graph grey parameters", 32, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
            let buffers = [&uniform, &inputs[0], &inputs[1], &result, &inputs[0]];
            let entries = buffers.map(wgpu::Buffer::as_entire_binding);
            let entries =
                entries.into_iter().enumerate().map(|(i, resource)| wgpu::BindGroupEntry { binding: i as u32, resource }).collect::<Vec<_>>();
            let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("graph grey inputs"),
                layout: &renderer.pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            Self { renderer, rows, inputs, result, readback, uniform, binding }
        })
    }

    fn run(&self, foreground: &Grey16, background: Option<&Grey16>, mode: u32, opacity: f32, side: usize, cancel: &Cancel) -> Result<Grey16, String> {
        let (device, queue) = (&self.renderer.device, &self.renderer.queue);
        let mut pixels = Vec::with_capacity(side * side);
        let source_scale = if foreground.div == 255.0 { 257 } else { 1 };
        for y in (0..side).step_by(self.rows) {
            if cancel.is_cancelled() {
                return Err("GPU grayscale pass was cancelled".into());
            }
            let rows = (side - y).min(self.rows);
            if let Some(background) = background {
                let range = y * side..(y + rows) * side;
                let write = |buffer: &wgpu::Buffer, image: &Grey16| {
                    let src = image.px.get(range.clone()).ok_or("incomplete grayscale image")?;
                    queue.write_buffer(buffer, 0, bytemuck::cast_slice(src));
                    Ok::<(), String>(())
                };
                write(&self.inputs[0], foreground)?;
                write(&self.inputs[1], background)?;
            } else {
                upload_doubling_rows(queue, &self.inputs[0], foreground, y, rows)?;
            }
            let params = [side as u32, rows as u32, foreground.w as u32, mode, opacity.to_bits(), 0, 0, source_scale];
            queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&params));
            let bytes = (rows * side * 2) as u64;
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("graph grayscale band") });
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.renderer.pipeline);
                pass.set_bind_group(0, &self.binding, &[]);
                pass.dispatch_workgroups((side as u32 / 2).div_ceil(WORKGROUP), (rows as u32).div_ceil(WORKGROUP), 1);
            }
            encoder.copy_buffer_to_buffer(&self.result, 0, &self.readback, 0, bytes);
            let submission = queue.submit([encoder.finish()]);
            read_words(device, &self.readback, bytes, submission, &mut pixels)?;
        }
        let alpha = background.map_or(foreground.alpha, |b| b.alpha);
        Ok(Grey16 { w: side, h: side, px: pixels, div: 65535.0, alpha })
    }
}

/// Upload the sampled source rows plus one row of wrap padding on both ends.
fn upload_doubling_rows(queue: &wgpu::Queue, buffer: &wgpu::Buffer, source: &Grey16, first: usize, rows: usize) -> Result<(), String> {
    let (w, h) = (source.w, source.h);
    let mut source_row = (first / 2 + h - 1) % h;
    let mut dest_row = 0;
    let row_count = rows / 2 + 2;
    while dest_row < row_count {
        let take = (row_count - dest_row).min(h - source_row);
        let pixels = source.px.get(source_row * w..(source_row + take) * w).ok_or("incomplete grayscale image")?;
        queue.write_buffer(buffer, (dest_row * w * 2) as u64, bytemuck::cast_slice(pixels));
        dest_row += take;
        source_row = 0;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering::Relaxed},
    };

    use craft_material_graph::{context::EvalContext, graph::Graph, profile::Profiler, tex::Tex};

    #[derive(Default)]
    struct Blends(AtomicUsize);

    impl Profiler for Blends {
        fn enabled(&self) -> bool {
            false
        }

        fn span(&self, _: &str) -> Box<dyn std::any::Any + Send> {
            Box::new(())
        }

        fn resident_node(&self, _: u32, kind: &str) {
            if kind == "blend" {
                self.0.fetch_add(1, Relaxed);
            }
        }
    }

    #[test]
    fn a_texture_larger_than_one_storage_binding_fits_the_resident_budget() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let rasterizer = super::GreyRasterizer::new(&gpu.device, &gpu.queue, || 4 << 30).unwrap();
        let float_8k_bytes = 8192 * 8192 * 4;
        assert!(rasterizer.resident_storage_limit() < float_8k_bytes, "the default device limits bind at most 128 MiB");

        let texture = rasterizer.reserve_texture(float_8k_bytes);

        assert!(texture.is_some(), "an 8192 square R32Float texture is not a storage binding");
        assert!(rasterizer.reserve(float_8k_bytes).is_none());
    }

    #[test]
    fn imported_eight_bit_levels_enlarge_as_sixteen_bit_on_the_gpu() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };

        #[derive(Default)]
        struct Enlargements(AtomicUsize);

        impl Profiler for Enlargements {
            fn enabled(&self) -> bool {
                false
            }

            fn span(&self, _: &str) -> Box<dyn std::any::Any + Send> {
                Box::new(())
            }

            fn resident_node(&self, id: u32, _: &str) {
                if matches!(id, 4 | 5) {
                    self.0.fetch_or(1 << (id - 4), Relaxed);
                }
            }
        }

        let nodes = Arc::new(Enlargements::default());
        let context = EvalContext { accelerator: Some(Arc::new(gpu.accelerator())), profiler: Some(nodes.clone()), ..Default::default() };
        let graph = Graph::parse(
            r#"format 3
size 512
meta compiled "1"
node 1 input idx=0 format="u8_compiled"
node 2 transform_matrix in=@1 offset_x=0.00390625 format="u8_compiled"
node 3 levels_mid in=@2 high=0.11776099 out_low=0.204225 format="u8_compiled"
node 4 transform_matrix in=@3 format="u16_compiled" osize_log2_x=10 osize_log2_y=10
node 5 transform_matrix in=@3 format="u16_compiled" osize_log2_x=11 osize_log2_y=11
output once 4
output twice 5
"#,
        )
        .unwrap();
        let input = Tex::gray_fn(512, 512, |u, v| {
            let x = (u * 512.0) as u32;
            let y = (v * 512.0) as u32;
            ((x * 1229 + y * 1153 + x * y * 13) & 255) as f32 / 255.0
        });
        let inputs = [Some(input)];
        let cpu = graph.evaluate_with_inputs(None, &inputs).unwrap();
        let gpu = graph.evaluate_in(&context, None, &inputs).unwrap();

        assert_eq!(nodes.0.load(Relaxed), 0b11);
        for (name, side) in [("once", 1024), ("twice", 2048)] {
            let (cpu, gpu) = (&cpu[name], &gpu[name]);
            assert_eq!((gpu.w, gpu.h, gpu.px.len()), (side, side, cpu.px.len()));
            let mismatch = cpu.px.iter().zip(&gpu.px).position(|(a, b)| a != b);
            assert_eq!(mismatch, None, "{name}: first mismatched pixel");
        }
    }

    #[test]
    fn imported_masked_blends_keep_binary_alpha_and_match_the_cpu() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let nodes = Arc::new(Blends::default());
        let context = EvalContext { accelerator: Some(Arc::new(gpu.accelerator())), profiler: Some(nodes.clone()), ..Default::default() };
        let cases = [
            ("copy", 1, 0.0, 1.0, 0.43, true, true),
            ("screen", 1, 1.0, 0.0, 0.43, true, true),
            ("overlay", 1, 1.0, 0.0, 0.43, true, true),
            ("min", 1, 0.0, 1.0, 0.43, false, true),
            ("soft_light_compiled", 1, 1.0, 0.0, 0.43, true, true),
            ("divide_compiled", 1, 0.0, 1.0, 0.43, true, true),
            ("add_sub_compiled", 1, 1.0, 0.0, 0.43, true, true),
            ("copy", 2, 1.0, 1.0, 0.43, true, true),
            ("multiply", 4, 0.0, 1.0, 0.43, true, true),
            ("switch", 2, 0.0, 0.0, 0.43, true, true),
            ("switch", 2, 0.0, 1.0, 1.0, false, true),
            ("switch", 2, 0.0, 1.0, 0.43, true, false),
        ];
        for (mode, alpha_mode, foreground_alpha, background_alpha, opacity, masked, accelerated) in cases {
            let attr = if masked { "mask=@3" } else { "" };
            let graph = Graph::parse(&format!(
                "format 3\nsize 256\nmeta compiled \"1\"\nnode 1 input idx=0 gray=1 format=\"u16_compiled\"\n\
                 node 2 input idx=1 gray=1 format=\"u16_compiled\"\nnode 3 input idx=2 gray=1 format=\"u16_compiled\"\n\
                 node 4 levels_mid in=@1 out_low_a={foreground_alpha} out_high_a={foreground_alpha} format=\"u16_compiled\"\n\
                 node 5 levels_mid in=@2 out_low_a={background_alpha} out_high_a={background_alpha} format=\"u16_compiled\"\n\
                 node 6 transform_matrix in=@4 offset_x=0.0078125 format=\"u16_compiled\"\n\
                 node 7 blend fg=@6 bg=@5 {attr} mode=\"{mode}\" alpha_mode={alpha_mode} opacity={opacity} format=\"u16_compiled\"\noutput out 7\n"
            ))
            .unwrap();
            let pixels = |frequency: f32| Tex::gray_fn(256, 256, |u, v| 0.5 + 0.5 * (u * frequency).sin() * (v * (frequency + 3.0)).cos());
            let inputs = [Some(pixels(39.0)), Some(pixels(53.0)), Some(pixels(23.0))];
            nodes.0.store(0, Relaxed);
            let cpu = &graph.evaluate_with_inputs(None, &inputs).unwrap()["out"];
            let gpu = &graph.evaluate_in(&context, None, &inputs).unwrap()["out"];
            let label = format!("{mode} alpha={alpha_mode} foreground={foreground_alpha} background={background_alpha} masked={masked}");
            assert_eq!(nodes.0.load(Relaxed), usize::from(accelerated), "{label}");
            assert_eq!(cpu.px.iter().any(|p| p[3] > 0.0 && p[3] < 1.0), !accelerated, "{label}");
            assert_eq!((gpu.w, gpu.h, gpu.px.len()), (cpu.w, cpu.h, cpu.px.len()), "{label}");
            let max = cpu
                .px
                .iter()
                .zip(&gpu.px)
                .map(|(a, b)| {
                    assert_eq!(a[3], b[3], "{label}");
                    ((a[0] - b[0]).abs() * 65535.0).round() as u32
                })
                .max()
                .unwrap_or_default();
            assert!(max <= 1, "{label}: {max} u16 steps");
        }
    }
}
