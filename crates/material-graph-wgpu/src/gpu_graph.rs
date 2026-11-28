//! Native GPU execution of compute-heavy operations in imported material graphs.

use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasher, Hash, Hasher, RandomState};
use std::sync::{Mutex, OnceLock, PoisonError};

use craft_material_graph::accelerator::{
    Accelerator, FilterOperation, GreyInput, GreyPass, HalfTransform, PixelImageInput, PixelOutput, PixelRequest, ResidentColorOutput,
    ResidentColorPass, ResidentFilter, ResidentGrey16, ResidentGreyFloat, ResidentGreyPass,
};
use craft_material_graph::context::Cancel;
use craft_material_graph::func::PixelShader;
use craft_material_graph::tex::{Grey, Grey16, ImgRef, Tex};
use wgpu::util::DeviceExt;

use crate::gpu_graph_grey::GreyRasterizer;
use crate::gpu_graph_half::{HalfDevicePixels, HalfRasterizer};

const PIXEL_COUNT_THRESHOLD: usize = 64 * 1024;
const MAX_PIPELINES: usize = 64;
const ROW_BUFFER_BYTES: u64 = 16 << 20;
const RESIDENT_SCRATCH_BYTES: u64 = 8 << 20;
const SHADER: &str = include_str!("shaders/graph_pixel.wgsl");
const COMPILED_GREY: &str = include_str!("shaders/graph_compiled_grey.wgsl");

const PACKED_PIXEL_ENTRY: &str = "
    @group(0) @binding(1) var<storage, read_write> result: array<u32>;
    @compute @workgroup_size(8, 8)
    fn main(@builtin(global_invocation_id) id: vec3u) {
        let size = vec2u(parameters[0].xy);
        if (id.x >= size.x / 2u || id.y >= size.y) { return; }
        let uv = (vec2f(f32(id.x * 2u), f32(id.y)) + 0.5) / vec2f(size);
        let values = vec2f(evaluate_pixel(uv), evaluate_pixel(uv + vec2f(1.0 / f32(size.x), 0.0)));
        let finite = (bitcast<vec2u>(values) & vec2u(0x7f800000u)) != vec2u(0x7f800000u);
        let words = compiled_16bit(select(vec2f(0.0), values, finite));
        result[id.y * (size.x / 2u) + id.x] = words.x | (words.y << 16u);
    }
";

/// The wgpu implementation of the material graph accelerator.
pub struct GraphAccelerator {
    device: wgpu::Device,
    queue: wgpu::Queue,
    memory_budget_bytes: fn() -> u64,
    // Metal can produce different texel coordinates for packed affine transforms and directional warps.
    spatial_sampling_on_gpu: bool,
    pipelines: Mutex<Pipelines>,
    pixels: Mutex<PixelCache>,
    hashers: [RandomState; 2],
    patterns: OnceLock<Result<crate::gpu_graph_patterns::PatternRasterizer, String>>,
    grey: OnceLock<Result<GreyRasterizer, String>>,
    half: OnceLock<Result<HalfRasterizer, String>>,
    color: OnceLock<Result<crate::gpu_graph_color::ColorRasterizer, String>>,
    filters: OnceLock<Result<crate::gpu_graph_filter::FilterRasterizer, String>>,
}

#[derive(Default)]
struct Pipelines {
    values: HashMap<String, Option<wgpu::ComputePipeline>>,
    order: VecDeque<String>,
}

#[derive(Default)]
struct PixelCache {
    values: HashMap<[u64; 2], (usize, PixelOutput)>,
    order: VecDeque<[u64; 2]>,
    bytes: usize,
}

impl PixelCache {
    fn get(&mut self, key: [u64; 2]) -> Option<PixelOutput> {
        let (_, pixels) = self.values.get(&key)?;
        self.order.retain(|&cached| cached != key);
        self.order.push_back(key);
        Some(pixels.clone())
    }

    fn put(&mut self, key: [u64; 2], bytes: usize, pixels: &PixelOutput, limit: usize) {
        if bytes > limit || self.values.contains_key(&key) {
            self.trim(limit);
            return;
        }
        self.trim(limit - bytes);
        self.order.push_back(key);
        self.values.insert(key, (bytes, pixels.clone()));
        self.bytes += bytes;
    }

    /// Drop the oldest entries until the cache holds at most `limit` bytes.
    fn trim(&mut self, limit: usize) {
        while self.bytes > limit {
            let Some(key) = self.order.pop_front() else { return };
            if let Some((bytes, _)) = self.values.remove(&key) {
                self.bytes -= bytes;
            }
        }
    }
}

impl GraphAccelerator {
    /// Create an accelerator that reads the host memory budget in bytes as each graph job runs.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, memory_budget_bytes: fn() -> u64) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            memory_budget_bytes,
            spatial_sampling_on_gpu: device.adapter_info().backend != wgpu::Backend::Metal,
            pipelines: Mutex::default(),
            pixels: Mutex::default(),
            hashers: std::array::from_fn(|_| RandomState::new()),
            patterns: OnceLock::new(),
            grey: OnceLock::new(),
            color: OnceLock::new(),
            filters: OnceLock::new(),
            half: OnceLock::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_spatial_sampling_on_gpu(mut self, on_gpu: bool) -> Self {
        self.spatial_sampling_on_gpu = on_gpu;
        self
    }

    fn pixel_cache_limit(&self) -> usize {
        ((self.memory_budget_bytes)() / 32).min(512 << 20) as usize
    }

    fn pixel_key(&self, request: &PixelRequest<'_>, shader: &PixelShader) -> Option<[u64; 2]> {
        let mut hashes = self.hashers.each_ref().map(BuildHasher::build_hasher);
        for hash in &mut hashes {
            request.size.hash(hash);
            shader.source.hash(hash);
            hash.write(bytemuck::cast_slice(&shader.uniforms));
        }
        for &binding in &shader.images {
            let &(_, image) = request.environment.images.get(binding)?;
            let mut add = |data: &[u8]| {
                for hash in &mut hashes {
                    (image.w(), image.h()).hash(hash);
                    data.hash(hash);
                }
            };
            match image {
                ImgRef::Rgba(image) => add(bytemuck::cast_slice(&image.px)),
                ImgRef::Grey(craft_material_graph::tex::GreyIn::F(image, _)) => add(bytemuck::cast_slice(&image.px)),
                ImgRef::Grey(craft_material_graph::tex::GreyIn::Q(image)) => {
                    add(bytemuck::cast_slice(&image.px));
                    for hash in &mut hashes {
                        image.div.to_bits().hash(hash);
                    }
                }
            }
        }
        Some(hashes.each_ref().map(Hasher::finish))
    }

    fn pipeline(&self, shader: &PixelShader, packed: bool) -> Option<wgpu::ComputePipeline> {
        let mut cache = self.pipelines.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(value) = cache.values.get(&shader.source) {
            return value.clone();
        }
        let (pixel, finite) = if shader.grey {
            ("f32", "(bitcast<u32>(value) & 0x7f800000u) != 0x7f800000u")
        } else {
            ("vec4f", "(bitcast<vec4u>(value) & vec4u(0x7f800000u)) != vec4u(0x7f800000u)")
        };
        let source = if packed {
            format!("{SHADER}\n{COMPILED_GREY}\n{}\n{PACKED_PIXEL_ENTRY}", shader.source)
        } else {
            format!(
                "{SHADER}\n{}\n\
            @group(0) @binding(1) var<storage, read_write> result: array<{pixel}>;\n\
            @compute @workgroup_size(8, 8)\n\
            fn main(@builtin(global_invocation_id) id: vec3u) {{\n\
                let dimensions = vec4u(parameters[0]);\n\
                if (id.x >= dimensions.x || id.y >= dimensions.w) {{ return; }}\n\
                let uv = (vec2f(vec2u(id.x, id.y + dimensions.z)) + 0.5) / vec2f(dimensions.xy);\n\
                let value = evaluate_pixel(uv);\n\
                result[id.y * dimensions.x + id.x] = select({pixel}(0.0), value, {finite});\n\
            }}",
                shader.source
            )
        };
        let pipeline = crate::validation::scoped(&self.device, "GPU pixel graph pipeline", || {
            let module = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("tc_graph_pixel"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("tc_graph_pixel"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        })
        .map_err(|reason| log::warn!("{reason}; evaluating the graph node on CPU"))
        .ok();
        if cache.order.len() >= MAX_PIPELINES
            && let Some(old) = cache.order.pop_front()
        {
            cache.values.remove(&old);
        }
        cache.order.push_back(shader.source.clone());
        cache.values.insert(shader.source.clone(), pipeline.clone());
        pipeline
    }

    fn evaluate(&self, request: &PixelRequest<'_>, shader: PixelShader) -> Result<Option<PixelOutput>, String> {
        let [width, height] = request.size.map(|v| v as u32);
        let max = self.device.limits();
        if shader.images.len() as u32 > max.max_sampled_textures_per_shader_stage
            || (shader.uniforms.len() as u64 + 1) * 16 > max.max_storage_buffer_binding_size
        {
            return Ok(None);
        }
        let images =
            shader.images.iter().map(|&binding| request.environment.images.get(binding).map(|(_, image)| *image)).collect::<Option<Vec<_>>>();
        let Some(images) = images else { return Ok(None) };
        if images.iter().any(|image| image.is_empty() || image.w().max(image.h()) as u32 > max.max_texture_dimension_2d) {
            return Ok(None);
        }
        if request.cancel.is_cancelled() {
            return Err("GPU pixel graph cancelled".into());
        }
        let Some(pipeline) = self.pipeline(&shader, false) else { return Ok(None) };
        let (device, queue) = (&self.device, &self.queue);
        let components = if shader.grey { 1 } else { 4 };
        let row_bytes = u64::from(width) * components * 4;
        let limit = max.max_storage_buffer_binding_size.min(max.max_buffer_size).min(ROW_BUFFER_BYTES);
        let max_groups = max.max_compute_workgroups_per_dimension;
        let rows = (limit / row_bytes).min(u64::from(height)).min(u64::from(max_groups) * 8) as u32;
        if rows == 0 || width.div_ceil(8) > max_groups {
            return Ok(None);
        }
        let byte_count = row_bytes * u64::from(rows);
        let mut params = vec![[width as f32, height as f32, 0.0, rows as f32]];
        params.extend(shader.uniforms);
        let (parameters, readback, bind_group) = crate::validation::scoped(device, "GPU pixel graph inputs", || {
            let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tc_graph_pixel_parameters"),
                contents: bytemuck::cast_slice(&params),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            let result = buffer(device, "tc_graph_pixel_result", byte_count, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
            let readback = buffer(device, "tc_graph_pixel_readback", byte_count, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
            let mut inputs = Vec::with_capacity(images.len());
            for (index, &image) in images.iter().enumerate() {
                let prior = images.iter().take(index).position(|&previous| same_image(previous, image));
                let view = prior.and_then(|slot| inputs.get(slot).cloned()).unwrap_or_else(|| upload(device, queue, image));
                inputs.push(view);
            }
            let mut entries = vec![
                wgpu::BindGroupEntry { binding: 0, resource: parameters.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: result.as_entire_binding() },
            ];
            entries.extend(
                inputs
                    .iter()
                    .enumerate()
                    .map(|(slot, view)| wgpu::BindGroupEntry { binding: slot as u32 + 2, resource: wgpu::BindingResource::TextureView(view) }),
            );
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("tc_graph_pixel_bindings"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            (parameters, (result, readback), bind_group)
        })
        .inspect_err(|_| {
            // Flush the texture writes of the inputs prepared before the failure
            queue.submit([]);
        })?;
        let (result, readback) = readback;
        let mut grey = Vec::new();
        let mut rgba = Vec::new();
        if shader.grey {
            grey.reserve_exact(width as usize * height as usize);
        } else {
            rgba.reserve_exact(width as usize * height as usize);
        }
        for y in (0..height).step_by(rows as usize) {
            if request.cancel.is_cancelled() {
                // The first band has not submitted the input texture writes yet
                queue.submit([]);
                return Err("GPU pixel graph cancelled".into());
            }
            let rows = rows.min(height - y);
            let bytes = u64::from(rows) * row_bytes;
            queue.write_buffer(&parameters, 0, bytemuck::cast_slice(&[width as f32, height as f32, y as f32, rows as f32]));
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_graph_pixel") });
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(width.div_ceil(8), rows.div_ceil(8), 1);
            }
            encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, bytes);
            let submission = queue.submit([encoder.finish()]);
            let slice = readback.slice(..bytes);
            let (sender, receiver) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).map_err(|e| e.to_string())?;
            receiver.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
            {
                let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
                if shader.grey {
                    grey.extend_from_slice(bytemuck::try_cast_slice::<_, f32>(&data).map_err(|e| e.to_string())?);
                } else {
                    rgba.extend_from_slice(bytemuck::try_cast_slice::<_, [f32; 4]>(&data).map_err(|e| e.to_string())?);
                }
            }
            readback.unmap();
        }
        let [w, h] = request.size;
        Ok(Some(if shader.grey { PixelOutput::Grey(Grey { w, h, px: grey }) } else { PixelOutput::Rgba(Tex { w, h, px: rgba }) }))
    }

    fn grey_rasterizer(&self) -> Option<&GreyRasterizer> {
        self.grey
            .get_or_init(|| {
                GreyRasterizer::new(&self.device, &self.queue, self.memory_budget_bytes).inspect_err(|reason| {
                    log::warn!("{reason}; evaluating grayscale graph passes on CPU");
                })
            })
            .as_ref()
            .ok()
    }

    fn half_rasterizer(&self) -> Option<&HalfRasterizer> {
        self.half
            .get_or_init(|| {
                HalfRasterizer::new(&self.device, &self.queue).inspect_err(|reason| log::warn!("{reason}; evaluating half-float transforms on CPU"))
            })
            .as_ref()
            .ok()
    }

    fn evaluate_resident_pixel(
        &self,
        shader: PixelShader,
        images: &[GreyInput<'_>],
        size: [usize; 2],
        cancel: &Cancel,
    ) -> Result<Option<ResidentGrey16>, String> {
        if self.resident_pixel_unsupported(&shader, size, cancel)
            || !shader.images.iter().any(|&i| matches!(images.get(i), Some(GreyInput::Device(_))))
            || shader.images.len() + 2 > self.device.limits().max_storage_buffers_per_shader_stage as usize
        {
            return Ok(None);
        }
        let Some(rasterizer) = self.grey_rasterizer() else { return Ok(None) };
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let mut inputs = Vec::with_capacity(shader.images.len());
        for &binding in &shader.images {
            let Some(&input) = images.get(binding) else { return Ok(None) };
            let Some(bound) = rasterizer.bind_input(input)? else { return Ok(None) };
            inputs.push(bound);
        }
        let bindings = inputs.iter().map(|image| image.buffer.as_entire_binding()).collect::<Vec<_>>();
        let output = self.dispatch_resident_pixel(rasterizer, &shader, &bindings, None, size, cancel)?;
        if output.is_some() {
            for input in &inputs {
                input.retain_until_completed(&self.queue);
            }
        }
        Ok(output)
    }

    fn resident_pixel_unsupported(&self, shader: &PixelShader, [w, h]: [usize; 2], cancel: &Cancel) -> bool {
        let max = self.device.limits();
        w < 2
            || !w.is_power_of_two()
            || h == 0
            || !shader.grey
            || (shader.uniforms.len() as u64 + 1) * 16 > max.max_storage_buffer_binding_size
            || (w / 2).div_ceil(8).max(h.div_ceil(8)) > max.max_compute_workgroups_per_dimension as usize
            || cancel.is_cancelled()
    }

    fn evaluate_resident_textured_pixel(
        &self,
        shader: PixelShader,
        images: &[PixelImageInput<'_>],
        size: [usize; 2],
        cancel: &Cancel,
    ) -> Result<Option<ResidentGrey16>, String> {
        let max = self.device.limits();
        if self.resident_pixel_unsupported(&shader, size, cancel)
            || !shader.images.iter().any(|&i| matches!(images.get(i), Some(PixelImageInput::Float(_))))
            || shader.images.len() > max.max_sampled_textures_per_shader_stage as usize
        {
            return Ok(None);
        }
        let Some(rasterizer) = self.grey_rasterizer() else { return Ok(None) };
        let Some(half) = self.half_rasterizer() else { return Ok(None) };
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let mut views = Vec::with_capacity(shader.images.len());
        let mut uploaded = Vec::new();
        let mut resident = Vec::new();
        let prepared = crate::validation::scoped(&self.device, "resident graph pixel textures", || {
            for &binding in &shader.images {
                match images.get(binding) {
                    Some(PixelImageInput::Float(image)) => {
                        let Some(pixels) = HalfDevicePixels::viewable(image, &self.device) else { return false };
                        views.push(pixels.view.clone());
                        resident.push(pixels);
                    }
                    Some(PixelImageInput::Host(image)) => {
                        if image.is_empty() || image.w().max(image.h()) > max.max_texture_dimension_2d as usize {
                            return false;
                        }
                        let bytes_per_pixel = match image {
                            ImgRef::Grey(craft_material_graph::tex::GreyIn::Q(_)) => 2,
                            ImgRef::Grey(_) => 4,
                            ImgRef::Rgba(_) => 16,
                        };
                        let Some(reservation) = rasterizer.reserve_texture((image.w() as u64) * (image.h() as u64) * bytes_per_pixel) else {
                            return false;
                        };
                        views.push(upload(&self.device, &self.queue, *image));
                        uploaded.push(reservation);
                    }
                    None => return false,
                }
            }
            true
        });
        let output = prepared.and_then(|prepared| {
            if !prepared {
                return Ok(None);
            }
            let bindings = views.iter().map(wgpu::BindingResource::TextureView).collect::<Vec<_>>();
            self.dispatch_resident_pixel(rasterizer, &shader, &bindings, Some(half), size, cancel)
        });
        if output.as_ref().is_ok_and(Option::is_some) {
            for input in resident {
                input.retain_until_completed(&self.queue);
            }
        } else if !uploaded.is_empty() {
            // Flush the writes even if a later allocation prevented the pixel dispatch.
            self.queue.submit([]);
        }
        if !uploaded.is_empty() {
            self.queue.on_submitted_work_done(move || drop(uploaded));
        }
        output
    }

    fn dispatch_resident_pixel(
        &self,
        rasterizer: &GreyRasterizer,
        shader: &PixelShader,
        inputs: &[wgpu::BindingResource<'_>],
        pack: Option<&HalfRasterizer>,
        [w, h]: [usize; 2],
        cancel: &Cancel,
    ) -> Result<Option<ResidentGrey16>, String> {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let Some(output) = rasterizer.allocate_resident(w, h)? else { return Ok(None) };
        let row_bytes = w as u64 * 4;
        let rows = if pack.is_some() { h.min((RESIDENT_SCRATCH_BYTES / row_bytes).max(1) as usize) } else { h };
        let bytes = row_bytes * rows as u64;
        let scratch_reservation = if pack.is_some() {
            if w.div_ceil(8) > self.device.limits().max_compute_workgroups_per_dimension as usize {
                return Ok(None);
            }
            let Some(reservation) = rasterizer.reserve(bytes) else { return Ok(None) };
            Some(reservation)
        } else {
            None
        };
        // Reuse the single-pixel shader for sampled inputs. Inlining large functions twice
        // into the paired writer makes cold compilation much slower.
        let Some(pipeline) = self.pipeline(shader, pack.is_none()) else { return Ok(None) };
        let (device, queue) = (&self.device, &self.queue);
        let mut params = vec![[w as f32, h as f32, 0.0, h as f32]];
        params.extend_from_slice(&shader.uniforms);
        let submitted = crate::validation::scoped(device, "resident graph pixel processor", || {
            let scratch = pack.map(|_| buffer(device, "sampled graph pixel scratch", bytes, wgpu::BufferUsages::STORAGE));
            let target = scratch.as_ref().unwrap_or(&output.buffer);
            let mut encoder = device.create_command_encoder(&Default::default());
            let columns = if pack.is_some() { w as u32 } else { w as u32 / 2 };
            for y in (0..h).step_by(rows) {
                if cancel.is_cancelled() {
                    return false;
                }
                let band_rows = rows.min(h - y) as u32;
                params[0] = [w as f32, h as f32, y as f32, band_rows as f32];
                let parameters = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("resident graph pixel parameters"),
                    contents: bytemuck::cast_slice(&params),
                    usage: wgpu::BufferUsages::STORAGE,
                });
                let entries = [parameters.as_entire_binding(), target.as_entire_binding()]
                    .into_iter()
                    .chain(inputs.iter().cloned())
                    .enumerate()
                    .map(|(binding, resource)| wgpu::BindGroupEntry { binding: binding as u32, resource })
                    .collect::<Vec<_>>();
                let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("resident graph pixel inputs"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &entries,
                });
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &binding, &[]);
                    pass.dispatch_workgroups(columns.div_ceil(8), band_rows.div_ceil(8), 1);
                }
                if let Some((pack, scratch)) = pack.zip(scratch.as_ref()) {
                    pack.pack_result(&mut encoder, &parameters, scratch, &output.buffer, [w as u32, band_rows]);
                }
            }
            queue.submit([encoder.finish()]);
            true
        })?;
        if !submitted {
            return Ok(None);
        }
        if let Some(reservation) = scratch_reservation {
            queue.on_submitted_work_done(move || drop(reservation));
        }
        output.retain_until_completed(queue);
        Ok(Some(ResidentGrey16 { w, h, div: 65535.0, alpha: 1.0, pixels: output }))
    }
}

impl Accelerator for GraphAccelerator {
    fn cache_tag(&self) -> &str {
        if self.spatial_sampling_on_gpu { "wgpu-graph-v11" } else { "wgpu-graph-v11-metal" }
    }

    fn pixel(&self, request: PixelRequest<'_>) -> Result<Option<PixelOutput>, String> {
        if request.size[0] * request.size[1] < PIXEL_COUNT_THRESHOLD {
            return Ok(None);
        }
        let Some(shader) = request.program.pixel_shader(request.environment) else { return Ok(None) };
        let bytes = request.size[0] * request.size[1] * if shader.grey { 4 } else { 16 };
        let limit = self.pixel_cache_limit();
        // A lowered host budget also applies to the outputs cached before it
        self.pixels.lock().unwrap_or_else(PoisonError::into_inner).trim(limit);
        let key = (bytes <= limit && !request.cancel.is_cancelled()).then(|| self.pixel_key(&request, &shader)).flatten();
        if let Some(key) = key
            && let Some(cached) = self.pixels.lock().unwrap_or_else(PoisonError::into_inner).get(key)
        {
            return Ok(Some(cached));
        }
        match self.evaluate(&request, shader) {
            Err(error) if !request.cancel.is_cancelled() => {
                log::warn!("{error}; evaluating the graph node on CPU");
                Ok(None)
            }
            Ok(Some(output)) => {
                if let Some(key) = key {
                    // The budget may have been lowered while the job ran
                    self.pixels.lock().unwrap_or_else(PoisonError::into_inner).put(key, bytes, &output, self.pixel_cache_limit());
                }
                Ok(Some(output))
            }
            result => result,
        }
    }

    fn pattern(
        &self,
        request: craft_material_graph::patterns::RasterRequest<'_>,
    ) -> Result<Option<craft_material_graph::patterns::RasterOutput>, String> {
        let cancel = request.cancel;
        let rasterizer = self.patterns.get_or_init(|| match crate::gpu_graph_patterns::PatternRasterizer::new(&self.device, &self.queue) {
            Ok(rasterizer) => Ok(rasterizer),
            Err(reason) => {
                log::warn!("{reason}; evaluating pattern programs on CPU");
                Err(reason)
            }
        });
        let Ok(rasterizer) = rasterizer else { return Ok(None) };
        match rasterizer.render(request) {
            Err(reason) if !cancel.is_cancelled() => {
                log::warn!("{reason}; evaluating the pattern program on CPU");
                Ok(None)
            }
            result => result,
        }
    }

    fn grey_pass(&self, pass: GreyPass<'_>, cancel: &Cancel) -> Result<Option<Grey16>, String> {
        let Some(rasterizer) = self.grey_rasterizer() else { return Ok(None) };
        match rasterizer.render(pass, cancel) {
            Err(reason) if !cancel.is_cancelled() => {
                log::warn!("{reason}; evaluating the grayscale graph pass on CPU");
                Ok(None)
            }
            result => result,
        }
    }

    fn resident_grey(&self, pass: ResidentGreyPass<'_>, cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        let Some(rasterizer) = self.grey_rasterizer() else { return Ok(None) };
        resident_or_fallback(rasterizer.render_resident(pass, cancel), cancel)
    }

    fn resident_color(&self, pass: ResidentColorPass<'_>, cancel: &Cancel) -> Result<Option<ResidentColorOutput>, String> {
        if !self.spatial_sampling_on_gpu {
            match &pass {
                ResidentColorPass::WrappedAffine { .. } => return Ok(None),
                ResidentColorPass::DirectionalWarp { settings, .. } if settings.intensity != 0.0 => return Ok(None),
                _ => {}
            }
        }
        let Some(pool) = self.grey_rasterizer() else { return Ok(None) };
        let rasterizer = self
            .color
            .get_or_init(|| crate::gpu_graph_color::ColorRasterizer::new(&self.device, &self.queue).inspect_err(|reason| log::warn!("{reason}")));
        let Ok(rasterizer) = rasterizer else { return Ok(None) };
        resident_or_fallback(rasterizer.render(pass, pool, cancel), cancel)
    }

    fn resident_pixel(
        &self,
        shader: PixelShader,
        images: &[GreyInput<'_>],
        size: [usize; 2],
        cancel: &Cancel,
    ) -> Result<Option<ResidentGrey16>, String> {
        resident_or_fallback(self.evaluate_resident_pixel(shader, images, size, cancel), cancel)
    }

    fn resident_filter(&self, filter: ResidentFilter<'_>, cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        if !self.spatial_sampling_on_gpu {
            match &filter.operation {
                FilterOperation::Affine { .. } => return Ok(None),
                FilterOperation::DirectionalWarp { intensity, .. } if *intensity != 0.0 => return Ok(None),
                _ => {}
            }
        }
        let Some(pool) = self.grey_rasterizer() else { return Ok(None) };
        let rasterizer = self
            .filters
            .get_or_init(|| crate::gpu_graph_filter::FilterRasterizer::new(&self.device, &self.queue).inspect_err(|reason| log::warn!("{reason}")));
        let Ok(rasterizer) = rasterizer else { return Ok(None) };
        resident_or_fallback(rasterizer.render(filter, pool, cancel), cancel)
    }

    fn resident_half_transform(&self, pass: HalfTransform<'_>, cancel: &Cancel) -> Result<Option<ResidentGreyFloat>, String> {
        let Some(grey) = self.grey_rasterizer() else { return Ok(None) };
        let Some(half) = self.half_rasterizer() else { return Ok(None) };
        resident_or_fallback(half.render(grey, pass, cancel), cancel)
    }

    fn resident_textured_pixel(
        &self,
        shader: PixelShader,
        images: &[PixelImageInput<'_>],
        size: [usize; 2],
        cancel: &Cancel,
    ) -> Result<Option<ResidentGrey16>, String> {
        resident_or_fallback(self.evaluate_resident_textured_pixel(shader, images, size, cancel), cancel)
    }
}

fn resident_or_fallback<T>(result: Result<Option<T>, String>, cancel: &Cancel) -> Result<Option<T>, String> {
    match result {
        Err(reason) if !cancel.is_cancelled() => {
            log::warn!("{reason}; retrying the graph node with bounded transfers");
            Ok(None)
        }
        result => result,
    }
}

fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false })
}

fn same_image(a: ImgRef<'_>, b: ImgRef<'_>) -> bool {
    use craft_material_graph::tex::GreyIn::{F, Q};
    match (a, b) {
        (ImgRef::Rgba(a), ImgRef::Rgba(b)) => std::ptr::eq(a, b),
        (ImgRef::Grey(F(a, _)), ImgRef::Grey(F(b, _))) => std::ptr::eq(a, b),
        (ImgRef::Grey(Q(a)), ImgRef::Grey(Q(b))) => std::ptr::eq(a, b),
        _ => false,
    }
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, image: ImgRef<'_>) -> wgpu::TextureView {
    let w = image.w() as u32;
    let h = image.h() as u32;
    let (format, stride) = match image {
        ImgRef::Rgba(_) => (wgpu::TextureFormat::Rgba32Float, 16),
        ImgRef::Grey(craft_material_graph::tex::GreyIn::F(..)) => (wgpu::TextureFormat::R32Float, 4),
        ImgRef::Grey(craft_material_graph::tex::GreyIn::Q(_)) => (wgpu::TextureFormat::R16Uint, 2),
    };
    let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tc_graph_pixel_input"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride * w), rows_per_image: Some(h) };
    match image {
        ImgRef::Rgba(image) => queue.write_texture(texture.as_image_copy(), bytemuck::cast_slice(&image.px), layout, size),
        ImgRef::Grey(craft_material_graph::tex::GreyIn::F(image, _)) => {
            queue.write_texture(texture.as_image_copy(), bytemuck::cast_slice(&image.px), layout, size);
        }
        ImgRef::Grey(craft_material_graph::tex::GreyIn::Q(image)) => {
            queue.write_texture(texture.as_image_copy(), bytemuck::cast_slice(&image.px), layout, size);
        }
    }
    texture.create_view(&Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_material_graph::context::Cancel;
    use craft_material_graph::func::{FnEnv, compile_named};
    use craft_material_graph::graph::Graph;

    #[test]
    fn imported_pixel_program_samples_on_gpu() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let graph = Graph::parse("size 256\nsubgraph example\nmeta kind \"function\"\nmeta float32 \"1\"\nnode 1 f_pos\nnode 2 f_sample_gray a=@1 slot=\"in\" filter=1\nnode 3 f_float v=0.2\nnode 4 f_add a=@2 b=@3\nnode 5 f_out in=@4\nend\nnode 1 pixel_processor fn=\"example\"\noutput out 1\n").unwrap();
        let program = compile_named(&graph.subgraphs, "example").unwrap();
        let input = Grey::from_fn(17, 19, |u, v| u * v);
        let steps = Grey16 { w: input.w, h: input.h, px: input.px.iter().map(|v| (v * 255.0).round() as u16).collect(), div: 255.0, alpha: 1.0 };
        let backend = gpu.accelerator();
        let cancel = Cancel::new();
        use craft_material_graph::tex::GreyIn::{F, Q};
        for input in [F(&input, 1.0), Q(&steps)] {
            let mut environment = FnEnv::new([256.0, 256.0]);
            environment.images.push(("in", ImgRef::Grey(input)));
            let Some(PixelOutput::Grey(pixels)) =
                backend.pixel(PixelRequest { program: &program, environment: &environment, size: [256, 256], cancel: &cancel }).unwrap()
            else {
                panic!("the imported pixel program must execute on GPU");
            };
            let expected = Grey::from_fn(256, 256, |u, v| program.run(&environment, [f64::from(u), f64::from(v)]).unwrap().as_f32());
            assert_eq!((pixels.w, pixels.h, pixels.px.len()), (expected.w, expected.h, expected.px.len()));
            assert!(expected.px.iter().zip(pixels.px).all(|(&a, b)| (a - b).abs() < 2e-6));
        }
    }

    #[test]
    fn pixel_program_bands_stay_within_the_device_dispatch_limit() {
        let limits = wgpu::Limits { max_compute_workgroups_per_dimension: 16, ..wgpu::Limits::default() };
        let Some(gpu) = crate::headless::Headless::with_limits(limits).unwrap() else { return };
        let graph = Graph::parse("size 128\nsubgraph example\nmeta kind \"function\"\nmeta float32 \"1\"\nnode 1 f_pos\nnode 2 f_swizzle a=@1 mask=\"y\"\nnode 3 f_out in=@2\nend\nnode 1 pixel_processor fn=\"example\"\noutput out 1\n").unwrap();
        let program = compile_named(&graph.subgraphs, "example").unwrap();
        let environment = FnEnv::new([128.0, 1024.0]);
        let cancel = Cancel::new();

        let output =
            gpu.accelerator().pixel(PixelRequest { program: &program, environment: &environment, size: [128, 1024], cancel: &cancel }).unwrap();

        let Some(PixelOutput::Grey(pixels)) = output else { panic!("a 128 pixel wide program fits 16 workgroups per row") };
        let expected = Grey::from_fn(128, 1024, |u, v| program.run(&environment, [f64::from(u), f64::from(v)]).unwrap().as_f32());
        assert_eq!((pixels.w, pixels.h, pixels.px.len()), (expected.w, expected.h, expected.px.len()));
        assert!(expected.px.iter().zip(pixels.px).all(|(&a, b)| (a - b).abs() < 2e-6));
    }

    #[test]
    fn a_lowered_budget_shrinks_the_pixel_cache_it_already_filled() {
        static BUDGET_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(4 << 30);
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let backend = GraphAccelerator::new(&gpu.device, &gpu.queue, || BUDGET_BYTES.load(std::sync::atomic::Ordering::Relaxed));
        let graph = Graph::parse("size 256\nsubgraph example\nmeta kind \"function\"\nmeta float32 \"1\"\nnode 1 f_pos\nnode 2 f_swizzle a=@1 mask=\"x\"\nnode 3 f_out in=@2\nend\nnode 1 pixel_processor fn=\"example\"\noutput out 1\n").unwrap();
        let program = compile_named(&graph.subgraphs, "example").unwrap();
        let environment = FnEnv::new([256.0, 256.0]);
        let cancel = Cancel::new();
        let request = || PixelRequest { program: &program, environment: &environment, size: [256, 256], cancel: &cancel };
        assert!(backend.pixel(request()).unwrap().is_some());
        let cached_bytes = || backend.pixels.lock().unwrap().bytes;
        assert!(cached_bytes() > 0, "a 256 KiB output fits the cache of a 4 GiB budget");

        BUDGET_BYTES.store(1 << 20, std::sync::atomic::Ordering::Relaxed);
        backend.pixel(request()).unwrap();

        assert!(cached_bytes() <= backend.pixel_cache_limit(), "{} cached bytes over the lowered limit", cached_bytes());
    }

    #[test]
    fn a_lowered_budget_evicts_old_pixels_even_when_the_next_result_cannot_be_cached() {
        let pixels = PixelOutput::Grey(Grey::from_fn(2, 2, |u, v| u * v));
        let bytes = 4 * size_of::<f32>();
        let mut cache = PixelCache::default();
        cache.put([1, 2], bytes, &pixels, 2 * bytes);
        cache.put([3, 4], bytes, &pixels, 2 * bytes);

        cache.put([3, 4], bytes, &pixels, bytes);
        assert!(cache.get([1, 2]).is_none());
        assert!(cache.get([3, 4]).is_some());
        assert_eq!(cache.bytes, bytes);

        cache.put([5, 6], bytes, &pixels, bytes / 2);
        assert!(cache.values.is_empty());
        assert!(cache.order.is_empty());
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn turn_reduction_rounds_the_product_toward_zero_like_the_reference() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let source = format!(
            "{SHADER}\n@group(0) @binding(1) var<storage, read_write> turns: array<vec4f>;\n\
             @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) id: vec3u) {{\n\
                 let x = parameters[id.x];\n\
                 turns[id.x] = vec4f(tc_turn(x.x), tc_turn(x.y), tc_turn(x.z), tc_turn(x.w));\n\
             }}"
        );
        // WGSL permits indeterminate results for non-finite phases. Compare only finite inputs,
        // from a tenth of a radian to past 2^23 turns (about 5.3e7), with both signs.
        let inputs = (0..4096)
            .map(|i| {
                let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
                sign * 10f32.powf(i as f32 / 400.0 - 1.0) * 1.000_123
            })
            .collect::<Vec<f32>>();

        let bytes = gpu.run_compute(&source, bytemuck::cast_slice(&inputs), (inputs.len() * 4) as u64, (inputs.len() / 4) as u32);

        let turns = bytemuck::cast_slice::<u8, f32>(&bytes);
        for (&x, &turn) in inputs.iter().zip(turns) {
            assert_eq!(turn, reference_turn(x), "phase of {x:e}");
        }
    }

    /// The reference: x / 2 pi with the single-precision product rounded toward zero, as `func.rs` reads it
    fn reference_turn(x: f32) -> f32 {
        let product = f64::from(x) * f64::from(std::f32::consts::FRAC_1_PI / 2.0);
        let near = product as f32;
        let t = if f64::from(near).abs() > product.abs() { if near > 0.0 { near.next_down() } else { near.next_up() } } else { near };
        (t - t.floor()) * 6.283_185_5
    }

    /// `tc_turn` of graph_pixel.wgsl step by step, to check its integer rounding test on every machine
    fn shader_turn(x: f32) -> f32 {
        let mantissa = |x: f32| {
            let bits = x.abs().to_bits();
            let (exponent, fraction) = (bits >> 23, bits & 0x7f_ffff);
            if exponent == 0 { (fraction, 1) } else { (fraction | 0x80_0000, exponent) }
        };
        let rounded_away = |x: f32, near: f32| {
            let (a, b, n) = (mantissa(x), mantissa(std::f32::consts::FRAC_1_PI / 2.0), mantissa(near));
            let middle = (a.0 >> 12) * (b.0 & 0xfff) + (a.0 & 0xfff) * (b.0 >> 12);
            let low_sum = (a.0 & 0xfff) * (b.0 & 0xfff) + ((middle & 0xfff) << 12);
            let high = (a.0 >> 12) * (b.0 >> 12) + (middle >> 12) + (low_sum >> 24);
            let low = low_sum & 0xff_ffff;
            let shift = n.1 as i32 + 150 - a.1 as i32 - b.1 as i32;
            if !(0..=48).contains(&shift) {
                return false;
            }
            let (near_high, near_low) = if shift > 24 { (n.0 << (shift - 24), 0) } else { (n.0 >> (24 - shift), (n.0 << shift) & 0xff_ffff) };
            near_high > high || (near_high == high && near_low > low)
        };
        let near = x * (std::f32::consts::FRAC_1_PI / 2.0);
        let t = if near != 0.0 && rounded_away(x, near) { f32::from_bits(near.to_bits() - 1) } else { near };
        (t - t.floor()) * 6.283_185_5
    }

    #[test]
    fn turn_rounding_test_matches_the_reference_without_a_gpu() {
        let inputs = (1..0x7f00_0000_u32).step_by(40_009).flat_map(|bits| [f32::from_bits(bits), -f32::from_bits(bits)]);

        let mismatch = inputs.clone().find(|&x| shader_turn(x).to_bits() != reference_turn(x).to_bits());

        assert_eq!(mismatch, None, "first phase that differs from the reference");
        assert!(inputs.clone().any(|x| x.abs() > 5.3e7), "the sweep reaches past 2^23 turns");
    }

    #[test]
    fn imported_pixel_program_binds_indexed_random_initializers() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let backend = gpu.accelerator();
        let cancel = Cancel::new();
        for (site, seed_source, independent) in [("size", 1, true), ("value", 1, true), ("pixel", 1, false), ("size", 4, false)] {
            let graph = Graph::parse(&format!(
                "size 256\nsubgraph example\nmeta kind \"function\"\nmeta float32 \"1\"\n\
                 node 1 f_seed\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 4 f_to_int a=@3\n\
                 node 5 f_random lcg=\"{site}\" s=@{seed_source} k=2\nnode 6 f_set a=@5 name=\"offset\"\n\
                 node 7 f_get name=\"offset\" type=0\nnode 8 f_add a=@3 b=@7\nnode 9 f_sequence a=@6 b=@8\n\
                 node 10 f_out in=@9\nend\nnode 1 pixel_processor fn=\"example\"\noutput out 1\n",
            ))
            .unwrap();
            let program = compile_named(&graph.subgraphs, "example").unwrap();
            let mut environment = FnEnv::new([256.0, 256.0]);
            for seed in [432, 123_456_789] {
                environment.seed = seed;
                let output = backend.pixel(PixelRequest { program: &program, environment: &environment, size: [256, 256], cancel: &cancel }).unwrap();
                if !independent {
                    assert!(output.is_none());
                    continue;
                }
                let Some(PixelOutput::Grey(pixels)) = output else { panic!("the indexed draw should be bound on the GPU") };
                let expected = Grey::from_fn(256, 256, |u, v| program.run(&environment, [f64::from(u), f64::from(v)]).unwrap().as_f32());
                assert_eq!(pixels.px, expected.px);
            }
        }
    }
}
