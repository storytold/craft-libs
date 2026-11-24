//! Ordered, spatially binned drawing for imported procedural pattern programs.

use std::ops::Range;

use craft_material_graph::patterns::{RasterOutput, RasterRequest, RasterStamp};
use craft_material_graph::tex::{Grey, Tex};
use wgpu::util::DeviceExt;

const TILE: u32 = 8;
const READBACK_BYTES: u64 = 8 << 20;
// Sparse stamps can still hit the same tile repeatedly. Bound the packed bin lists independently of the output.
const BIN_BYTES: u64 = 48 << 20;
const STAMP_BYTES: u64 = 48 << 20;

pub(crate) struct PatternRasterizer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    stamp_bytes: u64,
}

impl PatternRasterizer {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, String> {
        let pipeline = crate::validation::scoped(device, "graph pattern raster shader", || {
            let module = device.create_shader_module(wgpu::include_wgsl!("shaders/graph_patterns.wgsl"));
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("graph pattern raster"),
                layout: None,
                module: &module,
                entry_point: Some("raster"),
                compilation_options: Default::default(),
                cache: None,
            })
        })?;
        Ok(Self { device: device.clone(), queue: queue.clone(), pipeline, stamp_bytes: STAMP_BYTES })
    }

    pub(crate) fn render(&self, request: RasterRequest<'_>) -> Result<Option<RasterOutput>, String> {
        let limits = self.device.limits();
        let max_bytes = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
        let [w, h] = request.size.map(|s| s as u32);
        let grey = request.grey_alpha.is_some();
        let pixel_bytes = u64::from(w) * if grey { 4 } else { 16 };
        if w == 0 || h == 0 || pixel_bytes > max_bytes.min(READBACK_BYTES) || request.cancel.is_cancelled() {
            return Ok(None);
        }
        if request.region.iter().chain(&request.period).chain(&request.clear).any(|v| !v.is_finite()) {
            return Ok(None);
        }
        let [bw, bh] = [w.div_ceil(TILE), h.div_ceil(TILE)];
        let stamp_limit = (max_bytes.min(self.stamp_bytes) / size_of::<Stamp>() as u64) as usize;
        if bw > limits.max_compute_workgroups_per_dimension || stamp_limit == 0 {
            return Ok(None);
        }
        let bin_limit = max_bytes.min(BIN_BYTES);
        let whole = (request.stamps.len() <= stamp_limit).then(|| RasterBatch::new(&request, bw, 0..bh, stamp_limit, bin_limit)).flatten();
        let Some(images) = pack_images(&request, max_bytes) else { return Ok(None) };
        let device = &self.device;
        let queue = &self.queue;
        let input = |label: &str, contents: &[u8]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage: wgpu::BufferUsages::STORAGE })
        };
        let band_height = ((max_bytes.min(READBACK_BYTES) / pixel_bytes) as u32).min(h).min(TILE * limits.max_compute_workgroups_per_dimension);
        if band_height == 0 {
            return Ok(None);
        }
        let (images, out, readback, uniform) = crate::validation::scoped(device, "graph pattern raster buffers", || {
            let buffer =
                |label, size, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
            (
                input("graph pattern images", bytemuck::cast_slice(&images)),
                buffer("graph pattern output", pixel_bytes * u64::from(band_height), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC),
                buffer("graph pattern readback", pixel_bytes * u64::from(band_height), wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
                buffer("graph pattern parameters", std::mem::size_of::<Params>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
            )
        })?;
        let mut grey_pixels = Vec::new();
        let mut rgba_pixels = Vec::new();
        if grey {
            grey_pixels.reserve_exact((w * h) as usize);
        } else {
            rgba_pixels.reserve_exact((w * h) as usize);
        }
        let mut render = |batch: RasterBatch, tile_rows: Range<u32>| -> Result<bool, String> {
            let (stamps, bins) = crate::validation::scoped(device, "graph pattern batch", || {
                (input("graph pattern stamps", bytemuck::cast_slice(&batch.stamps)), input("graph pattern bins", bytemuck::cast_slice(&batch.bins)))
            })?;
            drop(batch);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("graph pattern inputs"),
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: stamps.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: bins.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: images.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: out.as_entire_binding() },
                ],
            });
            let end = (tile_rows.end * TILE).min(h);
            for y in (tile_rows.start * TILE..end).step_by(band_height as usize) {
                if request.cancel.is_cancelled() {
                    return Ok(false);
                }
                let rows = (end - y).min(band_height);
                let params = Params {
                    image: [w, h, y, tile_rows.start],
                    mode: [
                        rows,
                        u32::from(request.color) | (u32::from(grey) << 1),
                        u32::from(request.framed),
                        u32::from(request.background.is_some()),
                    ],
                    region: request.region,
                    period: [request.period[0], request.period[1], 0.0, 0.0],
                    clear: request.clear,
                };
                let bytes = pixel_bytes * u64::from(rows);
                queue.write_buffer(&uniform, 0, bytemuck::bytes_of(&params));
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("graph pattern raster") });
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&self.pipeline);
                    pass.set_bind_group(0, &bind_group, &[]);
                    pass.dispatch_workgroups(bw, rows.div_ceil(TILE), 1);
                }
                encoder.copy_buffer_to_buffer(&out, 0, &readback, 0, bytes);
                let submission = queue.submit([encoder.finish()]);
                let slice = readback.slice(..bytes);
                let (sender, receiver) = std::sync::mpsc::channel();
                slice.map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
                device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: None }).map_err(|e| e.to_string())?;
                receiver.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
                let result = (|| {
                    let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
                    if grey {
                        grey_pixels.extend_from_slice(bytemuck::try_cast_slice::<u8, f32>(&data).map_err(|e| e.to_string())?);
                    } else {
                        rgba_pixels.extend_from_slice(bytemuck::try_cast_slice::<u8, [f32; 4]>(&data).map_err(|e| e.to_string())?);
                    }
                    Ok::<(), String>(())
                })();
                readback.unmap();
                result?;
            }
            Ok(true)
        };
        if let Some(whole) = whole {
            if !render(whole, 0..bh)? {
                return Ok(None);
            }
        } else {
            // Each band receives only the stamps that touch its rows, in original order
            let mut start = 0;
            let mut rows = (band_height / TILE).max(1).min(bh.div_ceil(2));
            while start < bh {
                let end = (start + rows).min(bh);
                if let Some(batch) = RasterBatch::new(&request, bw, start..end, stamp_limit, bin_limit) {
                    if !render(batch, start..end)? {
                        return Ok(None);
                    }
                    start = end;
                } else if end - start == 1 || request.cancel.is_cancelled() {
                    return Ok(None);
                } else {
                    rows = (end - start).div_ceil(2);
                }
            }
        }
        let [w, h] = request.size;
        Ok(Some(match request.grey_alpha {
            Some(alpha) => RasterOutput::Grey(Grey { w, h, px: grey_pixels }, alpha),
            None => RasterOutput::Rgba(Tex { w, h, px: rgba_pixels }),
        }))
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    image: [u32; 4],
    mode: [u32; 4],
    region: [f32; 4],
    period: [f32; 4],
    clear: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Stamp {
    bounds: [i32; 4],
    repeats: [i32; 4],
    placement: [f32; 4],
    pattern: [u32; 4],
    brightness: [f32; 4],
}

impl From<&RasterStamp> for Stamp {
    fn from(stamp: &RasterStamp) -> Self {
        let pattern = if (0..=16).contains(&stamp.pattern) { stamp.pattern as u32 } else { 1 };
        // Image slot zero is the built-in shape. Nonzero slots are shifted by one to leave room for the background.
        let image = stamp.image.map_or(0, |i| i + 1);
        let flags = pattern | (image << 8) | (u32::from(stamp.nearest) << 30) | (u32::from(stamp.keep_larger) << 31);
        Self {
            bounds: stamp.bounds,
            repeats: stamp.repeats,
            placement: [stamp.center[0], stamp.center[1], stamp.half_size[0], stamp.half_size[1]],
            pattern: [stamp.rotation[0].to_bits(), stamp.rotation[1].to_bits(), stamp.variation.to_bits(), flags],
            brightness: stamp.brightness,
        }
    }
}

/// Columns or rows intersected by an unwrapped span, each appearing once even if the stamp wraps repeatedly.
fn tile_ranges(start: i32, end: i32, size: i32) -> [std::ops::Range<u32>; 2] {
    let tile = |v| v as u32 / TILE;
    if end - start + 1 >= size {
        return [0..tile(size - 1) + 1, 0..0];
    }
    let first = start.rem_euclid(size);
    let last = end.rem_euclid(size);
    if first <= last {
        return [tile(first)..tile(last) + 1, 0..0];
    }
    if tile(first) <= tile(last) + 1 {
        // Both spans hit the same tile (or abut), covering all of them.
        return [0..tile(size - 1) + 1, 0..0];
    }
    [0..tile(last) + 1, tile(first)..tile(size - 1) + 1]
}

struct RasterBatch {
    stamps: Vec<Stamp>,
    bins: Vec<u32>,
}

impl RasterBatch {
    fn new(request: &RasterRequest<'_>, columns: u32, rows: Range<u32>, stamp_limit: usize, bin_limit: u64) -> Option<Self> {
        let header = columns as usize * rows.len() + 1;
        let mut total = header as u64;
        if total * 4 > bin_limit {
            return None;
        }
        let mut lists = vec![Vec::new(); header - 1];
        let mut stamps = Vec::new();
        let [w, h] = request.size.map(|s| s as i32);
        for (i, stamp) in request.stamps.iter().enumerate() {
            if i % 1024 == 0 && request.cancel.is_cancelled() {
                return None;
            }
            if request.framed && (stamp.repeats[2] - stamp.repeats[0] > 64 || stamp.repeats[3] - stamp.repeats[1] > 64) {
                // Keep a malformed frame with many repeated periods from monopolizing the graphics queue
                return None;
            }
            let [x0, y0, x1, y1] = stamp.bounds;
            let y_tiles = tile_ranges(y0, y1, h).map(|r| r.start.max(rows.start)..r.end.min(rows.end));
            if y_tiles.iter().all(Range::is_empty) {
                continue;
            }
            if stamps.len() == stamp_limit {
                return None;
            }
            let index = stamps.len() as u32;
            stamps.push(Stamp::from(stamp));
            let x_tiles = tile_ranges(x0, x1, w);
            for y in y_tiles.into_iter().flatten() {
                for x in x_tiles.iter().cloned().flatten() {
                    total += 1;
                    if total * 4 > bin_limit {
                        return None;
                    }
                    lists.get_mut(((y - rows.start) * columns + x) as usize)?.push(index);
                }
            }
        }
        let mut bins = vec![0; header];
        bins.reserve(total as usize - header);
        for (i, list) in lists.into_iter().enumerate() {
            *bins.get_mut(i)? = bins.len() as u32;
            bins.extend(list);
        }
        *bins.get_mut(header - 1)? = bins.len() as u32;
        if stamps.is_empty() {
            stamps.push(Stamp::default());
        }
        Some(Self { stamps, bins })
    }
}

/// One storage buffer with a table of offset, width, height, components, followed by the pixels.
fn pack_images(request: &RasterRequest<'_>, limit: u64) -> Option<Vec<u32>> {
    let mut data = vec![0; (request.images.len() + 1) * 4];
    for (index, image) in std::iter::once(request.background).chain(request.images.iter().copied().map(Some)).enumerate() {
        let Some(image) = image else { continue };
        let components = if request.color || (index == 0 && request.grey_alpha.is_none()) { 4 } else { 1 };
        if (data.len() as u64 + image.px.len() as u64 * components) * 4 > limit {
            return None;
        }
        let desc = [data.len() as u32, image.w as u32, image.h as u32, components as u32];
        data.get_mut(index * 4..index * 4 + 4)?.copy_from_slice(&desc);
        if components == 4 {
            data.extend_from_slice(bytemuck::cast_slice(&image.px));
        } else {
            data.extend(image.px.iter().map(|p| p[0].to_bits()));
        }
    }
    Some(data)
}

#[cfg(test)]
mod tests {
    use craft_material_graph::accelerator::Accelerator;
    use craft_material_graph::context::EvalContext;
    use craft_material_graph::graph::Graph;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct ObservedRasterizer(PatternRasterizer, AtomicUsize);

    impl Accelerator for ObservedRasterizer {
        fn cache_tag(&self) -> &str {
            "test pattern rasterizer"
        }

        fn pattern(&self, request: RasterRequest<'_>) -> Result<Option<RasterOutput>, String> {
            let result = self.0.render(request)?;
            if result.is_some() {
                self.1.fetch_add(1, Ordering::Relaxed);
            }
            Ok(result)
        }
    }

    #[test]
    fn pattern_raster_keeps_order_for_wrapped_and_framed_images() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let observed = Arc::new(ObservedRasterizer(PatternRasterizer::new(&gpu.device, &gpu.queue).unwrap(), AtomicUsize::default()));
        let eval = EvalContext { accelerator: Some(observed.clone()), ..Default::default() };
        let common = r#"size 256
meta compiled "1"
subgraph draws
meta kind "patterns"
node 1 pp_quadrant draws=1 pattern=4 branch_offset_x=0.41 size_x=1.22 size_y=0.64 rotation=0.19 luminance=0.72
node 2 pp_quadrant draws=1 image=1 image_filter=0 offset_x=-0.22 size_x=0.39 size_y=0.51 rotation=0.31 luminance=-0.42 max=1
node 3 pp_iterate first=@1 body=@2 count=3
node 4 pp_out in=@3
end
node 1 color r=0.81 g=0.25 b=0.41 a=0.2 osize_log2_x=4 osize_log2_y=4
node 2 color r=0.15 g=0.15 b=0.15 a=1 osize_log2_x=3 osize_log2_y=3
"#;
        for frame in ["", "color=1 frame_x0=0.1 frame_x1=1.1 frame_y0=-0.3 frame_y1=0.6 repeat_x0=0.2 repeat_x1=0.9"] {
            let graph =
                Graph::parse(&format!("{common}node 3 pattern_program program=\"draws\" in=@1 bg=@2 osize_mode=1 {frame}\noutput o 3\n")).unwrap();
            let expected = graph.evaluate(Some(256)).unwrap();
            let actual = graph.evaluate_in(&eval, Some(256), &[]).unwrap();
            let (expected_image, actual_image) = (&expected["o"], &actual["o"]);
            assert_eq!(
                (actual_image.w, actual_image.h, actual_image.px.len()),
                (expected_image.w, expected_image.h, expected_image.px.len()),
                "pattern raster size differs for {frame:?}"
            );
            assert!(
                expected["o"].px.iter().zip(&actual["o"].px).flat_map(|(a, b)| a.iter().zip(b)).all(|(a, b)| (a - b).abs() < 2e-5),
                "pattern raster differs for {frame:?}"
            );
        }
        assert_eq!(observed.1.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn pattern_raster_splits_large_draw_lists_in_row_order() {
        let Some(gpu) = crate::headless::Headless::new().unwrap() else { return };
        let mut rasterizer = PatternRasterizer::new(&gpu.device, &gpu.queue).unwrap();
        let cancel = craft_material_graph::context::Cancel::new();
        let stamps: Vec<_> = [0, 10, 27]
            .into_iter()
            .flat_map(|row| {
                [0.9, -0.68].map(|value| RasterStamp {
                    bounds: [3, row, 6, row + 3],
                    repeats: [0; 4],
                    center: [5.0 / 20.0, (row as f32 + 2.0) / 33.0],
                    half_size: [2.0 / 20.0, 2.0 / 33.0],
                    rotation: [0.0, 1.0],
                    variation: 0.0,
                    pattern: 1,
                    brightness: [value; 4],
                    image: None,
                    nearest: false,
                    keep_larger: false,
                })
            })
            .collect();
        let render = |rasterizer: &PatternRasterizer| {
            let Some(RasterOutput::Grey(image, _)) = rasterizer
                .render(RasterRequest {
                    size: [20, 33],
                    background: None,
                    clear: [0.2, 0.2, 0.2, 1.0],
                    color: false,
                    grey_alpha: Some(1.0),
                    framed: false,
                    region: [0.0, 1.0, 0.0, 1.0],
                    period: [1.0, 1.0],
                    stamps: &stamps,
                    images: &[],
                    cancel: &cancel,
                })
                .unwrap()
            else {
                panic!("the same pattern must fit the GPU when divided into bands");
            };
            image
        };

        let wide = render(&rasterizer);
        rasterizer.stamp_bytes = 4 * size_of::<Stamp>() as u64;
        let banded = render(&rasterizer);

        assert!(wide.px[4] > wide.px[0]);
        assert!(wide.px[27 * wide.w + 4] > wide.px[0]);
        assert_eq!(wide.px, banded.px);
    }
}
