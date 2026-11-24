//! Resident spatial filters for packed grayscale graphs.

use std::sync::Arc;

use craft_material_graph::accelerator::{FilterOperation, GreyInput, ResidentFilter, ResidentGrey16, ResidentGreyPass};
use craft_material_graph::context::Cancel;
use wgpu::util::DeviceExt;

use crate::gpu_graph_grey::GreyRasterizer;

const WORKGROUP: u32 = 8;
const DIRECTIONAL_WARP: u32 = 0;
const GRADIENT_WARP: u32 = 1;
const BOX: u32 = 2;
const SHORT_LINE: u32 = 3;
const SHARPEN: u32 = 4;
const SHEARED_LINE: u32 = 5;
const AFFINE: u32 = 6;
const SHADER: &str = concat!(
    include_str!("shaders/graph_filter.wgsl"),
    "\n",
    include_str!("shaders/graph_compiled_grey.wgsl"),
    "\n",
    include_str!("shaders/graph_sampling.wgsl")
);

pub(crate) struct FilterRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
}

impl FilterRasterizer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, String> {
        let pipeline = crate::validation::scoped(device, "graph grayscale filter shader", || {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("graph grayscale filters"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("graph grayscale filter"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        })?;
        Ok(Self { device: device.clone(), queue: queue.clone(), pipeline })
    }

    pub(crate) fn render(&self, filter: ResidentFilter<'_>, pool: &GreyRasterizer, cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        let [w, h] = filter.size;
        let [groups_x, groups_y] = [(w as u32 / 2).div_ceil(WORKGROUP), (h as u32).div_ceil(WORKGROUP)];
        let limit = self.device.limits().max_compute_workgroups_per_dimension;
        if cancel.is_cancelled() || groups_x == 0 || groups_y == 0 || groups_x.max(groups_y) > limit {
            return Ok(None);
        }
        let source = filter.source;
        let (sw, sh, divisor, alpha) = source.info();
        let [wrap_x, wrap_y] = filter.tile.map(u32::from);
        let mut params = Params {
            source: [sw as u32, sh as u32, divisor as u32, 0],
            output: [w as u32, h as u32, filter.div as u32, 0],
            map: [0; 4],
            control: [0, 0, wrap_x, wrap_y],
            values: [0.0; 4],
            direction: [0.0; 4],
            rounding_barrier: [0; 4],
        };
        let mut passes = Vec::with_capacity(2);
        let mut shifts = None;
        let mut reduced = None;
        let map = match filter.operation {
            FilterOperation::DirectionalWarp { map, intensity, offset, direction, aspect } => {
                params.control[0] = DIRECTIONAL_WARP;
                params.values = [intensity, offset, aspect[0], aspect[1]];
                params.direction = [direction[0], direction[1], 0.0, 0.0];
                Some(map)
            }
            FilterOperation::GradientWarp { map, displacement } => {
                params.control[0] = GRADIENT_WARP;
                params.values = [displacement[0], displacement[1], 0.0, 0.0];
                Some(map)
            }
            FilterOperation::Box { radius, round_between } => {
                params.control[0] = BOX;
                push_box_passes(&mut passes, params, radius, round_between);
                None
            }
            FilterOperation::ShortLine { major, slope, radius } => {
                params.control[0] = SHORT_LINE;
                params.control[1] = major as u32;
                params.values = [radius, slope, 0.0, 0.0];
                None
            }
            FilterOperation::ShearedLine { major, slope, radius } => {
                params.control[0] = SHEARED_LINE;
                params.control[1] = major as u32;
                let pad = (radius - 0.5).ceil() as i32;
                params.values = [radius, pad as f32, 0.0, 0.0];
                shifts = Some(shear_shifts(if major == 0 { w } else { h }, pad, f64::from(slope)));
                None
            }
            FilterOperation::Sharpen { weights } => {
                params.control[0] = SHARPEN;
                params.values = [weights[0], weights[1], 0.0, 0.0];
                None
            }
            FilterOperation::Affine { matrix, offset, bilinear, mip_side, edge_fade } => {
                params.control[0] = AFFINE;
                params.control[1] = u32::from(bilinear);
                if let Some(side) = mip_side {
                    let Some(image) = pool.render_resident(ResidentGreyPass::Reduce { source, side }, cancel)? else { return Ok(None) };
                    params.source[0] = side as u32;
                    params.source[1] = side as u32;
                    params.source[2] = image.div as u32;
                    reduced = Some(image);
                }
                params.values = [offset[0], offset[1], edge_fade[0] as f32, edge_fade[1] as f32];
                params.direction = matrix;
                None
            }
        };
        if let Some(map) = map {
            let (mw, mh, div, _) = map.info();
            params.map = [mw as u32, mh as u32, div as u32, 0];
        }
        if !matches!(filter.operation, FilterOperation::Box { .. }) {
            passes.push(params);
        }

        let source = reduced.as_ref().map_or(source, GreyInput::Device);
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let Some(input) = pool.bind_input(source)? else { return Ok(None) };
        let external_map = match map {
            Some(map) if !same_input(source, map) => {
                let Some(bound) = pool.bind_input(map)? else { return Ok(None) };
                Some(bound)
            }
            _ => None,
        };
        if passes.is_empty() {
            return Ok(Some(ResidentGrey16 { w, h, div: filter.div, alpha, pixels: Arc::new(input) }));
        }
        let mut outputs = Vec::with_capacity(passes.len());
        for _ in &passes {
            let Some(output) = pool.allocate_resident(w, h)? else { return Ok(None) };
            outputs.push(output);
        }
        if cancel.is_cancelled() {
            return Ok(None);
        }

        crate::validation::scoped(&self.device, "resident graph spatial filter", || {
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("graph grayscale filters") });
            let mut front = &input.buffer;
            let shift_buffer = shifts.as_ref().map(|shifts| {
                self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("graph line blur subpixel shifts"),
                    contents: bytemuck::cast_slice(shifts),
                    usage: wgpu::BufferUsages::STORAGE,
                })
            });
            let auxiliary = shift_buffer.as_ref().unwrap_or(&external_map.as_ref().unwrap_or(&input).buffer);
            let layout = self.pipeline.get_bind_group_layout(0);
            for (params, output) in passes.iter().zip(&outputs) {
                let settings = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("graph filter settings"),
                    contents: bytemuck::bytes_of(params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let buffers = [&settings, front, auxiliary, &output.buffer];
                let entries =
                    std::array::from_fn::<_, 4, _>(|i| wgpu::BindGroupEntry { binding: i as u32, resource: buffers[i].as_entire_binding() });
                let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("graph filter inputs"),
                    layout: &layout,
                    entries: &entries,
                });
                let mut pass =
                    encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("graph grayscale filter"), timestamp_writes: None });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(groups_x, groups_y, 1);
                front = &output.buffer;
            }
            self.queue.submit([encoder.finish()]);
        })?;
        input.retain_until_completed(&self.queue);
        if let Some(map) = external_map {
            map.retain_until_completed(&self.queue);
        }
        for pixels in &outputs {
            pixels.retain_until_completed(&self.queue);
        }
        Ok(outputs.pop().map(|pixels| ResidentGrey16 { w, h, div: filter.div, alpha, pixels }))
    }
}

/// One pass per axis of a box. An axis of at most half a pixel only runs to round to the node's format, and without
/// `round_between` it leaves that to the second axis when that one blurs.
fn push_box_passes(passes: &mut Vec<Params>, mut params: Params, radius: [f32; 2], round_between: bool) {
    let second_axis_blurs = radius[1] > 0.5;
    for (axis, radius) in radius.into_iter().enumerate() {
        let converts = params.source[2] != params.output[2];
        let rounds_later = axis == 0 && !round_between && second_axis_blurs;
        if radius <= 0.5 && (!converts || rounds_later) {
            continue;
        }

        params.control[1] = axis as u32;
        params.values[0] = radius;
        let radius = f64::from(radius);
        let edge = radius - ((radius - 0.5).ceil() - 0.5);
        params.values[1] = (edge / (1.0 + edge) * 256.0).round() as f32;
        passes.push(params);
        params.source = params.output;
    }
}

/// Compute each column's forward and reverse shear in the reference evaluator's precision.
fn shear_shifts(columns: usize, pad: i32, slope: f64) -> Vec<[i32; 2]> {
    let pack = |offset: f64| {
        let row = offset.floor();
        ((row as i32) << 9) | ((offset - row) * 256.0).round() as i32
    };
    (-pad..columns as i32 + pad)
        .map(|column| {
            let shift = (f64::from(column) + 0.5 - columns as f64 / 2.0) * slope;
            [pack(shift), pack(-shift)]
        })
        .collect()
}

fn same_input(a: GreyInput<'_>, b: GreyInput<'_>) -> bool {
    match (a, b) {
        (GreyInput::Host(a), GreyInput::Host(b)) => std::ptr::eq(a, b),
        (GreyInput::Device(a), GreyInput::Device(b)) => std::ptr::eq(a, b),
        _ => false,
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    source: [u32; 4],
    output: [u32; 4],
    map: [u32; 4],
    control: [u32; 4],
    values: [f32; 4],
    direction: [f32; 4],
    /// Always zero: the shader XORs it into each result the reference rounds on its own.
    rounding_barrier: [u32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_material_graph::{context::EvalContext, graph::Graph, profile::Profiler, tex::Tex};
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

    use crate::GraphAccelerator;

    #[derive(Default)]
    struct Nodes {
        resident: AtomicUsize,
        readbacks: AtomicUsize,
    }

    impl Profiler for Nodes {
        fn enabled(&self) -> bool {
            false
        }

        fn span(&self, _: &str) -> Box<dyn std::any::Any + Send> {
            Box::new(())
        }

        fn resident_node(&self, _: u32, _: &str) {
            self.resident.fetch_add(1, Relaxed);
        }

        fn resident_readback(&self, _: u32, _: usize) {
            self.readbacks.fetch_add(1, Relaxed);
        }
    }

    #[test]
    fn imported_grayscale_filters_match_the_cpu() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        for (accelerator, cpu_sampling) in gpu.spatial_sampling_modes() {
            check_imported_grayscale_filters(accelerator, cpu_sampling);
        }
    }

    fn check_imported_grayscale_filters(accelerator: GraphAccelerator, cpu_sampling: bool) {
        let nodes = Arc::new(Nodes::default());
        let context = EvalContext { accelerator: Some(Arc::new(accelerator)), profiler: Some(nodes.clone()), ..Default::default() };
        let size = 256_usize;
        let make_inputs = |map_size| {
            [
                Some(Tex::gray_fn(size, size, |u, v| 0.5 + 0.45 * (u * 18.0).sin() * (v * 20.0).cos())),
                Some(Tex::gray_fn(map_size, map_size, |u, v| 0.5 + 0.5 * (u * 9.0).sin() * (v * 15.0).cos())),
            ]
        };
        for (filter, div, options, map_size) in [
            ("dir_warp", 65535, "map=@2 angle=0.166 intensity=5", size),
            ("dir_warp", 65535, "map=@2 angle=-0.124 intensity=0.9 map_offset=0.5 tile_x=0 tile_y=0", size),
            ("dir_warp", 65535, "map=@2 angle=0.25 intensity=42", 64),
            ("dir_warp", 65535, "map=@2 angle=-0.25 intensity=42 tile_x=0 tile_y=0", 512),
            ("grad_warp", 65535, "map=@2 intensity=0.8 aspect_x=0.5 tile_x=0", 128),
            ("box_blur", 65535, "intensity=2.17 aspect_y=0.33", size),
            ("box_blur", 255, "intensity=1.7 tile_x=0 tile_y=0", size),
            ("box_blur", 65535, "intensity=8.3 aspect_y=0.65", size),
            ("box_blur", 255, "intensity=8.3 tile_y=0", size),
            ("line_blur", 65535, "intensity=1.3 angle=0.125", size),
            ("line_blur", 65535, "intensity=2.1 angle=0.25", size),
            ("line_blur", 65535, "intensity=5.28 angle=0.0625", size),
            ("line_blur", 65535, "intensity=10.56 angle=0.1875 tile_x=0 tile_y=0", size),
            ("line_blur", 65535, "intensity=5.28 angle=-0.125 tile_x=0", size),
            ("line_blur", 65535, "intensity=10.56 angle=0.25", size),
            ("line_blur", 65535, "intensity=26.4 angle=0", size),
            ("line_blur", 65535, "intensity=47.52 angle=0.25", size),
            ("line_blur", 65535, "intensity=47.52 angle=0.125", size),
            ("line_blur", 65535, "intensity=47.52 angle=-0.125", size),
            ("laplacian_sharpen", 255, "intensity=0.4", size),
        ] {
            nodes.resident.store(0, Relaxed);
            let format = if div == 255 { "u8_compiled" } else { "u16_compiled" };
            let map_log2 = map_size.ilog2();
            let graph = Graph::parse(&format!(
                "format 3\nsize {size}\nmeta compiled \"1\"\nnode 1 input idx=0 gray=1 format=\"{format}\" osize_mode=1\n\
                 node 2 input idx=1 gray=1 format=\"{format}\" osize_log2_x={map_log2} osize_log2_y={map_log2}\n\
                 node 3 {filter} in=@1 format=\"{format}\" {options}\noutput out 3\n"
            ))
            .unwrap();
            let inputs = make_inputs(map_size);
            let cpu = &graph.evaluate_with_inputs(None, &inputs).unwrap()["out"];
            let gpu = &graph.evaluate_in(&context, None, &inputs).unwrap()["out"];
            assert_eq!(nodes.resident.load(Relaxed), usize::from(!(cpu_sampling && filter == "dir_warp")), "{filter} {options}");
            assert_eq!((gpu.w, gpu.h, gpu.px.len()), (cpu.w, cpu.h, cpu.px.len()), "{filter} {options}");
            let errors = cpu.px.iter().zip(&gpu.px).map(|(a, b)| ((a[0] - b[0]).abs() * div as f32).round() as u32);
            let (max, sum) = errors.fold((0, 0), |(max, sum), error| (max.max(error), sum + error));
            let mean = f64::from(sum) / cpu.px.len() as f64;
            // Sampling a differently sized map can cross one 1/256 interpolation weight.
            let max_steps = if map_size != size { 16 } else { 1 };
            assert!(max <= max_steps && mean < 0.05, "{filter} {options}: errors {max} maximum, {mean} mean quantization steps");
        }
    }

    #[test]
    fn box_axes_round_to_the_node_format_like_the_cpu() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let nodes = Arc::new(Nodes::default());
        let context = EvalContext { accelerator: Some(Arc::new(gpu.accelerator())), profiler: Some(nodes.clone()), ..Default::default() };
        // 16-bit detail that rounding to 8 bits before the blur loses
        let inputs = [Some(Tex::gray_fn(256, 256, |u, v| 0.5 + 0.004 * (u * 301.0).sin() + 0.003 * (v * 457.0).cos()))];
        for (filter, format, options) in [
            ("line_blur", "u8_compiled", "intensity=1.3 angle=0.25"),
            ("line_blur", "u16_compiled", "intensity=1.3 angle=0.25"),
            ("box_blur", "u8_compiled", "intensity=1.7"),
            ("box_blur", "u8_compiled", "intensity=1.7 aspect_x=0"),
        ] {
            nodes.resident.store(0, Relaxed);
            let graph = Graph::parse(&format!(
                "format 3\nsize 256\nmeta compiled \"1\"\nnode 1 input idx=0 gray=1 format=\"u16_compiled\"\n\
                 node 2 {filter} in=@1 {options} format=\"{format}\"\noutput out 2\n"
            ))
            .unwrap();

            let cpu = &graph.evaluate_with_inputs(None, &inputs).unwrap()["out"];
            let gpu = &graph.evaluate_in(&context, None, &inputs).unwrap()["out"];

            let label = format!("{filter} {options} into {format}");
            assert_eq!(nodes.resident.load(Relaxed), 1, "{label} must run on the GPU");
            assert_eq!((gpu.w, gpu.h, gpu.px.len()), (cpu.w, cpu.h, cpu.px.len()), "{label}");
            let mismatch = cpu.px.iter().zip(&gpu.px).position(|(a, b)| a != b);
            assert_eq!(mismatch, None, "{label}: first mismatched pixel");
        }
    }

    #[test]
    fn wrapped_taps_just_below_a_negative_tie_keep_the_reference_weight() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let side = 256.0_f32;
        let source = format!(
            "{}\n@group(0) @binding(0) var<storage, read> positions: array<vec2f>;\n\
             @group(0) @binding(1) var<storage, read_write> taps: array<vec4f>;\n\
             @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) id: vec3u) {{\n\
                 let read = stepped_taps(positions[id.x], vec2u({side}u), vec2(true));\n\
                 taps[id.x] = vec4f(vec2f(read.base), read.weights);\n\
             }}",
            include_str!("shaders/graph_sampling.wgsl")
        );
        // Texel coordinates at, and a few floats either side of, every 1/256 tie on both sides of zero
        let mut scaled = Vec::new();
        for tie in -256..256 {
            let tie = (tie as f32 + 0.5) / 256.0;
            let below = |x: f32, n: u32| if x < 0.0 { f32::from_bits(x.to_bits() + n) } else { f32::from_bits(x.to_bits() - n) };
            let above = |x: f32, n: u32| if x < 0.0 { f32::from_bits(x.to_bits() - n) } else { f32::from_bits(x.to_bits() + n) };
            scaled.extend([tie, below(tie, 1), below(tie, 2), above(tie, 1), above(tie, 2)]);
        }
        let positions = scaled.iter().map(|&s| [s / side, s / side]).collect::<Vec<_>>();

        let bytes = gpu.run_compute(&source, bytemuck::cast_slice(&positions), (positions.len() * 16) as u64, positions.len() as u32);

        let taps = bytemuck::cast_slice::<u8, [f32; 4]>(&bytes);
        for (&s, tap) in scaled.iter().zip(taps) {
            // The reference read in double precision: the texel before the position and its weight in steps of 1/256, half up
            let x = f64::from(s) - 0.5;
            let base = x.floor();
            let weight = (((x - base) * 256.0 + 0.5).floor() / 256.0) as f32;
            assert_eq!([tap[0], tap[2]], [base as f32, weight], "texel coordinate {s:e}");
        }
    }

    #[test]
    fn texel_centers_round_like_the_reference_division() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let source = format!(
            "{}\n@group(0) @binding(0) var<storage, read> pixels: array<vec2u>;\n\
             @group(0) @binding(1) var<storage, read_write> centers: array<f32>;\n\
             @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3u) {{\n\
                 if id.x < arrayLength(&pixels) {{ centers[id.x] = texel_center(pixels[id.x].x, pixels[id.x].y); }}\n\
             }}",
            include_str!("shaders/graph_sampling.wgsl")
        );
        // Every pixel of small sides and of large sides with and without a power of two, up to the largest resident side
        let sides = (1..=130).chain([1000, 1022, 1023, 1366, 2046, 2047, 2048, 3000, 4094, 4095, 4096, 6000, 8190, 8191, 8192]);
        let pixels = sides.flat_map(|side: u32| (0..side).map(move |p| [p, side])).collect::<Vec<_>>();

        let bytes = gpu.run_compute(&source, bytemuck::cast_slice(&pixels), (pixels.len() * 4) as u64, pixels.len().div_ceil(64) as u32);

        let centers = bytemuck::cast_slice::<u8, f32>(&bytes);
        for (&[p, side], &center) in pixels.iter().zip(centers) {
            assert_eq!(center.to_bits(), ((p as f32 + 0.5) / side as f32).to_bits(), "pixel {p} of {side}");
        }
    }

    #[test]
    fn imported_levels_and_gradient_keep_a_smaller_warp_resident() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let nodes = Arc::new(Nodes::default());
        let context = EvalContext { accelerator: Some(Arc::new(gpu.accelerator())), profiler: Some(nodes.clone()), ..Default::default() };
        let graph = Graph::parse(
            "format 3\nsize 512\nmeta compiled \"1\"\n\
             node 1 input idx=0 gray=1 format=\"u16_compiled\"\n\
             node 2 input idx=1 gray=1 format=\"u16_compiled\"\n\
             node 3 dir_warp in=@1 map=@2 intensity=0 format=\"u16_compiled\"\n\
             node 4 levels_mid in=@3 mid=0.4 format=\"u16_compiled\"\n\
             node 5 colorize in=@4 table=1 p0=0 r0=0 g0=0 b0=0 p1=0.4 r1=1 g1=1 b1=1 p2=1 r2=0.2 g2=0.2 b2=0.2 format=\"u16_compiled\"\n\
             output out 5\n",
        )
        .unwrap();
        let inputs =
            [Some(Tex::gray_fn(512, 512, |u, v| 0.5 + 0.4 * (u * 18.0).sin() * (v * 20.0).cos())), Some(Tex::gray_fn(512, 512, |u, v| u * v))];
        let cpu = graph.evaluate_with_inputs(None, &inputs).unwrap();
        let accelerated = graph.evaluate_in(&context, None, &inputs).unwrap();
        assert_eq!(nodes.resident.load(Relaxed), 3);
        assert_eq!(nodes.readbacks.load(Relaxed), 1);
        assert_eq!(cpu["out"].px, accelerated["out"].px);
    }

    #[test]
    fn affine_transform_keeps_packed_grey_and_alpha() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        for (accelerator, cpu_sampling) in gpu.spatial_sampling_modes() {
            check_packed_grey_affine_transform(accelerator, cpu_sampling);
        }
    }

    #[test]
    #[ignore = "checks the GPU sampling that Metal leaves to the reference evaluator"]
    fn affine_transform_keeps_packed_grey_and_alpha_with_gpu_sampling() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        check_packed_grey_affine_transform(gpu.gpu_spatial_sampling(), false);
    }

    #[test]
    #[ignore = "checks the GPU sampling that Metal leaves to the reference evaluator"]
    fn imported_grayscale_filters_match_the_cpu_with_gpu_sampling() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        check_imported_grayscale_filters(gpu.gpu_spatial_sampling(), false);
    }

    fn check_packed_grey_affine_transform(accelerator: GraphAccelerator, cpu_sampling: bool) {
        let nodes = Arc::new(Nodes::default());
        let context = EvalContext { accelerator: Some(Arc::new(accelerator)), profiler: Some(nodes.clone()), ..Default::default() };
        let source = [Some(Tex::gray_fn(256, 256, |u, v| 0.5 + 0.5 * (u * 39.0).sin() * (v * 53.0).cos()))];
        for (log2, transform) in [
            (10, "offset_x=-0.0009765625"),
            (10, "offset_y=-0.0009765625"),
            (8, "m00=2 m11=2"),
            (8, "m00=4 m11=4"),
            (8, "m00=0 m11=0 m01=-1 m10=1"),
            (8, "m00=1.4 m11=0.85 m01=0.23 m10=-0.3 offset_x=0.34 filter=0 mip=0"),
            // A shear where a fused or reordered coordinate sum picks another texel or 1/256 weight
            (11, "m00=1.4 m11=0.85 m01=0.23 m10=-0.3 offset_x=0.34 filter=0 mip=0"),
            (11, "m00=1.4 m11=0.85 m01=0.23 m10=-0.3 offset_x=0.34 mip=0"),
        ] {
            nodes.resident.store(0, Relaxed);
            nodes.readbacks.store(0, Relaxed);
            let graph = Graph::parse(&format!(
                "format 3\nsize 256\nmeta compiled \"1\"\nnode 1 input idx=0 gray=1 format=\"u16_compiled\"\n\
                 node 2 levels_mid in=@1 out_low=1 out_high=0 format=\"u16_compiled\"\n\
                 node 3 transform_matrix in=@2 {transform} osize_log2_x={log2} osize_log2_y={log2} format=\"u16_compiled\"\n\
                 node 4 laplacian_sharpen in=@3 intensity=0 format=\"u16_compiled\"\noutput out 4\n"
            ))
            .unwrap();
            let cpu = &graph.evaluate_with_inputs(None, &source).unwrap()["out"];
            let gpu = &graph.evaluate_in(&context, None, &source).unwrap()["out"];
            assert_eq!(nodes.resident.load(Relaxed), if cpu_sampling { 1 } else { 2 }, "{transform}");
            assert_eq!(nodes.readbacks.load(Relaxed), 1, "{transform}");
            assert_eq!((gpu.w, gpu.h, gpu.px.len()), (1 << log2, 1 << log2, cpu.px.len()));
            assert!(cpu.px.iter().zip(&gpu.px).all(|(a, b)| a[3] == 0.0 && b[3] == 0.0 && (a[0] - b[0]).abs() <= 1.01 / 65535.0), "{transform}");
        }
    }
}
