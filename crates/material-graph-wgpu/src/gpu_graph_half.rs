//! Wrapped grayscale half-float transforms into GPU-resident sampled textures.

use std::{any::Any, sync::Arc};

use craft_material_graph::accelerator::{GreyTransformInput, HalfTransform, ResidentFloatPixels, ResidentGreyFloat};
use craft_material_graph::context::Cancel;
use wgpu::util::DeviceExt;

use crate::gpu_graph_grey::{GreyRasterizer, ResidentBytes};

const SHADER: &str = include_str!("shaders/graph_half.wgsl");
const DECODE: &str = include_str!("shaders/graph_compiled_grey.wgsl");
const PACKED_INPUT: &str = "
    @group(0) @binding(1) var<storage, read> input: array<u32>;
    fn read_pixel(p: vec2u) -> f32 {
        let i = p.y * dims.x + p.x;
        return decode((input[i / 2u] >> ((i & 1u) * 16u)) & 0xffffu);
    }
";
const FLOAT_INPUT: &str = "
    @group(0) @binding(1) var input: texture_2d<f32>;
    fn read_pixel(p: vec2u) -> f32 { return textureLoad(input, vec2i(p), 0).x; }
";
const REDUCE: u32 = 0;
const DOUBLE: u32 = 1;
const DOUBLE_FINAL: u32 = 2;
const SAMPLE_FINAL: u32 = 3;
const READBACK_BYTES: u32 = 8 << 20;

pub(crate) struct HalfRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    from_packed: wgpu::ComputePipeline,
    from_float: wgpu::ComputePipeline,
    pack: wgpu::ComputePipeline,
}

impl HalfRasterizer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, String> {
        let (from_packed, from_float, pack) = crate::validation::scoped(device, "graph half-float pipelines", || {
            let compile = |label: &'static str, source: String| {
                let shader =
                    device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(source.into()) });
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(label),
                    layout: None,
                    module: &shader,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: None,
                })
            };
            (
                compile("graph packed to half", format!("{DECODE}\n{PACKED_INPUT}\n{SHADER}")),
                compile("graph float to half", format!("{DECODE}\n{FLOAT_INPUT}\n{SHADER}")),
                compile("graph pack float result", format!("{DECODE}\n{}", include_str!("shaders/graph_pack_float_grey.wgsl"))),
            )
        })?;
        Ok(Self { device: device.clone(), queue: queue.clone(), from_packed, from_float, pack })
    }

    pub(crate) fn pack_result(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        parameters: &wgpu::Buffer,
        scratch: &wgpu::Buffer,
        output: &wgpu::Buffer,
        size: [u32; 2],
    ) {
        let entries = [parameters, scratch, output]
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry { binding: binding as u32, resource: buffer.as_entire_binding() })
            .collect::<Vec<_>>();
        let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pack sampled graph result"),
            layout: &self.pack.get_bind_group_layout(0),
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.pack);
        pass.set_bind_group(0, &binding, &[]);
        let [w, h] = size;
        pass.dispatch_workgroups((w / 2).div_ceil(8), h.div_ceil(8), 1);
    }

    pub(crate) fn render(&self, gray: &GreyRasterizer, pass: HalfTransform<'_>, cancel: &Cancel) -> Result<Option<ResidentGreyFloat>, String> {
        let HalfTransform { source, side, mip } = pass;
        let limits = self.device.limits();
        let max_side = (limits.max_texture_dimension_2d as usize).min((limits.max_compute_workgroups_per_dimension as usize).saturating_mul(8));
        if side == 0 || !side.is_power_of_two() || side > max_side || cancel.is_cancelled() {
            return Ok(None);
        }
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let (width, packed, float) = match source {
            GreyTransformInput::Quantized(source) => {
                let (w, h, div, alpha) = source.info();
                if w != h || !w.is_power_of_two() || div != 65535.0 || alpha != 1.0 {
                    return Ok(None);
                }
                let Some(input) = gray.bind_input(source)? else { return Ok(None) };
                (w, Some(input), None)
            }
            GreyTransformInput::Float(image) => {
                let Some(input) = HalfDevicePixels::viewable(image, &self.device) else { return Ok(None) };
                if image.w != image.h || !image.w.is_power_of_two() {
                    return Ok(None);
                }
                (image.w, None, Some(input))
            }
        };
        let mut steps = Vec::new();
        let mut width_now = width;
        if mip.is_none() && width < side {
            while width_now < side {
                width_now *= 2;
                steps.push((width_now, if width_now < side { DOUBLE } else { DOUBLE_FINAL }));
            }
        } else {
            let from_level = mip.map_or(side.min(width), |level| width.checked_shr(level).unwrap_or(0).max(1));
            while width_now > from_level {
                width_now /= 2;
                steps.push((width_now, REDUCE));
            }
            steps.push((side, SAMPLE_FINAL));
        }

        let mut stages = Vec::with_capacity(steps.len());
        for (width, mode) in steps {
            if cancel.is_cancelled() {
                return Ok(None);
            }
            let Some(reservation) = gray.reserve_texture((width as u64) * (width as u64) * 4) else { return Ok(None) };
            let texture = crate::validation::scoped(&self.device, "graph half-float texture", || {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("resident graph grayscale float"),
                    size: extent(width as u32, width as u32),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R32Float,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            })?;
            let view = texture.create_view(&Default::default());
            stages.push((
                width as u32,
                mode,
                HalfDevicePixels { device: self.device.clone(), queue: self.queue.clone(), texture, view, reservation },
            ));
        }

        crate::validation::scoped(&self.device, "graph half-float transform", || {
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("graph half-float transform") });
            let mut from_packed = packed.as_ref();
            let mut from_float = float;
            let mut input_size = width as u32;
            for (output_size, mode, target) in &stages {
                let parameters = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("graph half-float dimensions"),
                    contents: bytemuck::cast_slice(&[input_size, *output_size, *mode, 0u32]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let (pipeline, input) = if let Some(input) = from_packed.take() {
                    (&self.from_packed, input.buffer.as_entire_binding())
                } else if let Some(input) = from_float {
                    (&self.from_float, wgpu::BindingResource::TextureView(&input.view))
                } else {
                    return Err("graph half-float transform has no source".to_string());
                };
                let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("graph half-float transform"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: parameters.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: input },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&target.view) },
                    ],
                });
                let mut compute = encoder.begin_compute_pass(&Default::default());
                compute.set_pipeline(pipeline);
                compute.set_bind_group(0, &binding, &[]);
                compute.dispatch_workgroups(output_size.div_ceil(8), output_size.div_ceil(8), 1);
                from_float = Some(target);
                input_size = *output_size;
            }
            self.queue.submit([encoder.finish()]);
            Ok(())
        })??;

        if let Some(input) = packed {
            input.retain_until_completed(&self.queue);
        }
        if let Some(input) = float {
            input.retain_until_completed(&self.queue);
        }
        let reservations = stages.iter().map(|(_, _, target)| target.reservation.clone()).collect::<Vec<_>>();
        self.queue.on_submitted_work_done(move || drop(reservations));
        Ok(stages.pop().map(|(_, _, pixels)| ResidentGreyFloat { w: side, h: side, pixels: Arc::new(pixels) }))
    }
}

#[derive(Debug)]
pub(crate) struct HalfDevicePixels {
    device: wgpu::Device,
    queue: wgpu::Queue,
    texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    reservation: Arc<ResidentBytes>,
}

impl HalfDevicePixels {
    pub(crate) fn viewable<'a>(image: &'a ResidentGreyFloat, device: &wgpu::Device) -> Option<&'a Self> {
        let pixels = image.pixels.as_any().downcast_ref::<Self>()?;
        (pixels.device == *device && pixels.texture.width() as usize == image.w && pixels.texture.height() as usize == image.h).then_some(pixels)
    }

    pub(crate) fn retain_until_completed(&self, queue: &wgpu::Queue) {
        let reservation = self.reservation.clone();
        queue.on_submitted_work_done(move || drop(reservation));
    }
}

impl ResidentFloatPixels for HalfDevicePixels {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn read(&self, cancel: &Cancel) -> Result<Vec<f32>, String> {
        let (width, height) = (self.texture.width(), self.texture.height());
        let line = width * 4;
        let row = line.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let height_per_band = (READBACK_BYTES / row).max(1).min(height);
        let staging = crate::validation::scoped(&self.device, "graph half-float readback", || {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("resident grayscale float readback"),
                size: u64::from(row) * u64::from(height_per_band),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        })?;
        let mut pixels = Vec::with_capacity(width as usize * height as usize);
        for y in (0..height).step_by(height_per_band as usize) {
            if cancel.is_cancelled() {
                return Err("GPU float grayscale readback was cancelled".into());
            }
            let rows = height_per_band.min(height - y);
            let slice = staging.slice(..u64::from(row) * u64::from(rows));
            let submission = crate::validation::scoped(&self.device, "graph half-float copy", || {
                let mut encoder = self.device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: 0, y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &staging,
                        layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(rows) },
                    },
                    extent(width, rows),
                );
                self.queue.submit([encoder.finish()])
            })?;
            let (sender, receiver) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            self.device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).map_err(|e| e.to_string())?;
            receiver.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
            let copied = (|| {
                let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
                for bytes in data.chunks_exact(row as usize) {
                    let bytes = bytes.get(..line as usize).ok_or("incomplete graph half-float row")?;
                    pixels.extend_from_slice(bytemuck::try_cast_slice::<u8, f32>(bytes).map_err(|e| e.to_string())?);
                }
                Ok::<(), String>(())
            })();
            staging.unmap();
            copied?;
        }
        Ok(pixels)
    }
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d { width, height, depth_or_array_layers: 1 }
}
