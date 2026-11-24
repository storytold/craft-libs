//! Compiled color graph operations with the same GPU storage budget as grayscale nodes.

use craft_material_graph::accelerator::{ColorInput, ResidentColor16, ResidentColorOutput, ResidentColorPass, ResidentGrey16};
use craft_material_graph::context::Cancel;
use wgpu::util::DeviceExt;

use crate::gpu_graph_grey::{GreyDevicePixels, GreyRasterizer};

const SHADER: &str = concat!(
    include_str!("shaders/graph_color.wgsl"),
    "\n",
    include_str!("shaders/graph_compiled_grey.wgsl"),
    "\n",
    include_str!("shaders/graph_blend.wgsl"),
    "\n",
    include_str!("shaders/graph_sampling.wgsl"),
);

const BLEND: u32 = 0;
const GREY_PACKED: u32 = 1;
const GREY_ALPHA: u32 = 2;
const DIRECTIONAL_WARP: u32 = 3;
const WRAPPED_AFFINE: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    extent: [u32; 4],
    formats: [u32; 4],
    alphas: [f32; 4],
    weights: [f32; 4],
    control: [u32; 4],
    direction: [f32; 4],
    /// Always zero: the shader XORs it into each result the reference rounds on its own.
    rounding_barrier: [u32; 4],
}

pub(crate) struct ColorRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
}

impl ColorRasterizer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, String> {
        let pipeline = crate::validation::scoped(device, "graph color shader", || {
            let module = device
                .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("graph color"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("graph color"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            })
        })?;
        Ok(Self { device: device.clone(), queue: queue.clone(), pipeline })
    }

    fn bind(&self, pool: &GreyRasterizer, input: ColorInput<'_>, area: usize) -> Result<Option<GreyDevicePixels>, String> {
        match input {
            ColorInput::Grey(image) => pool.bind_input(image),
            ColorInput::Host(image) if image.px.len() == area => pool.upload_pixels(bytemuck::cast_slice(&image.px)),
            ColorInput::Host(_) => Ok(None),
            ColorInput::Device(image) => {
                Ok(GreyDevicePixels::from_pixels(image.pixels.as_ref(), area as u64 * 8).filter(|pixels| pixels.belongs_to(&self.device)).cloned())
            }
        }
    }

    pub(crate) fn render(&self, pass: ResidentColorPass<'_>, pool: &GreyRasterizer, cancel: &Cancel) -> Result<Option<ResidentColorOutput>, String> {
        let mut params = Params::default();
        let (front, back, mask, op, alpha) = match pass {
            ResidentColorPass::Blend { foreground, background, mask, mode, opacity, alpha_mode, alpha } => {
                params.extent[3] = mode;
                params.alphas[3] = opacity;
                params.control[0] = alpha_mode;
                (foreground, Some(background), mask, BLEND, alpha)
            }
            ResidentColorPass::Grayscale { source, weights, alpha } => {
                params.weights = weights;
                (source, None, None, if alpha.is_some() { GREY_PACKED } else { GREY_ALPHA }, alpha)
            }
            ResidentColorPass::DirectionalWarp { source, map, settings, alpha } => {
                let (mw, mh, ..) = map.info();
                if ![mw, mh].into_iter().chain(input_size(source)).all(usize::is_power_of_two) || mw < 2 {
                    return Ok(None);
                }
                let [tile_x, tile_y] = settings.tile.map(u32::from);
                params.control = [mw as u32, mh as u32, tile_x, tile_y];
                params.weights = [settings.intensity, settings.offset, settings.aspect[0], settings.aspect[1]];
                params.direction[..2].copy_from_slice(&settings.direction);
                (source, Some(ColorInput::Grey(map)), None, DIRECTIONAL_WARP, alpha)
            }
            ResidentColorPass::WrappedAffine { source, matrix, offset } => {
                if source.w != source.h || !source.w.is_power_of_two() {
                    return Ok(None);
                }
                params.direction = matrix;
                params.weights[..2].copy_from_slice(&offset);
                (ColorInput::Device(source), None, None, WRAPPED_AFFINE, source.alpha)
            }
        };
        let [w, h] = input_size(front);
        if cancel.is_cancelled() || !(2..=8192).contains(&w) || !w.is_multiple_of(2) || !(1..=8192).contains(&h) {
            return Ok(None);
        }
        let group = [(w as u32 / 2).div_ceil(8), (h as u32).div_ceil(8)];
        let limit = self.device.limits().max_compute_workgroups_per_dimension;
        if group.into_iter().any(|g| g > limit) || op == BLEND && back.into_iter().chain(mask).any(|input| input_size(input) != [w, h]) {
            return Ok(None);
        }
        let [first, second, third] = [front, back.unwrap_or(front), mask.unwrap_or(front)].map(input_format);
        let (Some((format_f, alpha_f)), Some((format_b, alpha_b)), Some((format_m, alpha_m))) = (first, second, third) else { return Ok(None) };
        params.extent[..3].copy_from_slice(&[w as u32, h as u32, op]);
        params.formats = [format_f, format_b, format_m, u32::from(mask.is_some())];
        params.alphas[..3].copy_from_slice(&[alpha_f, alpha_b, alpha_m]);
        self.device.poll(wgpu::PollType::Poll).map_err(|error| error.to_string())?;
        let (front_input, back_input) = (front, back);
        let Some(front) = self.bind(pool, front, w * h)? else { return Ok(None) };
        // An image connected to several slots is uploaded once
        let back = match back {
            Some(image) if same_input(image, front_input) => Some(front.clone()),
            Some(image) => {
                let [width, height] = input_size(image);
                let Some(bound) = self.bind(pool, image, width * height)? else { return Ok(None) };
                Some(bound)
            }
            None => None,
        };
        let mask = match mask {
            Some(image) if same_input(image, front_input) => Some(front.clone()),
            Some(image) if back_input.is_some_and(|back_input| same_input(image, back_input)) => back.clone(),
            Some(image) => {
                let Some(bound) = self.bind(pool, image, w * h)? else { return Ok(None) };
                Some(bound)
            }
            None => None,
        };
        let Some(output) = pool.allocate_resident(if op == GREY_PACKED { w } else { w * 4 }, h)? else { return Ok(None) };
        if cancel.is_cancelled() {
            return Ok(None);
        }

        crate::validation::scoped(&self.device, "resident graph color pass", || {
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("resident graph color") });
            let settings = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("resident graph color parameters"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let buffers =
                [&settings, &front.buffer, &back.as_ref().unwrap_or(&front).buffer, &mask.as_ref().unwrap_or(&front).buffer, &output.buffer];
            let entries = std::array::from_fn::<_, 5, _>(|i| wgpu::BindGroupEntry { binding: i as u32, resource: buffers[i].as_entire_binding() });
            let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("resident graph color inputs"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &entries,
            });
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(group[0], group[1], 1);
            drop(pass);
            self.queue.submit([encoder.finish()]);
        })?;
        for pixels in [&front, output.as_ref()].into_iter().chain(back.as_ref()).chain(mask.as_ref()) {
            pixels.retain_until_completed(&self.queue);
        }
        Ok(Some(match (op, alpha) {
            (GREY_PACKED, Some(alpha)) => ResidentColorOutput::Grey(ResidentGrey16 { w, h, div: 65535.0, alpha, pixels: output }),
            _ => ResidentColorOutput::Color(ResidentColor16 { w, h, alpha, pixels: output }),
        }))
    }
}

fn input_size(input: ColorInput<'_>) -> [usize; 2] {
    match input {
        ColorInput::Grey(image) => {
            let (w, h, ..) = image.info();
            [w, h]
        }
        ColorInput::Host(image) => [image.w, image.h],
        ColorInput::Device(image) => [image.w, image.h],
    }
}

fn same_input(a: ColorInput<'_>, b: ColorInput<'_>) -> bool {
    use craft_material_graph::accelerator::GreyInput;
    match (a, b) {
        (ColorInput::Host(a), ColorInput::Host(b)) => std::ptr::eq(a, b),
        (ColorInput::Device(a), ColorInput::Device(b)) => std::ptr::eq(a, b),
        (ColorInput::Grey(GreyInput::Host(a)), ColorInput::Grey(GreyInput::Host(b))) => std::ptr::eq(a, b),
        (ColorInput::Grey(GreyInput::Device(a)), ColorInput::Grey(GreyInput::Device(b))) => std::ptr::eq(a, b),
        _ => false,
    }
}

fn input_format(input: ColorInput<'_>) -> Option<(u32, f32)> {
    match input {
        ColorInput::Grey(image) => {
            let (_, _, div, alpha) = image.info();
            match div {
                65535.0 => Some((0, alpha)),
                255.0 => Some((1, alpha)),
                _ => None,
            }
        }
        ColorInput::Host(_) => Some((2, 0.0)),
        ColorInput::Device(_) => Some((3, 0.0)),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    use crate::GraphAccelerator;
    use craft_material_graph::{context::EvalContext, graph::Graph, profile::Profiler, tex::Tex};

    #[derive(Default)]
    struct Nodes(Mutex<BTreeMap<String, usize>>);

    impl Profiler for Nodes {
        fn enabled(&self) -> bool {
            false
        }

        fn span(&self, _: &str) -> Box<dyn std::any::Any + Send> {
            Box::new(())
        }

        fn resident_node(&self, _: u32, kind: &str) {
            *self.0.lock().unwrap().entry(kind.into()).or_default() += 1;
        }

        fn resident_readback(&self, _: u32, _: usize) {
            *self.0.lock().unwrap().entry("readback".into()).or_default() += 1;
        }
    }

    #[test]
    fn a_blend_of_one_host_image_with_itself_uploads_it_once() {
        use craft_material_graph::accelerator::{ColorInput, ResidentColorPass};
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        // A 2 MiB resident budget holds one 1 MiB upload and the 512 KiB output, not a second upload
        let pool = crate::gpu_graph_grey::GreyRasterizer::new(&gpu.device, &gpu.queue, || 16 << 20).unwrap();
        let rasterizer = super::ColorRasterizer::new(&gpu.device, &gpu.queue).unwrap();
        let image = Tex::from_fn(256, 256, |u, v| [u, v, u * v, 1.0]);
        let cancel = craft_material_graph::context::Cancel::new();
        let pass = ResidentColorPass::Blend {
            foreground: ColorInput::Host(&image),
            background: ColorInput::Host(&image),
            mask: Some(ColorInput::Host(&image)),
            mode: 0,
            opacity: 0.5,
            alpha_mode: 0,
            alpha: None,
        };

        let output = rasterizer.render(pass, &pool, &cancel).unwrap();

        assert!(output.is_some(), "the blend must stay on the GPU within the budget");
    }

    #[test]
    fn compiled_color_blends_and_grayscale_stay_resident() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let nodes = Arc::new(Nodes::default());
        let ctx = EvalContext { accelerator: Some(Arc::new(gpu.accelerator())), profiler: Some(nodes.clone()), ..Default::default() };
        let graph = Graph::parse(
            "format 3\nsize 1024\nmeta compiled \"1\"\n\
             node 1 input idx=0 format=\"u16_compiled\"\nnode 2 input idx=1 gray=1 format=\"u8_compiled\"\n\
             node 3 input idx=2 format=\"u16_compiled\"\n\
             node 4 blend fg=@2 bg=@1 mask=@3 mode=\"add_sub_compiled\" opacity=0.62 format=\"u16_compiled\"\n\
             node 5 blend fg=@4 bg=@1 mode=\"switch\" alpha_mode=1 opacity=0.34 format=\"u16_compiled\"\n\
             node 6 grayscale_conversion in=@5 r=0.299 g=0.587 b=0.114 a=0.02 limit=1 format=\"u16_compiled\"\n\
             node 7 input idx=1 gray=1 format=\"u16_compiled\"\n\
             node 8 blend fg=@7 bg=@6 mode=\"multiply\" opacity=0.38 format=\"u16_compiled\"\noutput out 8\n",
        )
        .unwrap();
        for varying_alpha in [false, true] {
            let inputs = [
                Some(Tex::from_fn(1024, 1024, |u, v| [u, v, (u + v) * 0.5, if varying_alpha { u } else { 1.0 }])),
                Some(Tex::gray_fn(1024, 1024, |u, v| 0.5 + 0.5 * (u * 32.0).sin() * (v * 33.0).cos())),
                Some(Tex::from_fn(1024, 1024, |u, v| [v, 1.0 - u, 1.0 - v, u])),
            ];
            nodes.0.lock().unwrap().clear();
            let cpu = &graph.evaluate_with_inputs(None, &inputs).unwrap()["out"];
            let gpu = &graph.evaluate_in(&ctx, None, &inputs).unwrap()["out"];
            let seen = nodes.0.lock().unwrap();
            assert_eq!(seen.get("blend"), Some(&3), "varying alpha={varying_alpha}, {seen:?}");
            assert_eq!(seen.get("grayscale_conversion"), Some(&1));
            assert_eq!(seen.get("readback"), Some(&1));
            assert_eq!((gpu.w, gpu.h, gpu.px.len()), (cpu.w, cpu.h, cpu.px.len()));
            let max = cpu
                .px
                .iter()
                .zip(&gpu.px)
                .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| ((a - b).abs() * 65535.0).round() as u32))
                .max()
                .unwrap_or_default();
            assert!(max <= 1, "varying alpha={varying_alpha}, {max} u16 steps");
        }
    }

    #[test]
    fn compiled_color_warps_match_the_cpu_through_blends() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        for (accelerator, cpu_sampling) in gpu.spatial_sampling_modes() {
            check_color_warps_through_blends(accelerator, cpu_sampling);
        }
    }

    #[test]
    #[ignore = "checks the GPU sampling that Metal leaves to the reference evaluator"]
    fn compiled_color_warps_match_the_cpu_through_blends_with_gpu_sampling() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        check_color_warps_through_blends(gpu.gpu_spatial_sampling(), false);
    }

    fn check_color_warps_through_blends(accelerator: GraphAccelerator, cpu_sampling: bool) {
        let nodes = Arc::new(Nodes::default());
        let ctx = EvalContext { accelerator: Some(Arc::new(accelerator)), profiler: Some(nodes.clone()), ..Default::default() };
        let source = Tex::from_fn(1024, 1024, |u, v| [u, v, (u * 19.0 + v * 12.0).cos() * 0.5 + 0.5, u * v]);
        for log2 in [9, 10] {
            let graph = Graph::parse(&format!(
                "format 3\nsize 1024\nmeta compiled \"1\"\nnode 1 input idx=0 format=\"u16_compiled\"\n\
                 node 2 input idx=1 gray=1 osize_log2_x={log2} osize_log2_y={log2} format=\"u16_compiled\"\n\
                 node 3 dir_warp in=@1 map=@2 angle=-0.25 intensity=26.865005 format=\"u16_compiled\"\n\
                 node 4 blend fg=@3 bg=@1 mode=\"switch\" opacity=0.5 format=\"u16_compiled\"\n\
                 node 5 dir_warp in=@4 map=@2 angle=0.123 intensity=5.7 map_offset=0.13 aspect_x=0.75 tile_x=0 format=\"u16_compiled\"\noutput out 5\n",
            ))
            .unwrap();
            let width = 1 << log2;
            let map = Tex::gray_fn(width, width, |u, v| 0.5 + 0.5 * (u * 37.0).sin() * (v * 11.0).cos());
            let inputs = [Some(source.clone()), Some(map)];
            nodes.0.lock().unwrap().clear();
            let cpu = &graph.evaluate_with_inputs(None, &inputs).unwrap()["out"];
            let gpu = &graph.evaluate_in(&ctx, None, &inputs).unwrap()["out"];
            let seen = nodes.0.lock().unwrap();
            assert_eq!(seen.get("dir_warp"), (!cpu_sampling).then_some(&2), "map {width}²: {seen:?}");
            assert_eq!(seen.get("blend"), Some(&1));
            assert_eq!(seen.get("readback"), Some(&1));
            assert_eq!((gpu.w, gpu.h, gpu.px.len()), (cpu.w, cpu.h, cpu.px.len()));
            let max = cpu
                .px
                .iter()
                .zip(&gpu.px)
                .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| ((a - b).abs() * 65535.0).round() as u32))
                .max()
                .unwrap_or_default();
            assert!(max <= 1, "map {width}²: {max} u16 steps");
        }
    }

    #[test]
    fn wrapped_color_transforms_keep_the_compiled_sample_and_alpha() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        for (accelerator, cpu_sampling) in gpu.spatial_sampling_modes() {
            check_wrapped_color_transform(accelerator, cpu_sampling);
        }
    }

    #[test]
    #[ignore = "checks the GPU sampling that Metal leaves to the reference evaluator"]
    fn wrapped_color_transforms_keep_the_compiled_sample_and_alpha_with_gpu_sampling() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        check_wrapped_color_transform(gpu.gpu_spatial_sampling(), false);
    }

    fn check_wrapped_color_transform(accelerator: GraphAccelerator, cpu_sampling: bool) {
        let nodes = Arc::new(Nodes::default());
        let context = EvalContext { accelerator: Some(Arc::new(accelerator)), profiler: Some(nodes.clone()), ..Default::default() };
        let graph = Graph::parse(
            r#"format 3
size 1024
meta compiled "1"
subgraph offset
meta kind "function"
meta float32 "1"
node 2 f_float v=0.12793
node 1 f_out in=@2
end
node 1 input idx=0 format="u16_compiled"
node 2 blend fg=@1 bg=@1 mode="copy" alpha_mode=1 format="u16_compiled"
node 3 transform_matrix in=@2 m00=-0.00000004371139 m01=-1 m10=1 m11=-0.00000004371139 fn.offset_x="offset" offset_y=-0.29517 format="u16_compiled"
node 4 transform_matrix in=@3 m00=0.81 m01=0.37 m10=-0.27 m11=1.1 mip=0 offset_y=0.20998 format="u16_compiled"
output out 4
"#,
        )
        .unwrap();
        let input =
            Tex::from_fn(1024, 1024, |u, v| [u * v, (u * 1.7 + v).fract(), (u * 19.0 + v * 12.0).cos() * 0.5 + 0.5, (u * 3.0 + 2.0 * v).fract()]);
        let inputs = [Some(input)];
        let cpu = &graph.evaluate_with_inputs(None, &inputs).unwrap()["out"];
        let gpu = &graph.evaluate_in(&context, None, &inputs).unwrap()["out"];
        let seen = nodes.0.lock().unwrap();
        assert_eq!(seen.get("blend"), Some(&1), "{seen:?}");
        assert_eq!(seen.get("transform_matrix"), (!cpu_sampling).then_some(&2), "{seen:?}");
        assert_eq!(seen.get("readback"), Some(&1), "{seen:?}");
        assert_eq!((gpu.w, gpu.h, gpu.px.len()), (cpu.w, cpu.h, cpu.px.len()));
        let mismatch = cpu.px.iter().zip(&gpu.px).position(|(a, b)| a != b);
        assert_eq!(mismatch, None, "first mismatched RGBA pixel");
    }
}
