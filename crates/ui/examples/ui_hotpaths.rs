//! Test-only headless performance harness for craft-ui hot paths.
//!
//! Run through the server task workflow, e.g. `cargo run -p craft-ui --example ui_hotpaths --locked`.
//! Set `CRAFT_UI_PERF_OUT` to write the raw JSON report (otherwise stdout); `CRAFT_UI_PERF_LABEL`
//! is copied into the report. Frames are `Context::run_ui` plus `Context::tessellate`; texture
//! deltas are counted but never uploaded, and no GPU, window, compositor or native input path is
//! exercised. Fonts are egui's bundled `default_fonts` unless `CRAFT_UI_PERF_FONT` points at a
//! pinned fixture file (recorded by the parent with its hash). Bodies, geometry, interactions and
//! the mesh fingerprint are collected on the final measured frame; the hover probe is a separate
//! unmeasured frame and never contributes to timings or counters.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

use craft_ui::{
    docking::{DockArea, DockError, DockStyle, Floating, Layout, Node, Permissions},
    layout::{SplitAxis, SplitSize, fit_toolbar, split_rect},
    tabs,
};
use egui::{Context, Id, Rect, pos2, vec2};
use serde_json::{Value, json};

const WARMUPS: usize = 50;
const MIN_WARMUP: Duration = Duration::from_secs(3);
const MICRO_SAMPLES: usize = 50;
const MICRO_BATCH: usize = 16;
const FRAME_SAMPLES: usize = 1000;
const EPS: f32 = 0.1;

#[derive(Default)]
struct Frames {
    times_ns: Vec<u64>,
    warmups: usize,
    warmup_elapsed_ns: u64,
    next_frame: usize,
    mesh_hash: u64,
    bodies: BTreeMap<u32, Rect>,
    shapes: usize,
    vertices: usize,
    indices: usize,
    set_images: usize,
    set_bytes: usize,
    free_images: usize,
    repaint_frames: usize,
    passes: Vec<usize>,
    galleys_after_warmup: usize,
}

/// Run all micro and dock/integration cases at 1× and 2×, gate each case, and emit the schema-3
/// JSON report to `CRAFT_UI_PERF_OUT` (or stdout). Fails when no bundled or fixture font is
/// available.
fn main() -> Result<(), Box<dyn Error>> {
    let (fonts, font_names) = font_definitions()?;
    if font_names.is_empty() {
        return Err("the example needs egui's bundled default_fonts or CRAFT_UI_PERF_FONT".into());
    }
    let mut micro = Vec::new();
    for scale in [1.0f32, 2.0] {
        for count in [1usize, 8, 64, 4096] {
            let natural: Vec<_> = tab_widths(count).into_iter().map(|width| width * scale).collect();
            let selected = count / 2;
            for available in [96.0f32, 640.0, 1920.0] {
                let (available, minimum, overflow) = (available * scale, 48.0 * scale, 64.0 * scale);
                check_tabs(&natural, selected, available, minimum, overflow)?;
                micro.push(micro_entry(
                    "tabs_fit",
                    json!({"tabs": count, "available": available, "selected": selected, "minimum_width": minimum,
                           "overflow_width": overflow, "size_scale": scale}),
                    || {
                        let natural = black_box(&natural);
                        black_box(tabs::fit(natural, black_box(selected), black_box(available), black_box(minimum), black_box(overflow)));
                    },
                ));
            }
        }
        for (shape, rect, axis) in [
            ("normal", Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0 * scale, 600.0 * scale)), SplitAxis::Horizontal),
            ("tiny", Rect::from_min_size(pos2(0.0, 0.0), vec2(120.0 * scale, 80.0 * scale)), SplitAxis::Vertical),
        ] {
            let (gap, minimum) = (4.0 * scale, 24.0 * scale);
            let span = match axis {
                SplitAxis::Horizontal => rect.width(),
                SplitAxis::Vertical => rect.height(),
            };
            // Dock layout uses `SplitSize::FixedFirst` (`docking/surface.rs`), so both fixed modes are
            // measured next to the ratio case for every shape and scale.
            for size in [SplitSize::Ratio(0.31), SplitSize::FixedFirst(span * 0.3), SplitSize::FixedSecond(span * 0.3)] {
                let layout = check_split(rect, axis, size, gap, minimum)?;
                micro.push(micro_entry(
                    "split_rect",
                    json!({"shape": shape, "axis": format!("{axis:?}"), "size": format!("{size:?}"), "gap": gap, "minimum": minimum,
                           "size_scale": scale}),
                    || {
                        black_box(split_rect(black_box(rect), black_box(axis), black_box(size), black_box(gap), black_box(minimum)));
                    },
                ));
                micro.push(micro_entry(
                    "split_resized",
                    json!({"shape": shape, "axis": format!("{axis:?}"), "size": format!("{size:?}"), "delta": 12.0 * scale,
                           "minimum": minimum, "size_scale": scale}),
                    || {
                        black_box(black_box(layout).resized(black_box(size), black_box(12.0 * scale), black_box(minimum)));
                    },
                ));
            }
        }
        for groups in [8usize, 4096] {
            let natural: Vec<_> = toolbar_widths(groups).into_iter().map(|width| width * scale).collect();
            let available = 1920.0 * scale;
            check_toolbar(&natural, available, 6.0 * scale, 40.0 * scale)?;
            micro.push(micro_entry(
                "fit_toolbar",
                json!({"groups": groups, "available": available, "spacing": 6.0 * scale, "overflow_width": 40.0 * scale,
                       "size_scale": scale}),
                || {
                    let natural = black_box(&natural);
                    black_box(fit_toolbar(natural, black_box(available), black_box(6.0 * scale), black_box(40.0 * scale)));
                },
            ));
        }
    }
    let mut dock = Vec::new();
    for (tag, panels, floating) in [("dock4", 4u32, true), ("dock32", 32u32, false)] {
        for viewport in [[800.0f32, 600.0], [1440.0, 900.0]] {
            for scale in [1.0f32, 2.0] {
                dock.push(run_case(Case { label: tag, kind: "dock", panels, floating, side_panel: false, viewport, scale }, &fonts)?);
            }
        }
    }
    let mut integration = Vec::new();
    for viewport in [[800.0f32, 600.0], [1440.0, 900.0]] {
        for scale in [1.0f32, 2.0] {
            integration.push(run_case(
                Case { label: "panel_integration", kind: "panel_integration", panels: 6, floating: false, side_panel: true, viewport, scale },
                &fonts,
            )?);
        }
    }
    let case_counts = json!({"micro": micro.len(), "dock": dock.len(), "panel_integration": integration.len()});
    let report = json!({
        "schema": 3, "harness": "craft-ui/examples/ui_hotpaths",
        "label": env::var("CRAFT_UI_PERF_LABEL").unwrap_or_default(),
        "os": env::consts::OS, "arch": env::consts::ARCH,
        "package_version": env!("CARGO_PKG_VERSION"), "debug_assertions": cfg!(debug_assertions),
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "theme": "dark",
        "render_config": {"visuals": "dark", "animation_time": 0.0, "text_cursor_blink": false, "max_texture_side": 4096,
            "window_focused": true, "clock": "frame index / 60 Hz", "pixels_per_point": [1.0, 2.0]},
        "notes": [
            "CPU-only headless measurement; no GPU renderer, native window, compositor or input.",
            "Dock frame scope is Context::run_ui + Context::tessellate; texture deltas are counted, not uploaded.",
            "Fonts are egui default_fonts or the CRAFT_UI_PERF_FONT fixture; no installed system fonts.",
            "Micro samples are mean-of-batch nanoseconds; frame samples are individual frame nanoseconds.",
            "Every micro operation black-boxes its inputs and result so loop-invariant hoisting cannot erase the measured work.",
            "Individual micro latencies are diagnostic only: below about one microsecond the single-call timer overhead dominates.",
            "Bodies, geometry, interactions and the mesh fingerprint come from the final measured frame; the hover probe is a separate unmeasured frame.",
        ],
        "minimum_warmups": WARMUPS, "minimum_warmup_ns": MIN_WARMUP.as_nanos(), "micro_samples": MICRO_SAMPLES, "micro_batch": MICRO_BATCH, "frame_samples": FRAME_SAMPLES,
        "fonts": {"source": font_source(), "font_file": env::var_os("CRAFT_UI_PERF_FONT").map(|path| path.to_string_lossy().into_owned()), "font_names": font_names,
            "families": fonts.families.iter().map(|(family, names)| (format!("{family:?}"), names)).collect::<BTreeMap<_, _>>()},
        "micro": micro, "dock": dock, "panel_integration": integration,
        "case_counts": case_counts,
    });
    let text = serde_json::to_string_pretty(&report)?;
    if let Ok(path) = env::var("CRAFT_UI_PERF_OUT") {
        std::fs::write(&path, &text)?;
        eprintln!("ui_hotpaths: wrote {path} ({} bytes)", text.len());
    } else {
        println!("{text}");
    }
    Ok(())
}

/// Synthetic tab widths from 40 to 119 pt, cycling with `(i * 17) % 80`.
fn tab_widths(count: usize) -> Vec<f32> {
    (0..count).map(|i| 40.0 + ((i * 17) % 80) as f32).collect()
}

/// Synthetic toolbar widths from 24 to 71 pt, cycling with `(i * 13) % 48`.
fn toolbar_widths(count: usize) -> Vec<f32> {
    (0..count).map(|i| 24.0 + ((i * 13) % 48) as f32).collect()
}

/// Time `op` after at least 50 warmup calls and 3 seconds, record 50 batch-mean samples (2000
/// operations per batch for split cases, 16 otherwise), and also record diagnostic single-call
/// latencies. Returns the counts, samples and summaries without p95/p99 where noted.
fn micro_entry(name: &str, params: Value, mut op: impl FnMut()) -> Value {
    let warmup_start = Instant::now();
    let mut warmups = 0;
    while warmups < WARMUPS || warmup_start.elapsed() < MIN_WARMUP {
        op();
        warmups += 1;
    }
    let warmup_elapsed_ns = warmup_start.elapsed().as_nanos() as u64;
    let mut samples = Vec::with_capacity(MICRO_SAMPLES);
    let batch = if name.starts_with("split_") { 2000 } else { MICRO_BATCH };
    for _ in 0..MICRO_SAMPLES {
        let start = Instant::now();
        for _ in 0..batch {
            op();
        }
        samples.push((start.elapsed().as_nanos() as u64) / batch as u64);
    }
    let mut summary = summarize(&samples);
    if let Some(stats) = summary.as_object_mut() {
        stats.remove("p95_ns");
        stats.remove("p99_ns");
    }
    let mut individual = Vec::with_capacity(MICRO_SAMPLES);
    for _ in 0..MICRO_SAMPLES {
        let start = Instant::now();
        op();
        individual.push(start.elapsed().as_nanos() as u64);
    }
    let mut latency = summarize(&individual);
    if let Some(stats) = latency.as_object_mut() {
        stats.remove("p99_ns");
    }
    let (sample_count, individual_count) = (samples.len(), individual.len());
    json!({"name": name, "params": params, "sample_kind": "mean_of_batch_ns", "iterations_per_sample": batch,
           "operations": warmups + MICRO_SAMPLES * batch + MICRO_SAMPLES,
           "warmups": warmups, "warmup_elapsed_ns": warmup_elapsed_ns, "samples": sample_count, "samples_ns": samples,
           "stats": summary, "individual_samples": individual_count, "individual_latency_ns": individual,
           "individual_latency_stats": latency,
           "individual_latency_note": "diagnostic only; timer overhead dominates single sub-microsecond calls"})
}

/// Sort a copy of `samples` and return count, median, p95, p99, min and max in nanoseconds.
fn summarize(samples: &[u64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    json!({"count": sorted.len(), "median_ns": percentile(&sorted, 50), "p95_ns": percentile(&sorted, 95),
           "p99_ns": percentile(&sorted, 99), "min_ns": sorted.first().copied().unwrap_or(0),
           "max_ns": sorted.last().copied().unwrap_or(0)})
}

/// Nearest-rank percentile of an ascending slice. Empty input returns 0; the index is clamped to
/// the last element.
fn percentile(sorted: &[u64], percent: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[(sorted.len() * percent).div_ceil(100).saturating_sub(1).min(sorted.len() - 1)]
}

/// Use egui's bundled fonts, or an empty definition set with the `CRAFT_UI_PERF_FONT` file inserted
/// first in both proportional and monospace families. Returns the used font names.
fn font_definitions() -> Result<(egui::FontDefinitions, Vec<String>), Box<dyn Error>> {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(path) = env::var_os("CRAFT_UI_PERF_FONT") {
        fonts = egui::FontDefinitions::empty();
        let name = "perf-fixture".to_owned();
        fonts.font_data.insert(name.clone(), Arc::new(egui::FontData::from_owned(std::fs::read(&path)?)));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().insert(0, name.clone());
        }
    }
    let names = fonts.font_data.keys().cloned().collect();
    Ok((fonts, names))
}

/// Describe the report's font source: the pinned `CRAFT_UI_PERF_FONT` file when
/// set, otherwise egui's bundled default fonts.
fn font_source() -> &'static str {
    if env::var_os("CRAFT_UI_PERF_FONT").is_some() { "CRAFT_UI_PERF_FONT (pinned fixture file)" } else { "egui default_fonts (bundled)" }
}

/// A headless context at `scale` with the given fonts, dark visuals, zero animation time and no
/// cursor blinking.
fn context(scale: f32, fonts: &egui::FontDefinitions) -> Context {
    let ctx = Context::default();
    ctx.set_pixels_per_point(scale);
    ctx.set_fonts(fonts.clone());
    ctx.set_visuals(egui::Visuals::dark());
    ctx.all_styles_mut(|style| {
        style.animation_time = 0.0;
        style.visuals.text_cursor.blink = false;
    });
    ctx
}

/// A headless screen rectangle from `(0, 0)` to `viewport[0] × viewport[1]`.
fn screen_rect(viewport: [f32; 2]) -> Rect {
    Rect::from_min_size(pos2(0.0, 0.0), vec2(viewport[0], viewport[1]))
}

/// A focused 60 Hz input for `viewport` with a bounded texture side for headless frames.
fn raw_input(viewport: [f32; 2], frame: usize) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(screen_rect(viewport)),
        time: Some(frame as f64 / 60.0),
        focused: true,
        max_texture_side: Some(4096),
        ..Default::default()
    }
}

/// Check that tab fitting partitions every index exactly once, uses finite nonnegative widths,
/// stays within `available` (plus epsilon) including the overflow control, and keeps the selected
/// tab (clamped to the last tab).
fn check_tabs(natural: &[f32], selected: usize, available: f32, minimum: f32, overflow: f32) -> Result<(), Box<dyn Error>> {
    let fit = tabs::fit(natural, selected, available, minimum, overflow);
    let mut seen = vec![false; natural.len()];
    let mut sum = 0.0f32;
    for (index, width) in &fit.shown {
        if *index >= natural.len() || seen[*index] || !width.is_finite() || *width < 0.0 {
            return Err(format!("tabs_fit invalid shown entry ({index}, {width})").into());
        }
        seen[*index] = true;
        sum += *width;
    }
    for index in &fit.overflow {
        if *index >= natural.len() || seen[*index] {
            return Err(format!("tabs_fit invalid overflow index {index}").into());
        }
        seen[*index] = true;
    }
    if seen.iter().any(|shown| !*shown) || sum + fit.overflow_width > available + EPS {
        return Err(format!("tabs_fit did not partition {available}: sum={sum} overflow={}", fit.overflow_width).into());
    }
    let selected = selected.min(natural.len().saturating_sub(1));
    if !natural.is_empty() && !fit.shown.iter().any(|(index, _)| *index == selected) {
        return Err("tabs_fit dropped the selected tab".into());
    }
    Ok(())
}

/// Check a split layout: both panes are positive and sum to the span minus the gap, do not overlap
/// or escape, `resized` stays finite, and an inverted rect is rejected. Returns the layout.
fn check_split(rect: Rect, axis: SplitAxis, size: SplitSize, gap: f32, minimum: f32) -> Result<craft_ui::layout::SplitLayout, Box<dyn Error>> {
    let layout = split_rect(rect, axis, size, gap, minimum).ok_or("split_rect returned None")?;
    let (span, first, second) = match axis {
        SplitAxis::Horizontal => (rect.width(), layout.first.width(), layout.second.width()),
        SplitAxis::Vertical => (rect.height(), layout.first.height(), layout.second.height()),
    };
    let (divider_end, second_start) = match axis {
        SplitAxis::Horizontal => (layout.divider.max.x, layout.second.min.x),
        SplitAxis::Vertical => (layout.divider.max.y, layout.second.min.y),
    };
    if !(first > 0.0 && second > 0.0) || (first + second + gap.min(span) - span).abs() > EPS {
        return Err(format!("split_rect geometry: first={first} second={second} span={span}").into());
    }
    if divider_end > second_start + EPS || !rect.expand(EPS).contains_rect(layout.first) || !rect.expand(EPS).contains_rect(layout.second) {
        return Err("split_rect panes overlap or escape their rect".into());
    }
    let next = layout.resized(size, 12.0, minimum).ok_or("split_resized returned None")?;
    let finite = match next {
        SplitSize::Ratio(v) | SplitSize::FixedFirst(v) | SplitSize::FixedSecond(v) => v.is_finite(),
    };
    if !finite {
        return Err("split_resized returned a non-finite size".into());
    }
    if split_rect(Rect::from_min_max(pos2(10.0, 10.0), pos2(0.0, 0.0)), axis, size, gap, minimum).is_some() {
        return Err("split_rect accepted an inverted rect".into());
    }
    Ok(layout)
}

/// Check that toolbar fitting is an exact prefix partition: shown widths match the natural widths,
/// overflow is the remaining suffix, a nonempty overflow reserves a positive control, and a
/// zero-width group is rejected.
fn check_toolbar(natural: &[f32], available: f32, spacing: f32, overflow: f32) -> Result<(), Box<dyn Error>> {
    let fit = fit_toolbar(natural, available, spacing, overflow).ok_or("fit_toolbar returned None")?;
    let mut expected = 0;
    for (index, width) in &fit.shown {
        if *index >= natural.len() || *index != expected || (*width - natural[*index]).abs() > EPS {
            return Err(format!("fit_toolbar broken prefix at {index}").into());
        }
        expected += 1;
    }
    if fit.overflow.iter().any(|index| *index < expected) || expected + fit.overflow.len() != natural.len() {
        return Err("fit_toolbar did not partition the groups".into());
    }
    if !fit.overflow.is_empty() && fit.overflow_width <= 0.0 {
        return Err("fit_toolbar overflowed without reserving a control".into());
    }
    if fit_toolbar(&[0.0], available, spacing, overflow).is_some() {
        return Err("fit_toolbar accepted a zero-width group".into());
    }
    Ok(())
}

/// A one-panel tabs node with the panel active.
fn leaf(panel: u32) -> Node<u32> {
    Node::Tabs { panels: vec![panel], active: 0 }
}

/// A balanced recursive split fixture over `panels`, alternating the axis with `depth`.
fn tree(panels: &[u32], depth: usize) -> Node<u32> {
    if panels.len() == 1 {
        return leaf(panels[0]);
    }
    let mid = panels.len() / 2;
    Node::Split {
        axis: if depth.is_multiple_of(2) { SplitAxis::Horizontal } else { SplitAxis::Vertical },
        size: SplitSize::Ratio(0.5),
        first: Box::new(tree(&panels[..mid], depth + 1)),
        second: Box::new(tree(&panels[mid..], depth + 1)),
    }
}

/// The benchmark layout: with `with_floating`, a fixed 240 pt second column beside a vertical split
/// plus floating window id 1; otherwise a balanced tree over `panels`.
fn dock_layout(panels: u32, with_floating: bool) -> Layout<u32> {
    if with_floating {
        Layout {
            root: Some(Node::Split {
                axis: SplitAxis::Horizontal,
                size: SplitSize::FixedSecond(240.0),
                first: Box::new(Node::Split {
                    axis: SplitAxis::Vertical,
                    size: SplitSize::Ratio(0.55),
                    first: Box::new(leaf(0)),
                    second: Box::new(leaf(1)),
                }),
                second: Box::new(leaf(2)),
            }),
            floating: vec![Floating::single(vec![3], 0, [80.0, 70.0, 300.0, 220.0]).with_id(1)],
        }
    } else {
        let all: Vec<u32> = (0..panels).collect();
        Layout { root: Some(tree(&all, 0)), floating: Vec::new() }
    }
}

/// Return the benchmark content's logical label, `Panel {panel}`.
fn panel_label(panel: &u32) -> String {
    format!("Panel {panel}")
}

/// Count texture-set images, their color bytes, and freed images in `delta`.
fn texture_counts(delta: &egui::TexturesDelta) -> (usize, usize, usize) {
    let mut images = 0;
    let mut bytes = 0;
    for deltas in delta.set.values() {
        for image in deltas {
            images += 1;
            let egui::ImageData::Color(data) = &image.image;
            bytes += data.as_raw().len();
        }
    }
    (images, bytes, delta.free.len())
}

/// Sum mesh vertices and indices, rejecting renderer callbacks because benchmark fixtures must not
/// produce them.
fn count_mesh(primitives: &[egui::epaint::ClippedPrimitive]) -> Result<(usize, usize), Box<dyn Error>> {
    let (mut vertices, mut indices) = (0, 0);
    for clipped in primitives {
        match &clipped.primitive {
            egui::epaint::Primitive::Mesh(mesh) => {
                vertices += mesh.vertices.len();
                indices += mesh.indices.len();
            }
            egui::epaint::Primitive::Callback(_) => return Err("the benchmark fixtures must not produce renderer callbacks".into()),
        }
    }
    Ok((vertices, indices))
}

// Fingerprint of one frame's tessellated primitives, beyond equal vertex/index counts. Texture IDs and
// clip bounds are included. This is a 64-bit FNV-1a checksum, not an exact mesh identity. Timings stop
// before this evidence collection.
/// A 64-bit FNV-1a fingerprint over primitive count, clip rects, texture ids, vertex data and
/// indices. This is a diagnostic checksum, not exact mesh identity; callbacks are rejected.
fn mesh_hash(primitives: &[egui::epaint::ClippedPrimitive]) -> Result<u64, Box<dyn Error>> {
    let mut hash = 0xcbf29ce484222325u64;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    feed(&(primitives.len() as u64).to_le_bytes());
    for clipped in primitives {
        for value in [clipped.clip_rect.min.x, clipped.clip_rect.min.y, clipped.clip_rect.max.x, clipped.clip_rect.max.y] {
            feed(&value.to_bits().to_le_bytes());
        }
        match &clipped.primitive {
            egui::epaint::Primitive::Mesh(mesh) => {
                feed(&[0]);
                match mesh.texture_id {
                    egui::TextureId::Managed(id) => {
                        feed(&[0]);
                        feed(&id.to_le_bytes());
                    }
                    egui::TextureId::User(id) => {
                        feed(&[1]);
                        feed(&id.to_le_bytes());
                    }
                }
                feed(&(mesh.vertices.len() as u64).to_le_bytes());
                feed(&(mesh.indices.len() as u64).to_le_bytes());
                for vertex in &mesh.vertices {
                    for value in [vertex.pos.x, vertex.pos.y, vertex.uv.x, vertex.uv.y] {
                        feed(&value.to_bits().to_le_bytes());
                    }
                    feed(&vertex.color.to_array());
                }
                for index in &mesh.indices {
                    feed(&index.to_le_bytes());
                }
            }
            egui::epaint::Primitive::Callback(_) => return Err("the benchmark fixtures must not produce renderer callbacks".into()),
        }
    }
    Ok(hash)
}

/// Run `FRAME_SAMPLES` measured frames after at least 50 warmup frames and 3 seconds. Times and
/// completed passes are recorded for every measured frame; the final measured frame also collects
/// bodies and the shapes, mesh counts and hash reported in the result. Texture and repaint counters
/// aggregate over measured frames, and texture deltas are cleared each frame.
fn measure_frames(
    ctx: &Context,
    viewport: [f32; 2],
    paint: &mut impl FnMut(&mut egui::Ui, &mut BTreeMap<u32, Rect>, bool),
) -> Result<Frames, Box<dyn Error>> {
    let mut result = Frames::default();
    result.times_ns.reserve(FRAME_SAMPLES);
    result.passes.reserve(FRAME_SAMPLES);
    let warmup_start = Instant::now();
    let mut frame = 0;
    while result.times_ns.len() < FRAME_SAMPLES {
        let warming = frame < WARMUPS || warmup_start.elapsed() < MIN_WARMUP;
        if !warming && result.times_ns.is_empty() {
            result.galleys_after_warmup = ctx.fonts(|fonts| fonts.num_galleys_in_cache());
            result.warmup_elapsed_ns = warmup_start.elapsed().as_nanos() as u64;
        }
        // The last measured frame also collects panel bodies, so bodies, geometry and mesh evidence all
        // describe the same rendered frame as the reported timings.
        let collect = !warming && result.times_ns.len() + 1 == FRAME_SAMPLES;
        result.bodies.clear();
        let start = Instant::now();
        let mut output = ctx.run_ui(raw_input(viewport, frame), |ui| paint(ui, &mut result.bodies, collect));
        let ppp = output.pixels_per_point;
        let shapes = output.shapes.len();
        let primitives = ctx.tessellate(output.shapes, ppp);
        if !warming {
            result.times_ns.push(start.elapsed().as_nanos() as u64);
            result.passes.push(output.platform_output.num_completed_passes);
            result.shapes = shapes;
            (result.vertices, result.indices) = count_mesh(&primitives)?;
            result.mesh_hash = mesh_hash(&primitives)?;
            let (sets, bytes, frees) = texture_counts(&output.textures_delta);
            result.set_images += sets;
            result.set_bytes += bytes;
            result.free_images += frees;
            if output.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|output| output.repaint_delay != std::time::Duration::MAX) {
                result.repaint_frames += 1;
            }
        } else {
            result.warmups += 1;
        }
        output.textures_delta.clear();
        frame += 1;
    }
    result.next_frame = frame;
    Ok(result)
}

/// Check every recorded body is positive, finite and inside the viewport; non-floating bodies may
/// not overlap by more than 0.5 pt. Returns the body count.
fn check_bodies(screen: Rect, bodies: &BTreeMap<u32, Rect>, floating: &BTreeSet<u32>) -> Result<usize, Box<dyn Error>> {
    for (panel, rect) in bodies {
        if !rect.is_positive() || !rect.min.is_finite() || !rect.max.is_finite() {
            return Err(format!("panel {panel} has empty or invalid body geometry {rect:?}").into());
        }
        if !screen.expand(EPS * 4.0).contains_rect(*rect) {
            return Err(format!("panel {panel} body {rect:?} escapes the viewport {screen:?}").into());
        }
    }
    let static_rects: Vec<(u32, Rect)> = bodies.iter().filter(|(panel, _)| !floating.contains(*panel)).map(|(panel, rect)| (*panel, *rect)).collect();
    for (i, (a, first)) in static_rects.iter().enumerate() {
        for (b, second) in static_rects.iter().skip(i + 1) {
            let overlap = first.intersect(*second);
            if overlap.width() > 0.5 && overlap.height() > 0.5 {
                return Err(format!("static panels {a} and {b} overlap: {overlap:?}").into());
            }
        }
    }
    Ok(bodies.len())
}

/// Check every interaction owned by the dock has a response with a positive hit rect inside the
/// viewport. Returns the interaction count.
fn check_interactions(ctx: &Context, dock_id: Id, screen: Rect) -> Result<usize, Box<dyn Error>> {
    let owned = ctx.data(|data| data.get_temp::<Vec<Id>>(dock_id.with("owned-interactions"))).unwrap_or_default();
    if owned.is_empty() {
        return Err("dock rendered no owned interactions".into());
    }
    for id in &owned {
        let response = ctx.read_response(*id).ok_or_else(|| format!("missing response for dock interaction {id:?}"))?;
        if !response.interact_rect.is_positive() {
            return Err(format!("dock interaction {id:?} has empty hit rect {:?}", response.interact_rect).into());
        }
        if !screen.expand(EPS * 4.0).contains_rect(response.interact_rect) {
            return Err(format!("dock interaction {id:?} hit rect {:?} escapes the viewport", response.interact_rect).into());
        }
    }
    Ok(owned.len())
}

/// Move the pointer to the first body's tab center in a separate unmeasured frame and require the
/// tab to hover; returns the probe JSON. The probe is excluded from timings and counters.
fn probe_tab_hover(
    ctx: &Context,
    viewport: [f32; 2],
    dock_id: Id,
    bodies: &BTreeMap<u32, Rect>,
    frame: usize,
    paint: &mut impl FnMut(&mut egui::Ui, &mut BTreeMap<u32, Rect>, bool),
) -> Result<Value, Box<dyn Error>> {
    let panel = bodies.keys().next().copied().ok_or("no panel body for hover probe")?;
    let tab_id = dock_id.with(("tab", panel));
    let rect = ctx.read_response(tab_id).ok_or("no tab response for hover probe")?.rect;
    let mut input = raw_input(viewport, frame);
    input.events.push(egui::Event::PointerMoved(rect.center()));
    let mut probe_bodies = BTreeMap::new();
    ctx.run_ui(input, |ui| paint(ui, &mut probe_bodies, true)).textures_delta.clear();
    if !ctx.read_response(tab_id).is_some_and(|response| response.hovered()) {
        return Err(format!("pointer at {panel}'s tab center did not hover its tab").into());
    }
    Ok(json!({"panel": panel, "tab_center": [rect.center().x, rect.center().y], "hovered": true,
               "scope": "separate unmeasured frame after the samples; excluded from timings and counters"}))
}

struct Case<'a> {
    label: &'a str,
    kind: &'a str,
    panels: u32,
    floating: bool,
    side_panel: bool,
    viewport: [f32; 2],
    scale: f32,
}

/// Render the fixture dock through `DockArea`, optionally recording each panel body's rect on the
/// collection frame, and return the dock error.
fn render_dock(
    ui: &mut egui::Ui,
    dock_id: Id,
    layout: &Layout<u32>,
    bodies: &mut BTreeMap<u32, Rect>,
    style: &DockStyle,
    collect: bool,
) -> Option<DockError> {
    DockArea::new(dock_id)
        .show(
            ui,
            layout,
            style,
            panel_label,
            |_| Permissions::default(),
            |ui, panel| {
                if collect {
                    bodies.insert(*panel, ui.max_rect());
                }
            },
        )
        .error
}

struct Scene {
    dock_id: Id,
    side_panel: bool,
    style: Option<DockStyle>,
    error: Option<DockError>,
    dock_rect: Rect,
    left_rect: Option<Rect>,
    central_rect: Option<Rect>,
}

impl Scene {
    /// Render the dock inside an optional 180 pt left panel and central panel, capturing panel geometry
    /// and preserving the first dock error; otherwise render at the available rect.
    fn render(&mut self, layout: &Layout<u32>, ui: &mut egui::Ui, bodies: &mut BTreeMap<u32, Rect>, collect: bool) {
        if self.style.is_none() {
            self.style = Some(DockStyle::from_ui(ui));
        }
        let Self { dock_id, side_panel, style, error, dock_rect, left_rect, central_rect } = self;
        let Some(style) = style.as_ref() else { return };
        if *side_panel {
            let _ = egui::Panel::left("ui-perf-left").resizable(false).exact_size(180.0).show(ui, |ui| {
                *left_rect = Some(ui.max_rect());
                ui.label("Left panel");
            });
            let _ = egui::CentralPanel::default().show(ui, |ui| {
                *central_rect = Some(ui.max_rect());
                let next = render_dock(ui, *dock_id, layout, bodies, style, collect);
                if error.is_none() {
                    *error = next;
                }
            });
        } else {
            *dock_rect = ui.available_rect_before_wrap();
            let next = render_dock(ui, *dock_id, layout, bodies, style, collect);
            if error.is_none() {
                *error = next;
            }
        }
    }
}

/// Run one case end to end: validate the layout, measure frames, gate sample count, warmup, one
/// completed pass per frame and dock error, check body count, interactions and geometry, run the
/// hover probe, and return the case JSON.
fn run_case(case: Case<'_>, fonts: &egui::FontDefinitions) -> Result<Value, Box<dyn Error>> {
    let ctx = context(case.scale, fonts);
    let dock_id = Id::unique(("ui-perf", case.label));
    let layout = dock_layout(case.panels, case.floating);
    layout.validate()?;
    let floating: BTreeSet<u32> = layout.floating.iter().flat_map(|group| group.panels().copied()).collect();
    let mut scene =
        Scene { dock_id, side_panel: case.side_panel, style: None, error: None, dock_rect: Rect::NOTHING, left_rect: None, central_rect: None };
    let frames = {
        let mut paint = |ui: &mut egui::Ui, bodies: &mut BTreeMap<u32, Rect>, collect: bool| scene.render(&layout, ui, bodies, collect);
        measure_frames(&ctx, case.viewport, &mut paint)?
    };
    if frames.times_ns.len() != FRAME_SAMPLES {
        return Err(format!("{} recorded {} of {FRAME_SAMPLES} frame samples", case.label, frames.times_ns.len()).into());
    }
    if frames.warmups < WARMUPS || frames.warmup_elapsed_ns < MIN_WARMUP.as_nanos() as u64 {
        return Err(format!("{} failed its warmup gate: warmups={} elapsed_ns={}", case.label, frames.warmups, frames.warmup_elapsed_ns).into());
    }
    if frames.passes.len() != frames.times_ns.len() || frames.passes.iter().any(|passes| *passes != 1) {
        return Err(format!("{} has measured frames without exactly one completed pass: {:?}", case.label, frames.passes).into());
    }
    if let Some(error) = scene.error {
        return Err(format!("{} reported {error}", case.label).into());
    }
    let screen = screen_rect(case.viewport);
    let body_count = check_bodies(screen, &frames.bodies, &floating)?;
    let interaction_count = check_interactions(&ctx, dock_id, screen)?;
    if body_count != case.panels as usize {
        return Err(format!("{} rendered {body_count} of {} panel bodies", case.label, case.panels).into());
    }
    let geometry = if case.side_panel {
        let left = scene.left_rect.ok_or("left panel did not render")?;
        let central = scene.central_rect.ok_or("central panel did not render")?;
        if !left.is_positive() || !central.is_positive() || left.right() > central.left() + EPS {
            return Err(format!("left/central panel geometry is invalid: left={left:?} central={central:?}").into());
        }
        json!({"left_rect": [left.min.x, left.min.y, left.width(), left.height()],
               "central_rect": [central.min.x, central.min.y, central.width(), central.height()]})
    } else {
        let dock_rect = scene.dock_rect;
        if !dock_rect.is_positive() {
            return Err(format!("{} had an empty available rect", case.label).into());
        }
        json!({"dock_rect": [dock_rect.min.x, dock_rect.min.y, dock_rect.width(), dock_rect.height()]})
    };
    let hover_probe = {
        let mut probe = |ui: &mut egui::Ui, bodies: &mut BTreeMap<u32, Rect>, collect: bool| scene.render(&layout, ui, bodies, collect);
        probe_tab_hover(&ctx, case.viewport, dock_id, &frames.bodies, frames.next_frame, &mut probe)?
    };
    let frame_stats = summarize(&frames.times_ns);
    Ok(json!({
        "case": case.label, "kind": case.kind, "panels": case.panels, "floating": case.floating,
        "viewport_points": case.viewport, "viewport_physical": [case.viewport[0] * case.scale, case.viewport[1] * case.scale],
        "pixels_per_point": case.scale, "frames": frames.times_ns.len(), "warmups": frames.warmups,
        "warmup_elapsed_ns": frames.warmup_elapsed_ns,
        "last_measured_mesh_fnv64": format!("{:016x}", frames.mesh_hash),
        "last_measured_mesh_scope": "64-bit FNV-1a fingerprint of the final measured frame's primitives; not an exact identity",
        "frame_scope": "Context::run_ui + Context::tessellate",
        "frame_times_ns": frames.times_ns, "frame_stats": frame_stats,
        "passes_per_frame": frames.passes,
        "counter_scope": "shapes, vertices, indices and the mesh fingerprint describe the final measured frame, which also collects bodies and geometry; texture and repaint counters are summed over all measured frames",
        "checks_scope": "final measured frame",
        "galleys_after_warmup": frames.galleys_after_warmup,
        "shapes": frames.shapes, "vertices": frames.vertices, "indices": frames.indices,
        "texture_set_images": frames.set_images, "texture_set_bytes": frames.set_bytes,
        "texture_free_images": frames.free_images, "repaint_frames": frames.repaint_frames,
        "galleys": ctx.fonts(|fonts| fonts.num_galleys_in_cache()),
        "hover_probe": hover_probe,
        "checks": {"bodies": body_count, "interactions": interaction_count, "geometry": geometry},
    }))
}
